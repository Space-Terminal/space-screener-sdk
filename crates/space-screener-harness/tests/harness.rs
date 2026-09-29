use std::collections::BTreeMap;

use serde_json::json;
use space_screener_check::Manifest;
use space_screener_check::recording::{Event, Recording};
use space_screener_harness::{CallOutcome, Harness, Offline, Verdict, replay, trial};

const MANIFEST: &str = "abi: 1\nid: test.wat\nversion: 0.1.0\nname: {en: Wat}\nlang: rust\n\
    min_terminal: 0.104.0\ntimer_ms: 1000\ncolumns: [{key: oi, type: number}]\n\
    limits: {cpu_ms_per_call: 50}\n";

/// A plugin whose `on_timer` sends `payload` to host function `first`, then passes the reply
/// bytes as they are to `second` (when given), so a test controls both through the data.
fn plugin(init: &str, first: &str, payload: &str, second: Option<&str>) -> Vec<u8> {
    let escaped = payload.replace('\\', "\\\\").replace('"', "\\\"");
    let len = payload.len();
    let second_import = second
        .map(|name| {
            format!(
                r#"(import "extism:host/user" "{name}" (func $second (param i64) (result i64)))"#
            )
        })
        .unwrap_or_default();
    let forward = if second.is_some() {
        "(drop (call $second (local.get $reply)))"
    } else {
        ""
    };
    let wat = format!(
        r#"(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/user" "{first}" (func $first (param i64) (result i64)))
  {second_import}
  (memory 1)
  (data (i32.const 0) "{escaped}")
  (func $send (param $ptr i32) (param $len i32) (result i64)
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
  (func (export "init") (result i32) {init})
  (func (export "on_timer") (result i32)
    (local $reply i64)
    (local.set $reply (call $first (call $send (i32.const 0) (i32.const {len}))))
    {forward}
    (i32.const 0)))"#
    );
    wat::parse_str(wat).unwrap()
}

fn manifest() -> Manifest {
    Manifest::parse(MANIFEST).unwrap()
}

const ROWS: &str = r#"{"rows":[{"key":"BTCUSDT","symbol":"BTCUSDT","cells":{"oi":5,"extra":1}}]}"#;

#[test]
fn trial_offline_shows_rows_and_column_warnings() {
    let wasm = plugin("(i32.const 0)", "emit_rows", ROWS, None);
    let report = trial(&manifest(), &wasm, None);
    assert_eq!(report.total_rows, 1, "{report:?}");
    assert_eq!(report.sample_rows[0].cells["oi"], json!(5));
    assert_eq!(report.verdict, Verdict::Warn);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.contains("`extra` is not a declared column")),
        "{:?}",
        report.issues
    );
}

#[test]
fn trap_in_init_fails_the_trial() {
    let wasm = plugin("(unreachable)", "emit_rows", ROWS, None);
    let report = trial(&manifest(), &wasm, None);
    assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    assert!(
        report.issues[0].starts_with("init trapped"),
        "{:?}",
        report.issues
    );
}

#[test]
fn forbidden_module_fails_before_running() {
    let wasm = wat::parse_str(
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
           (func (export "init") (result i32) i32.const 0)
           (func (export "on_timer") (result i32) i32.const 0))"#,
    )
    .unwrap();
    let report = trial(&manifest(), &wasm, None);
    assert_eq!(report.verdict, Verdict::Fail);
    assert!(
        report.issues[0].starts_with("forbidden_import"),
        "{:?}",
        report.issues
    );
}

