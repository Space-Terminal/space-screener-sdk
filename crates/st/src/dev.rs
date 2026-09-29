use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::project;
use crate::terminal::{Terminal, print_logs};

const DEBOUNCE: Duration = Duration::from_millis(300);
const LOG_POLL: Duration = Duration::from_secs(1);

struct Session<'a> {
    dir: &'a Path,
    terminal: &'a Terminal,
    open_pane: bool,
    id: Option<String>,
    log_seq: u64,
    terminal_down: bool,
}

impl Session<'_> {
    fn build_and_install(&mut self) {
        let built = match project::build(self.dir) {
            Ok(built) => built,
            Err(e) => {
                eprintln!("build failed: {e:#}");
                eprintln!("watching for changes…");
                return;
            }
        };
        match self
            .terminal
            .install(&built.manifest_text, &built.wasm, self.open_pane)
        {
            Ok(installed) => {
                println!(
                    "installed {} {} (running: {}, pane: {})",
                    installed.id, installed.version, installed.running, installed.pane
                );
                if self.id.as_deref() != Some(installed.id.as_str()) {
                    self.log_seq = 0;
                }
                self.id = Some(installed.id);
                self.open_pane = false;
                self.terminal_down = false;
            }
            Err(e) => eprintln!("install failed: {e:#}"),
        }
        eprintln!("watching for changes…");
    }

    fn poll_logs(&mut self) {
        let Some(id) = self.id.clone() else {
            return;
        };
        match self.terminal.logs(&id, self.log_seq) {
            Ok(logs) => {
                print_logs(&logs.lines);
                self.log_seq = logs.next.max(self.log_seq);
                self.terminal_down = false;
            }
            Err(e) if !self.terminal_down => {
                eprintln!("logs unavailable: {e:#}");
                self.terminal_down = true;
            }
            Err(_) => {}
        }
    }
}

// The project folder is watched non-recursively (editors that save by rename break
// file-level watches), so its events are filtered down to the sources that matter.
fn relevant(event: &Event, dir: &Path) -> bool {
    let sources = [dir.join("src"), dir.join("pdk")];
    let files = [
        dir.join("manifest.yaml"),
        dir.join("Cargo.toml"),
        dir.join("package.json"),
        dir.join("tsconfig.json"),
    ];
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && event
        .paths
        .iter()
        .any(|p| sources.iter().any(|s| p.starts_with(s)) || files.contains(p))
}

pub fn run(dir: &Path, cli_port: Option<u16>, open_pane: bool) -> Result<()> {
    let dir: PathBuf = dir
        .canonicalize()
        .with_context(|| format!("{} does not exist", dir.display()))?;
    let terminal = Terminal::connect(cli_port)?;
    println!("terminal: {}", terminal.describe());

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).context("cannot start the file watcher")?;
    watcher
        .watch(&dir.join("src"), RecursiveMode::Recursive)
        .context("cannot watch src/")?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .context("cannot watch the project folder")?;
    let pdk = dir.join("pdk");
    if pdk.is_dir() {
        watcher
            .watch(&pdk, RecursiveMode::Recursive)
            .context("cannot watch pdk/")?;
    }

    let mut session = Session {
        dir: &dir,
        terminal: &terminal,
        open_pane,
        id: None,
        log_seq: 0,
        terminal_down: false,
    };
    session.build_and_install();
    let mut next_poll = Instant::now();
    loop {
        match rx.recv_timeout(LOG_POLL) {
            Ok(Ok(event)) if relevant(&event, &dir) => {
                let deadline = Instant::now() + DEBOUNCE;
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    if rx.recv_timeout(left).is_err() {
                        break;
                    }
                }
                println!("change detected, rebuilding…");
                session.build_and_install();
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("watch error: {e}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if Instant::now() >= next_poll {
            session.poll_logs();
            next_poll = Instant::now() + LOG_POLL;
        }
    }
}
