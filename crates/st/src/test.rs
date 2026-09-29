use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use space_screener_check::Recording;
use space_screener_harness::{Alert, Output, Row, Status, replay};

use crate::project::{self, Built};

pub const RECORDINGS_DIR: &str = "recordings";
const EXPECTED_SUFFIX: &str = ".expected.json";
const DIFF_LINES: usize = 20;

/// What a replay must reproduce: the rows the pane would show, the alerts and the status.
/// Logs and CPU numbers are left out — they change without the output changing.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    rows: Vec<Row>,
    alerts: Vec<Alert>,
    status: Option<Status>,
}

impl From<&Output> for Snapshot {
    fn from(output: &Output) -> Self {
        Self {
            rows: output.rows.clone(),
            alerts: output.alerts.clone(),
            status: output.status.clone(),
        }
    }
}

fn is_recording(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "json")
        && !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(EXPECTED_SUFFIX))
}

fn recordings(dir: &Path) -> Result<Vec<PathBuf>> {
    let folder = dir.join(RECORDINGS_DIR);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("cannot list {}", folder.display()))?
            .path();
        if is_recording(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn expected_path(recording: &Path) -> PathBuf {
    let stem = recording
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording");
    recording.with_file_name(format!("{stem}{EXPECTED_SUFFIX}"))
}

pub fn read_recording(path: &Path) -> Result<Recording> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a recording made by `st record`", path.display()))
}

/// Replays every recording (or the one given) against the built plugin and compares the
/// result with its snapshot. A missing snapshot is written; `update` rewrites them all.
pub fn run(dir: &Path, file: Option<PathBuf>, update: bool, build: bool) -> Result<()> {
    let built = if build {
        project::build(dir)?
    } else {
        project::validate(dir)?
    };
    let files = match file {
        Some(file) => vec![file],
        None => recordings(dir)?,
    };
    if files.is_empty() {
        bail!(
            "no recordings in {}; make one with `st record` while the screener runs in the terminal",
            dir.join(RECORDINGS_DIR).display()
        );
    }
    let mut failed = 0;
    for path in &files {
        match test_one(&built, path, update) {
            Ok(true) => {}
            Ok(false) => failed += 1,
            Err(e) => {
                eprintln!("{}: {e:#}", path.display());
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} of {} recordings failed", files.len());
    }
    println!("ok: {} recordings", files.len());
    Ok(())
}

fn test_one(built: &Built, path: &Path, update: bool) -> Result<bool> {
    let recording = read_recording(path)?;
    if recording.id != built.manifest.id {
        eprintln!(
            "warning: {} was recorded for `{}`, the project is `{}`",
            path.display(),
            recording.id,
            built.manifest.id
        );
    }
    let output = replay(&built.manifest, &built.wasm, &recording)
        .with_context(|| format!("cannot replay {}", path.display()))?;
    let stats = &output.stats;
    println!(
        "{}: {} rows, {} alerts, {} host calls, {} not recorded, {} traps, max {} ms CPU per call",
        path.display(),
        output.rows.len(),
        output.alerts.len(),
        stats.host_calls,
        stats.misses,
        stats.traps,
        stats.cpu_ms_max
    );
    for warning in &output.warnings {
        eprintln!("  warning: {warning}");
    }
    let mut passed = true;
    if stats.traps > 0 {
        eprintln!("  the plugin trapped {} times", stats.traps);
        passed = false;
    }

    let snapshot = Snapshot::from(&output);
    let expected_path = expected_path(path);
    let expected = match std::fs::read(&expected_path) {
        Ok(bytes) if !update => Some(
            serde_json::from_slice::<Snapshot>(&bytes)
                .with_context(|| format!("cannot parse {}", expected_path.display()))?,
        ),
        _ => None,
    };
    match expected {
        Some(expected) if expected == snapshot => {}
        Some(expected) => {
            eprintln!(
                "  differs from {} (`st test --update` accepts the new output):",
                expected_path.display()
            );
            for line in diff(&expected, &snapshot).into_iter().take(DIFF_LINES) {
                eprintln!("    {line}");
            }
            passed = false;
        }
        None => {
            let mut text =
                serde_json::to_string_pretty(&snapshot).context("cannot encode the snapshot")?;
            text.push('\n');
            std::fs::write(&expected_path, text)
                .with_context(|| format!("cannot write {}", expected_path.display()))?;
            println!("  wrote {}", expected_path.display());
        }
    }
    Ok(passed)
}

fn row_changes(before: &Row, after: &Row) -> String {
    let mut changes = Vec::new();
    for (key, was) in &before.cells {
        match after.cells.get(key) {
            None => changes.push(format!("{key} {was} → (none)")),
            Some(now) if now != was => changes.push(format!("{key} {was} → {now}")),
            Some(_) => {}
        }
    }
    for (key, now) in after
        .cells
        .iter()
        .filter(|(k, _)| !before.cells.contains_key(*k))
    {
        changes.push(format!("{key} (none) → {now}"));
    }
    if (
        &before.symbol,
        &before.exchange,
        &before.market,
        before.rank,
    ) != (&after.symbol, &after.exchange, &after.market, after.rank)
    {
        changes.push("symbol/exchange/market/rank".into());
    }
    changes.join(", ")
}

fn diff(expected: &Snapshot, actual: &Snapshot) -> Vec<String> {
    let by_key = |rows: &[Row]| -> BTreeMap<String, Row> {
        rows.iter()
            .map(|row| (row.key.clone(), row.clone()))
            .collect()
    };
    let (before, after) = (by_key(&expected.rows), by_key(&actual.rows));
    let mut lines = Vec::new();
    for (key, row) in &before {
        match after.get(key) {
            None => lines.push(format!("- row {key}")),
            Some(now) if now != row => {
                lines.push(format!("~ row {key}: {}", row_changes(row, now)));
            }
            Some(_) => {}
        }
    }
    for key in after.keys().filter(|key| !before.contains_key(*key)) {
        lines.push(format!("+ row {key}"));
    }
    if expected.alerts != actual.alerts {
        lines.push(format!(
            "~ alerts: {} expected, {} now",
            expected.alerts.len(),
            actual.alerts.len()
        ));
    }
    if expected.status != actual.status {
        let show = |s: &Option<Status>| {
            s.as_ref()
                .map(|s| format!("{} ({})", s.text, s.tone))
                .unwrap_or_else(|| "none".into())
        };
        lines.push(format!(
            "~ status: {} → {}",
            show(&expected.status),
            show(&actual.status)
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, v: i64) -> Row {
        serde_json::from_value(serde_json::json!({"key": key, "cells": {"v": v}})).unwrap()
    }

    #[test]
    fn snapshot_file_sits_next_to_the_recording() {
        assert_eq!(
            expected_path(Path::new("recordings/2026-09-29T10-00-00Z.json")),
            Path::new("recordings/2026-09-29T10-00-00Z.expected.json")
        );
        assert!(is_recording(Path::new("recordings/a.json")));
        assert!(!is_recording(Path::new("recordings/a.expected.json")));
        assert!(!is_recording(Path::new("recordings/notes.txt")));
    }

    #[test]
    fn diff_names_rows_by_key() {
        let expected = Snapshot {
            rows: vec![row("a", 1), row("b", 2)],
            alerts: Vec::new(),
            status: None,
        };
        let actual = Snapshot {
            rows: vec![row("b", 3), row("c", 4)],
            alerts: Vec::new(),
            status: None,
        };
        let lines = diff(&expected, &actual);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("- row a"));
        assert_eq!(lines[1], "~ row b: v 2 → 3");
        assert!(lines[2].starts_with("+ row c"));
    }
}