#[test]
fn busy_loop_with_host_calls_is_cut_at_its_budget() {
    let wasm = wat::parse_str(
        r#"(module
  (import "extism:host/user" "now_ms" (func $now (param i64) (result i64)))
  (func (export "init") (result i32) i32.const 0)
  (func (export "on_timer") (result i32)
    (loop $spin (drop (call $now (i64.const 0))) (br $spin))
    (i32.const 0)))"#,
    )
    .unwrap();
    let mut harness = Harness::new(&manifest(), &wasm, Box::new(Offline)).unwrap();
    assert!(matches!(
        harness
            .init(&BTreeMap::new(), "0.104.71", Default::default(), 0)
            .unwrap(),
        CallOutcome::Done { .. }
    ));
    let outcome = harness.timer(1000).unwrap();
    assert!(matches!(outcome, CallOutcome::Cut { .. }), "{outcome:?}");
    assert!(outcome.needs_restart());
    harness.restart().unwrap();
    assert_eq!(harness.output().unwrap().stats.over_budget, 1);
}

fn recording(events: Vec<Event>, truncated: bool) -> Recording {
    Recording {
        v: 1,
        id: "test.wat".to_string(),
        version: "0.1.0".to_string(),
        terminal: "0.104.71".to_string(),
        lang: "en".to_string(),
        params: BTreeMap::new(),
        kv: BTreeMap::new(),
        started_ms: 1_000,
        events,
        truncated,
    }
}

#[test]
fn replay_answers_from_the_recording_and_counts_misses() {
    let request = r#"{"market":"futures","exchange":"binance"}"#;
    let wasm = plugin("(i32.const 0)", "symbols", request, Some("emit_rows"));
    let recorded_rows =
        json!({"rows": [{"key": "FROM-RECORDING", "cells": {"oi": 7}, "ttl_s": 1}]});
    let events = vec![
        Event::Init {
            t_ms: 1_000,
            input: json!({"params": {}, "terminal": "0.104.71", "lang": "en", "now_ms": 1_000}),
        },
        Event::Timer {
            t_ms: 2_000,
            input: json!({"now_ms": 2_000}),
        },
        Event::Call {
            t_ms: 2_001,
            function: "symbols".to_string(),
            input: json!({"exchange": "binance", "market": "futures"}),
            output: recorded_rows,
        },
        Event::Timer {
            t_ms: 2_500,
            input: json!({"now_ms": 2_500}),
        },
    ];
    let output = replay(&manifest(), &wasm, &recording(events, true)).unwrap();
    assert_eq!(output.rows.len(), 1, "{output:?}");
    assert_eq!(output.rows[0].key, "FROM-RECORDING");
    assert_eq!(
        output.stats.misses, 1,
        "the second timer asks again and is not recorded"
    );
    assert!(output.warnings.iter().any(|w| w.contains("size cap")));

    let expired = replay(
        &manifest(),
        &wasm,
        &recording(
            vec![
                Event::Timer {
                    t_ms: 2_000,
                    input: json!({"now_ms": 2_000}),
                },
                Event::Call {
                    t_ms: 2_001,
                    function: "symbols".to_string(),
                    input: json!({"exchange": "binance", "market": "futures"}),
                    output: json!({"rows": [{"key": "A", "cells": {"oi": 1}, "ttl_s": 1}]}),
                },
                Event::Timer {
                    t_ms: 9_000,
                    input: json!({"now_ms": 9_000}),
                },
            ],
            false,
        ),
    )
    .unwrap();
    assert!(
        expired.rows.is_empty(),
        "the row's 1 s TTL ran out on the virtual clock"
    );
}

#[test]
fn trial_with_a_recording_uses_it() {
    let wasm = plugin(
        "(i32.const 0)",
        "symbols",
        r#"{"exchange":"binance","market":"futures"}"#,
        Some("emit_rows"),
    );
    let events = vec![
        Event::Timer {
            t_ms: 2_000,
            input: json!({"now_ms": 2_000}),
        },
        Event::Call {
            t_ms: 2_001,
            function: "symbols".to_string(),
            input: json!({"exchange": "binance", "market": "futures"}),
            output: json!({"rows": [{"key": "R", "cells": {"oi": 3}}]}),
        },
    ];
    let report = trial(&manifest(), &wasm, Some(&recording(events, false)));
    assert!(report.recorded);
    assert_eq!(report.verdict, Verdict::Ok, "{report:?}");
    assert_eq!(report.total_rows, 1);
}
