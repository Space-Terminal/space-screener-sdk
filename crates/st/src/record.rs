use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use space_screener_check::recording::Event;

use crate::project;
use crate::registry::MAX_RECORDING_BYTES;
use crate::terminal::Terminal;
use crate::test::RECORDINGS_DIR;

pub const MIN_SECONDS: u64 = 5;
pub const MAX_SECONDS: u64 = 600;
const POLL: Duration = Duration::from_secs(1);
const PROGRESS_EVERY: Duration = Duration::from_secs(15);
/// Slack past the requested duration: the terminal ends a recording on its own clock.
const GRACE: Duration = Duration::from_secs(60);

/// A recording over the registry's cap still serves `st test`, but `st publish --recording`
/// refuses it: say so, with the seconds that would fit at the rate this one grew (10 % margin).
fn publish_size_warning(size: usize, recorded_ms: i64) -> Option<String> {
    if size <= MAX_RECORDING_BYTES {
        return None;
    }
    let seconds = recorded_ms.max(1) as f64 / 1000.0;
    let fit = (seconds * MAX_RECORDING_BYTES as f64 / size as f64 * 0.9).floor() as u64;
    Some(format!(
        "the recording is {} KiB, over the registry's {} KiB limit: for `st publish --recording` record fewer seconds (about {} s or less); `st test` can use it as is",
        size.div_ceil(1024),
        MAX_RECORDING_BYTES / 1024,
        fit.max(1)
    ))
}

/// `2026-09-29T16-40-12Z`: sortable, and valid in file names on every OS.
fn utc_stamp(unix_s: i64) -> String {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

fn default_out(dir: &Path) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    dir.join(RECORDINGS_DIR)
        .join(format!("{}.json", utc_stamp(now)))
}

/// Asks the terminal to restart the screener and record its run, waits, and saves the
/// recording for `st test`.
pub fn run(
    dir: &Path,
    id: Option<String>,
    seconds: u64,
    out: Option<PathBuf>,
    cli_port: Option<u16>,
) -> Result<()> {
    if !(MIN_SECONDS..=MAX_SECONDS).contains(&seconds) {
        bail!("--seconds must be {MIN_SECONDS}..={MAX_SECONDS}");
    }
    let id = project::screener_id(dir, id)?;
    let terminal = Terminal::connect(cli_port)?;
    terminal.record_start(&id, seconds).with_context(|| {
        format!(
            "cannot start recording `{id}`; the screener has to run in a pane of the terminal at {} (`st dev` opens one)",
            terminal.describe()
        )
    })?;
    println!("recording `{id}` for {seconds} s — the terminal restarted it from init…");

    let deadline = Instant::now() + Duration::from_secs(seconds) + GRACE;
    let mut next_progress = Instant::now() + PROGRESS_EVERY;
    let recording = loop {
        std::thread::sleep(POLL);
        let state = terminal.record_state(&id)?;
        match state.state.as_str() {
            "done" => {
                break state
                    .recording
                    .context("the terminal said `done` but sent no recording")?;
            }
            "recording" => {
                if Instant::now() >= next_progress {
                    if let Some(left) = state.remaining_ms {
                        println!("  {} s left", left.div_ceil(1000));
                    }
                    next_progress = Instant::now() + PROGRESS_EVERY;
                }
            }
            "none" => bail!(
                "the terminal has no recording for `{id}` (was the screener removed or the terminal restarted?)"
            ),
            other => bail!("unexpected recording state `{other}`"),
        }
        if Instant::now() >= deadline {
            bail!("the terminal did not finish the recording in time");
        }
    };

    let path = out.unwrap_or_else(|| default_out(dir));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(&recording).context("cannot encode the recording")?;
    std::fs::write(&path, &bytes).with_context(|| format!("cannot write {}", path.display()))?;

    let count = |pred: fn(&Event) -> bool| recording.events.iter().filter(|e| pred(e)).count();
    println!(
        "wrote {} ({} KiB): {} timers, {} clicks, {} data calls",
        path.display(),
        bytes.len().div_ceil(1024),
        count(|e| matches!(e, Event::Timer { .. })),
        count(|e| matches!(e, Event::Click { .. })),
        count(|e| matches!(e, Event::Call { .. })),
    );
    if recording.truncated {
        eprintln!(
            "warning: the recording hit the terminal's size cap and ends early; record fewer seconds"
        );
    }
    let recorded_ms = recording
        .events
        .last()
        .map_or(0, |last| last.t_ms() - recording.started_ms);
    let recorded_ms = if recorded_ms > 0 {
        recorded_ms
    } else {
        i64::try_from(seconds * 1000).unwrap_or(i64::MAX)
    };
    if let Some(warning) = publish_size_warning(bytes.len(), recorded_ms) {
        eprintln!("warning: {warning}");
    }
    println!("next: `st test` replays it headless and saves the expected rows");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_when_the_recording_is_too_big_to_publish() {
        assert_eq!(publish_size_warning(MAX_RECORDING_BYTES, 45_000), None);
        let warning = publish_size_warning(4194 * 1024, 45_000).unwrap();
        assert!(warning.contains("4194 KiB"), "{warning}");
        assert!(warning.contains("4096 KiB"), "{warning}");
        assert!(warning.contains("about 39 s or less"), "{warning}");
        assert!(
            publish_size_warning(100 * MAX_RECORDING_BYTES, 1_000)
                .unwrap()
                .contains("about 1 s")
        );
    }

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(utc_stamp(0), "1970-01-01T00-00-00Z");
        assert_eq!(utc_stamp(1_790_699_566), "2026-09-29T16-32-46Z");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29T00-00-00Z");
    }
}
