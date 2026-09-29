use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A row as the pane would show it, after TTL and caps. Cells are the plugin's JSON as is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(default)]
    pub cells: Map<String, Value>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rank: i32,
}

fn is_zero(rank: &i32) -> bool {
    *rank == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub t_ms: i64,
    pub level: String,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub t_ms: i64,
    pub level: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub text: String,
    pub tone: String,
}

/// `open_market` / `open_spread` a click asked for; the terminal would open that market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub t_ms: i64,
    pub kind: String,
    pub target: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Host function calls of any kind.
    pub host_calls: u64,
    /// Data calls the source had no reply for (a replay that drifted from its recording).
    pub misses: u64,
    /// The most CPU one call took, host functions included.
    pub cpu_ms_max: u64,
    pub traps: u32,
    /// Calls that returned an error.
    pub failures: u32,
    /// Calls over their CPU budget, cut or not.
    pub over_budget: u32,
    /// `http` hosts the plugin asked for without declaring them in the manifest.
    pub undeclared_hosts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// Sorted by key.
    pub rows: Vec<Row>,
    pub logs: Vec<LogLine>,
    pub alerts: Vec<Alert>,
    pub status: Option<Status>,
    pub intents: Vec<Intent>,
    /// Rows that do not match the manifest columns, dropped rows, a truncated recording.
    pub warnings: Vec<String>,
    pub stats: Stats,
}
