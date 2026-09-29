use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use space_screener_check::recording::{Event, Recording};
use space_screener_check::{Lang, Manifest};

use crate::harness::{CallOutcome, Harness, HarnessError};
use crate::output::{LogLine, Output, Row, Stats};
use crate::source::{Offline, Replay};

/// Terminal version the harness reports to `init` when no recording supplies one.
pub const HARNESS_TERMINAL: &str = "0.104.71";
/// Start of the virtual clock for trial runs without a recording (2026-01-01T00:00:00Z).
pub const TRIAL_EPOCH_MS: i64 = 1_767_225_600_000;
pub const TRIAL_TIMERS: usize = 3;
pub const TRIAL_SAMPLE_ROWS: usize = 20;
pub const TRIAL_LOG_LINES: usize = 200;
/// Wall-clock budget of a whole trial run; the remaining timers are skipped after it.
pub const TRIAL_WALL_LIMIT: Duration = Duration::from_secs(20);

fn lang_of(recording: &Recording) -> Lang {
    if recording.lang == "ru" {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// Runs a recording headless and deterministically: the recorded `init`, timers and clicks in
/// order, data host functions answered from the recording. A trap recreates the instance and
/// repeats `init`, as the terminal does.
pub fn replay(
    manifest: &Manifest,
    wasm: &[u8],
    recording: &Recording,
) -> Result<Output, HarnessError> {
    let replay = Replay::new(recording);
    let mut harness = Harness::new(manifest, wasm, Box::new(replay))?;
    harness.seed_kv(recording.kv.clone())?;
    if recording.truncated {
        harness.warn("the recording hit its size cap and ends early")?;
    }
    let default_init = json!({
        "params": manifest.merge_params(&recording.params),
        "terminal": recording.terminal,
        "lang": lang_of(recording),
        "now_ms": recording.started_ms,
    });
    let mut init_input = default_init;
    let mut started = false;
    for event in &recording.events {
        let outcome = match event {
            Event::Init { t_ms, input } => {
                init_input = input.clone();
                started = true;
                harness.init_with(input, *t_ms)?
            }
            Event::Timer { t_ms, input } => {
                if !started {
                    started = true;
                    harness.init_with(&init_input, recording.started_ms)?;
                }
                harness.timer_with(input, *t_ms)?
            }
            Event::Click { t_ms, input } => harness.click(input, *t_ms)?,
            Event::Call { .. } => continue,
        };
        if outcome.needs_restart() {
            harness.restart()?;
            let now = event.t_ms();
            harness.init_with(&init_input, now)?;
        }
    }
    if let Some(last) = recording.events.last() {
        harness.set_now(last.t_ms())?;
    }
    harness.output()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Fail,
}

/// What a moderator sees about a plugin run headless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrialReport {
    pub verdict: Verdict,
    pub issues: Vec<String>,
    pub sample_rows: Vec<Row>,
    pub total_rows: usize,
    pub logs: Vec<LogLine>,
    pub stats: Stats,
    /// A recording drove the run (otherwise the offline data set).
    pub recorded: bool,
}

impl TrialReport {
    fn fail(issue: String) -> Self {
        Self {
            verdict: Verdict::Fail,
            issues: vec![issue],
            sample_rows: Vec::new(),
            total_rows: 0,
            logs: Vec::new(),
            stats: Stats::default(),
            recorded: false,
        }
    }
}

/// `init` and up to three `on_timer` calls with the terminal's limits. Data comes from the
/// recording when given, else from the offline set (no network). A trap or a limit in `init`
/// fails the trial; anything else is a warning for the moderator.
pub fn trial(manifest: &Manifest, wasm: &[u8], recording: Option<&Recording>) -> TrialReport {
    match run_trial(manifest, wasm, recording) {
        Ok(report) => report,
        Err(HarnessError::Invalid(report)) => TrialReport {
            issues: report.errors.iter().map(ToString::to_string).collect(),
            ..TrialReport::fail(String::new())
        },
        Err(error) => TrialReport::fail(error.to_string()),
    }
}

fn run_trial(
    manifest: &Manifest,
    wasm: &[u8],
    recording: Option<&Recording>,
) -> Result<TrialReport, HarnessError> {
    let started = Instant::now();
    let mut issues = Vec::new();
    let mut verdict = Verdict::Ok;
    let mut harness = match recording {
        Some(recording) => {
            let mut harness = Harness::new(manifest, wasm, Box::new(Replay::new(recording)))?;
            harness.seed_kv(recording.kv.clone())?;
            if recording.truncated {
                harness.warn("the recording hit its size cap and ends early")?;
            }
            harness
        }
        None => Harness::new(manifest, wasm, Box::new(Offline))?,
    };

    let (init_input, start_ms, timers): (Value, i64, Vec<(i64, Value)>) = match recording {
        Some(recording) => {
            let init = recording.events.iter().find_map(|event| match event {
                Event::Init { t_ms, input } => Some((*t_ms, input.clone())),
                _ => None,
            });
            let (start_ms, init_input) = init.unwrap_or_else(|| {
                (
                    recording.started_ms,
                    json!({
                        "params": manifest.merge_params(&recording.params),
                        "terminal": recording.terminal,
                        "lang": lang_of(recording),
                        "now_ms": recording.started_ms,
                    }),
                )
            });
            let timers = recording
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::Timer { t_ms, input } => Some((*t_ms, input.clone())),
                    _ => None,
                })
                .take(TRIAL_TIMERS)
                .collect();
            (init_input, start_ms, timers)
        }
        None => {
            let step = i64::try_from(manifest.timer.as_millis()).unwrap_or(i64::MAX);
            let timers = (1..=TRIAL_TIMERS)
                .map(|n| {
                    let t_ms = TRIAL_EPOCH_MS + step.saturating_mul(n as i64);
                    (t_ms, json!({ "now_ms": t_ms }))
                })
                .collect();
            let input = json!({
                "params": manifest.default_params(),
                "terminal": HARNESS_TERMINAL,
                "lang": Lang::En,
                "now_ms": TRIAL_EPOCH_MS,
            });
            (input, TRIAL_EPOCH_MS, timers)
        }
    };

    let init = harness.init_with(&init_input, start_ms)?;
    match &init {
        CallOutcome::Done { .. } => {}
        CallOutcome::Failed { .. } | CallOutcome::OverBudget { .. } => {
            verdict = Verdict::Warn;
            issues.push(format!("init {}", init.describe()));
        }
        CallOutcome::NoExport => {
            verdict = Verdict::Fail;
            issues.push("init is not exported".to_string());
        }
        CallOutcome::Trapped { .. } | CallOutcome::Cut { .. } | CallOutcome::TimedOut { .. } => {
            verdict = Verdict::Fail;
            issues.push(format!("init {}", init.describe()));
        }
    }

    if verdict != Verdict::Fail {
        for (t_ms, input) in timers {
            if started.elapsed() > TRIAL_WALL_LIMIT {
                issues.push(format!(
                    "stopped after {} s of wall time",
                    TRIAL_WALL_LIMIT.as_secs()
                ));
                verdict = Verdict::Warn;
                break;
            }
            let outcome = harness.timer_with(&input, t_ms)?;
            if !outcome.is_ok() {
                verdict = Verdict::Warn;
                issues.push(format!("on_timer {}", outcome.describe()));
            }
            if outcome.needs_restart() {
                break;
            }
        }
    }

    let output = harness.output()?;
    if verdict != Verdict::Fail && output.rows.is_empty() {
        verdict = Verdict::Warn;
        issues.push(if recording.is_some() {
            "no rows after init and the timers".to_string()
        } else {
            "no rows after init and the timers (offline: http and history fail)".to_string()
        });
    }
    if !output.warnings.is_empty() && verdict == Verdict::Ok {
        verdict = Verdict::Warn;
    }
    issues.extend(output.warnings.iter().cloned());
    if output.stats.misses > 0 {
        issues.push(format!(
            "{} data calls were not in the recording",
            output.stats.misses
        ));
    }
    let skip = output.logs.len().saturating_sub(TRIAL_LOG_LINES);
    Ok(TrialReport {
        verdict,
        issues,
        total_rows: output.rows.len(),
        sample_rows: output.rows.into_iter().take(TRIAL_SAMPLE_ROWS).collect(),
        logs: output.logs.into_iter().skip(skip).collect(),
        stats: output.stats,
        recorded: recording.is_some(),
    })
}
