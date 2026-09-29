mod dev;
mod init;
mod manifest;
mod project;
mod record;
mod registry;
mod terminal;
mod test;
mod toolchain;
mod ts;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;

use crate::terminal::{Terminal, print_logs};

/// Space Terminal screener toolkit: scaffold, build and run wasm screeners.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Local API port of Space Terminal (default: SPACE_TERMINAL_PORT, the terminal config, 5055)
    #[arg(long, global = true)]
    port: Option<u16>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a screener project (Rust: Cargo.toml + src/lib.rs; TypeScript: package.json + src/index.ts)
    Init {
        /// Project folder; created if missing
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Screener id, e.g. author.oi-screener (default: local.<folder>)
        #[arg(long)]
        id: Option<String>,
        /// Path to the space-screener crate (default: the local SDK checkout st was built from, else git)
        #[arg(long)]
        sdk_path: Option<PathBuf>,
        /// Language of the screener: rust or ts (TypeScript, built with extism-js)
        #[arg(long, default_value = "rust")]
        lang: init::InitLang,
    },
    /// Build screener.wasm and check it against the terminal's rules
    Build {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Check manifest.yaml and an already built screener.wasm (and whether the catalog would take it)
    Validate {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Replay recordings headless and compare with the expected rows (recordings/*.json)
    Test {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// One recording instead of every file in recordings/
        file: Option<PathBuf>,
        /// Accept the current output as expected (rewrite the *.expected.json snapshots)
        #[arg(long)]
        update: bool,
        /// Use the screener.wasm already built instead of building first
        #[arg(long)]
        no_build: bool,
    },
    /// Restart the screener in the terminal and record its data for `st test`
    Record {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Screener id (default: from manifest.yaml in --dir)
        #[arg(long)]
        id: Option<String>,
        /// How long to record
        #[arg(long, default_value_t = 60)]
        seconds: u64,
        /// Output file (default: recordings/<UTC time>.json)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build, install into the running terminal, then rebuild and reinstall on every change
    Dev {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Do not ask the terminal to open a pane for the screener
        #[arg(long)]
        no_open: bool,
    },
    /// Save a publish token (made on the author page of the store) for `st publish`
    Login {
        /// The token; read from stdin when omitted
        #[arg(long)]
        token: Option<String>,
        /// Registry URL (default: ST_REGISTRY, then https://store.space-terminal.com)
        #[arg(long)]
        registry: Option<String>,
    },
    /// Build, check and upload the screener to the catalog; a moderator reviews each version
    Publish {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// A recording from `st record` for the trial run and the moderator (≤ 4 MiB)
        #[arg(long)]
        recording: Option<PathBuf>,
        /// Publish the screener.wasm already built instead of building first
        #[arg(long)]
        no_build: bool,
        /// Registry URL (default: ST_REGISTRY, the one saved by `st login`, the Space Market store)
        #[arg(long)]
        registry: Option<String>,
    },
    /// Print the screener log from the terminal
    Logs {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Screener id (default: from manifest.yaml in --dir)
        #[arg(long)]
        id: Option<String>,
        /// Print lines after this sequence number
        #[arg(long, default_value_t = 0)]
        since: u64,
        /// Keep printing new lines
        #[arg(short, long)]
        follow: bool,
    },
    /// Print the rows the screener currently shows in the terminal
    Rows {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        id: Option<String>,
        /// The terminal's response as is: {ok, version, status, status_text?, rows}
        #[arg(long)]
        json: bool,
        /// Show at most this many rows
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// List screeners installed in the terminal
    List,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init {
            path,
            id,
            sdk_path,
            lang,
        } => init::init(&path, id, sdk_path, lang),
        Command::Build { dir } => {
            let built = project::build(&dir)?;
            project::print_summary(&built);
            println!("wrote {}", dir.join(project::WASM_FILE).display());
            Ok(())
        }
        Command::Validate { dir } => {
            let built = project::validate(&dir)?;
            project::print_summary(&built);
            project::print_catalog_readiness(&built);
            println!("ok");
            Ok(())
        }
        Command::Test {
            dir,
            file,
            update,
            no_build,
        } => test::run(&dir, file, update, !no_build),
        Command::Record {
            dir,
            id,
            seconds,
            out,
        } => record::run(&dir, id, seconds, out, cli.port),
        Command::Dev { dir, no_open } => dev::run(&dir, cli.port, !no_open),
        Command::Login { token, registry } => registry::login(token, registry),
        Command::Publish {
            dir,
            recording,
            no_build,
            registry,
        } => registry::publish(&dir, recording, !no_build, registry),
        Command::Logs {
            dir,
            id,
            since,
            follow,
        } => {
            let id = project::screener_id(&dir, id)?;
            let terminal = Terminal::connect(cli.port)?;
            let mut seq = since;
            loop {
                let logs = terminal.logs(&id, seq)?;
                print_logs(&logs.lines);
                seq = logs.next.max(seq);
                if !follow {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        Command::Rows {
            dir,
            id,
            json,
            limit,
        } => {
            let id = project::screener_id(&dir, id)?;
            let terminal = Terminal::connect(cli.port)?;
            if json {
                let out = terminal.rows_json(&id)?;
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            let rows = terminal.rows(&id)?;
            println!(
                "status: {}{} · version {} · {} rows",
                rows.status,
                rows.status_text
                    .as_deref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default(),
                rows.version,
                rows.rows.len()
            );
            for row in rows.rows.iter().take(limit) {
                println!("{}", render_row(row));
            }
            Ok(())
        }
        Command::List => {
            let list = Terminal::connect(cli.port)?.list()?;
            if list.screeners.is_empty() {
                println!("no screeners installed");
            }
            for s in list.screeners {
                let name = s.name.and_then(|n| n.en.or(n.ru)).unwrap_or_default();
                println!(
                    "{} {} [{}{}] rows={} panes={}{} — {name}",
                    s.id,
                    s.version,
                    s.status,
                    s.status_text.map(|t| format!(": {t}")).unwrap_or_default(),
                    s.rows,
                    s.panes,
                    if s.dev { " dev" } else { "" },
                );
            }
            Ok(())
        }
    }
}

fn render_cell(v: &Value) -> String {
    match v {
        Value::Object(o) => o
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| o.get("v").map(render_cell).unwrap_or_default()),
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn render_row(row: &Value) -> String {
    let key = row.get("key").and_then(Value::as_str).unwrap_or("?");
    let cells = row
        .get("cells")
        .and_then(Value::as_object)
        .map(|cells| {
            cells
                .iter()
                .map(|(k, v)| format!("{k}={}", render_cell(v)))
                .collect::<Vec<_>>()
                .join("  ")
        })
        .unwrap_or_default();
    format!("{key}  {cells}")
}
