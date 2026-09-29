use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use space_screener_check::Recording;
use space_screener_harness::{Alert, Intent, Output, Row, Status, replay};

use crate::project::{self, Built};

pub const RECORDINGS_DIR: &str = "recordings";
const EXPECTED_SUFFIX: &str = ".expected.json";
const DIFF_LINES: usize = 20;
/// Error and warning lines of the plugin log shown when calls failed.
const ERROR_LINES: usize = 10;

/// What a replay must reproduce: the rows the pane would show, the alerts, the status and the
/// markets clicks opened. Logs and CPU numbers are left out — they change without the output
/// changing.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    rows: Vec<Row>,
    alerts: Vec<Alert>,
    status: Option<Status>,
    #[serde(default)]
    intents: Vec<Intent>,
}

impl From<&Output> for Snapshot {
    fn from(output: &Output) -> Self {
        Self {
            rows: output.rows.clone(),
            alerts: output.alerts.clone(),
            status: output.status.clone(),
            intents: output.intents.clone(),
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
    let broken = print_call_errors(&output);
    let mut passed = !broken;

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
        None if broken => eprintln!(
            "  {} not written: fix the failing calls first",
            expected_path.display()
        ),
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

/// Calls that returned an error, trapped or went over their CPU budget fail the test: a
/// snapshot of such a run would record the failure as the expected output.
fn print_call_errors(output: &Output) -> bool {
    let stats = &output.stats;
    if stats.failures + stats.traps + stats.over_budget == 0 {
        return false;
    }
    eprintln!(
        "  calls failed: {} returned an error, {} trapped, {} over the CPU budget",
        stats.failures, stats.traps, stats.over_budget
    );
    let lines: Vec<_> = output
        .logs
        .iter()
        .filter(|line| matches!(line.level.as_str(), "error" | "warn"))
        .collect();
    let mut shown: Vec<(&str, &str, usize)> = Vec::new();
    for line in &lines {
        match shown.last_mut() {
            Some((level, msg, count)) if *level == line.level && *msg == line.msg => *count += 1,
            _ => shown.push((&line.level, &line.msg, 1)),
        }
    }
    for (level, msg, count) in &shown[shown.len().saturating_sub(ERROR_LINES)..] {
        let times = if *count > 1 {
            format!(" (×{count})")
        } else {
            String::new()
        };
        eprintln!("    {level}{times} {msg}");
    }
    true
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
    if expected.intents != actual.intents {
        lines.push(format!(
            "~ clicks opened: {} expected, {} now",
            expected.intents.len(),
            actual.intents.len()
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

    const MANIFEST: &str = "abi: 1\nid: test.st\nversion: 0.1.0\nname: {en: Test}\nlang: rust\n\
        min_terminal: 0.104.0\ntimer_ms: 1000\ncolumns: [{key: v, type: number}]\n";

    /// `on_timer` sends `{"rows": [{"key": "a", "cells": {"v": 1}}]}` to emit_rows, then ends
    /// with `tail` (`(i32.const 0)` = success).
    fn plugin(tail: &str) -> Vec<u8> {
        let rows = r#"{"rows":[{"key":"a","cells":{"v":1}}]}"#;
        let len = rows.len();
        let data = rows.replace('"', "\\\"");
        let wat = format!(
            r#"(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "error_set" (func $error_set (param i64)))
  (import "extism:host/user" "emit_rows" (func $emit_rows (param i64) (result i64)))
  (memory 1)
  (data (i32.const 0) "{data}")
  (data (i32.const 256) "boom")
  (func $copy (param $ptr i32) (param $len i32) (result i64)
    (local $off i64) (local $i i32)
    (local.set $off (call $alloc (i64.extend_i32_u (local.get $len))))
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (call $store_u8
          (i64.add (local.get $off) (i64.extend_i32_u (local.get $i)))
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (local.get $off))
  (func (export "init") (result i32) (i32.const 0))
  (func (export "on_timer") (result i32)
    (drop (call $emit_rows (call $copy (i32.const 0) (i32.const {len}))))
    {tail}))"#
        );
        wat::parse_str(wat).unwrap()
    }

    const FAIL: &str = "(call $error_set (call $copy (i32.const 256) (i32.const 4))) (i32.const 1)";

    fn project(name: &str, wasm: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("st-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(RECORDINGS_DIR)).unwrap();
        std::fs::write(dir.join("manifest.yaml"), MANIFEST).unwrap();
        std::fs::write(dir.join("screener.wasm"), wasm).unwrap();
        let recording = serde_json::json!({
            "v": 1, "id": "test.st", "version": "0.1.0", "terminal": "0.104.71", "lang": "en",
            "params": {}, "started_ms": 1000,
            "events": [
                {"kind": "init", "t_ms": 1000, "input": {"params": {}, "terminal": "0.104.71", "lang": "en", "now_ms": 1000}},
                {"kind": "timer", "t_ms": 2000, "input": {"now_ms": 2000}}
            ]
        });
        std::fs::write(
            dir.join(RECORDINGS_DIR).join("r.json"),
            recording.to_string(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_clean_run_writes_its_snapshot_then_passes() {
        let dir = project("ok", &plugin("(i32.const 0)"));
        run(&dir, None, false, false).unwrap();
        assert!(dir.join(RECORDINGS_DIR).join("r.expected.json").is_file());
        run(&dir, None, false, false).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failing_calls_fail_the_test_and_write_no_snapshot() {
        for (name, tail) in [("error", FAIL), ("trap", "(unreachable)")] {
            let dir = project(name, &plugin(tail));
            assert!(run(&dir, None, false, false).is_err(), "{name}");
            assert!(run(&dir, None, true, false).is_err(), "{name}");
            assert!(
                !dir.join(RECORDINGS_DIR).join("r.expected.json").exists(),
                "{name}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn call_errors_show_the_plugin_message() {
        let manifest = space_screener_check::Manifest::parse(MANIFEST).unwrap();
        let dir = project("message", &plugin(FAIL));
        let recording = read_recording(&dir.join(RECORDINGS_DIR).join("r.json")).unwrap();
        let output = replay(&manifest, &plugin(FAIL), &recording).unwrap();
        assert_eq!(output.stats.failures, 1);
        assert!(
            output
                .logs
                .iter()
                .any(|l| l.level == "error" && l.msg.contains("boom")),
            "{:?}",
            output.logs
        );
        assert!(print_call_errors(&output));
        std::fs::remove_dir_all(&dir).unwrap();
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
            intents: Vec::new(),
        };
        let actual = Snapshot {
            rows: vec![row("b", 3), row("c", 4)],
            alerts: Vec::new(),
            status: None,
            intents: Vec::new(),
        };
        let lines = diff(&expected, &actual);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("- row a"));
        assert_eq!(lines[1], "~ row b: v 2 → 3");
        assert!(lines[2].starts_with("+ row c"));
    }
}
