//! A recording of one plugin run in the terminal (`st record`), replayed headless by `st test`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub v: u32,
    pub id: String,
    pub version: String,
    /// Terminal version that made the recording.
    pub terminal: String,
    /// Interface language: `ru` or `en`.
    pub lang: String,
    pub params: BTreeMap<String, Value>,
    /// The plugin's kv at `init`: kv survives restarts, so replay starts from it.
    #[serde(default)]
    pub kv: BTreeMap<String, Value>,
    pub started_ms: i64,
    pub events: Vec<Event>,
    /// The recording hit its size cap and ended before the requested duration.
    #[serde(default)]
    pub truncated: bool,
}

/// Entry points the host called (`init`, `timer`, `click`) and the data host functions the
/// plugin called, in the order they happened. Local host functions (kv, emit_rows, log, …)
/// are not recorded: replay runs them for real.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Event {
    Init {
        t_ms: i64,
        input: Value,
    },
    Timer {
        t_ms: i64,
        input: Value,
    },
    Click {
        t_ms: i64,
        input: Value,
    },
    Call {
        t_ms: i64,
        #[serde(rename = "fn")]
        function: String,
        /// The plugin's JSON input (`null` when empty).
        input: Value,
        /// The host's reply envelope: `{"ok": …}` or `{"err": {code, message}}`.
        output: Value,
    },
}

impl Event {
    pub fn t_ms(&self) -> i64 {
        match self {
            Self::Init { t_ms, .. }
            | Self::Timer { t_ms, .. }
            | Self::Click { t_ms, .. }
            | Self::Call { t_ms, .. } => *t_ms,
        }
    }
}

/// Host functions whose replies come from outside the plugin and therefore go into a
/// recording. `now_ms` is here so that replay reproduces the clock the plugin saw.
pub const RECORDED_FUNCTIONS: &[&str] = &[
    "http",
    "http_batch",
    "tickers",
    "symbols",
    "exchanges",
    "history_cluster",
    "history_replay",
    "signals",
    "now_ms",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_tagged_by_kind() {
        let json = r#"{"v":1,"id":"ivan.oi","version":"0.1.0","terminal":"0.104.71","lang":"ru",
            "params":{"n":5},"started_ms":1000,"events":[
            {"kind":"init","t_ms":1000,"input":{"now_ms":1000}},
            {"kind":"call","t_ms":1001,"fn":"tickers","input":{"exchange":"binance","market":"futures"},"output":{"ok":{"ts_ms":1,"tickers":[]}}},
            {"kind":"timer","t_ms":2000,"input":{"now_ms":2000}}]}"#;
        let recording: Recording = serde_json::from_str(json).unwrap();
        assert_eq!(recording.events.len(), 3);
        assert!(
            matches!(&recording.events[1], Event::Call { function, .. } if function == "tickers")
        );
        assert_eq!(recording.events[2].t_ms(), 2000);
        assert!(recording.kv.is_empty());
        assert!(!recording.truncated);
        let back: Recording =
            serde_json::from_value(serde_json::to_value(&recording).unwrap()).unwrap();
        assert_eq!(back, recording);
    }

    #[test]
    fn kv_and_truncation_roundtrip() {
        let json = r#"{"v":1,"id":"ivan.oi","version":"0.1.0","terminal":"0.104.71","lang":"en",
            "params":{},"kv":{"seen":["BTCUSDT"]},"started_ms":0,"events":[],"truncated":true}"#;
        let recording: Recording = serde_json::from_str(json).unwrap();
        assert_eq!(recording.kv["seen"], serde_json::json!(["BTCUSDT"]));
        assert!(recording.truncated);
    }
}
