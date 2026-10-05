//! Where replies to data host functions come from: a recording or a small offline data set.

use std::collections::{HashMap, VecDeque};

use serde_json::{Value, json};
use space_screener_check::recording::{Event, Recording};

/// Replies to the data host functions (`http`, `http_batch`, `tickers`, `symbols`,
/// `exchanges`, `history_*`, `signals`, `now_ms`). Local host functions (kv, rows, logs) never
/// reach it.
pub trait Source: Send {
    /// The reply envelope (`{"ok": …}` or `{"err": {code, message}}`), or `None` when the
    /// source has nothing for this call; the harness then answers `transport` and counts a miss
    /// (for `now_ms` it answers with the virtual clock instead).
    fn call(&mut self, function: &str, input: &Value, now_ms: i64) -> Option<Value>;
}

pub fn ok(value: Value) -> Value {
    json!({ "ok": value })
}

pub fn err(code: &str, message: impl Into<String>) -> Value {
    json!({ "err": { "code": code, "message": message.into() } })
}

/// Replies from a recording: the first unused recorded call with the same function and the
/// same JSON input (key order does not matter).
#[derive(Debug, Default)]
pub struct Replay {
    calls: HashMap<(String, String), VecDeque<Value>>,
}

impl Replay {
    pub fn new(recording: &Recording) -> Self {
        let mut calls: HashMap<(String, String), VecDeque<Value>> = HashMap::new();
        for event in &recording.events {
            if let Event::Call {
                function,
                input,
                output,
                ..
            } = event
            {
                calls
                    .entry((function.clone(), input.to_string()))
                    .or_default()
                    .push_back(output.clone());
            }
        }
        Self { calls }
    }

    /// Recorded calls nobody asked for yet.
    pub fn unused(&self) -> usize {
        self.calls.values().map(VecDeque::len).sum()
    }
}

impl Source for Replay {
    fn call(&mut self, function: &str, input: &Value, _now_ms: i64) -> Option<Value> {
        self.calls
            .get_mut(&(function.to_string(), input.to_string()))?
            .pop_front()
    }
}

/// A fixed, network-free data set for trial runs: a few exchanges and USDT pairs, no HTTP,
/// no history and no signals. Enough to see that a plugin starts, survives failed requests and
/// emits rows.
#[derive(Debug, Default)]
pub struct Offline;

pub const OFFLINE_EXCHANGES: &[(&str, &str)] = &[
    ("binance", "futures"),
    ("binance", "spot"),
    ("bybit", "futures"),
    ("okx", "futures"),
];

/// base, last price, 24h change %, 24h quote volume.
const OFFLINE_TICKERS: &[(&str, f64, f64, f64)] = &[
    ("BTC", 65_000.0, 1.2, 15_000_000_000.0),
    ("ETH", 3_200.0, -0.8, 8_000_000_000.0),
    ("SOL", 150.0, 3.4, 2_000_000_000.0),
    ("XRP", 0.55, 0.4, 900_000_000.0),
    ("DOGE", 0.12, -2.1, 600_000_000.0),
];

const OFFLINE: &str = "offline: the trial run has no network";

fn known(input: &Value) -> Result<(), Value> {
    let exchange = input.get("exchange").and_then(Value::as_str);
    let market = input.get("market").and_then(Value::as_str);
    match (exchange, market) {
        (Some(exchange), Some(market)) if OFFLINE_EXCHANGES.contains(&(exchange, market)) => Ok(()),
        (Some(exchange), Some(market)) => Err(err(
            "unknown_exchange",
            format!("{exchange} {market} is not in the offline data set"),
        )),
        _ => Err(err("bad_request", "exchange and market are required")),
    }
}

impl Source for Offline {
    fn call(&mut self, function: &str, input: &Value, now_ms: i64) -> Option<Value> {
        Some(match function {
            "exchanges" => ok(OFFLINE_EXCHANGES
                .iter()
                .map(|(exchange, market)| {
                    json!({ "exchange": exchange, "market": market, "connected": true })
                })
                .collect()),
            "symbols" => match known(input) {
                Ok(()) => ok(OFFLINE_TICKERS
                    .iter()
                    .map(|(base, ..)| {
                        json!({ "symbol": format!("{base}USDT"), "base": base, "quote": "USDT", "trading": true })
                    })
                    .collect()),
                Err(reply) => reply,
            },
            "tickers" => match known(input) {
                Ok(()) => ok(json!({
                    "ts_ms": now_ms,
                    "tickers": OFFLINE_TICKERS
                        .iter()
                        .map(|(base, last, change_pct, volume_quote)| json!({
                            "symbol": format!("{base}USDT"),
                            "base": base,
                            "quote": "USDT",
                            "last": last,
                            "change_pct": change_pct,
                            "volume_quote": volume_quote,
                        }))
                        .collect::<Vec<_>>(),
                })),
                Err(reply) => reply,
            },
            "http" | "history_cluster" | "history_replay" => err("transport", OFFLINE),
            "signals" => ok(json!({
                "seq": 0, "reset": true, "connected": false, "upserts": [], "removed": []
            })),
            "http_batch" => {
                let requests = input
                    .get("requests")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                ok((0..requests).map(|_| err("transport", OFFLINE)).collect())
            }
            _ => return None,
        })
    }
}
