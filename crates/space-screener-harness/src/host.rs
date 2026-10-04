//! Host functions of ABI v1 without a terminal: local ones (kv, rows, alerts, status, logs,
//! clicks) run here for real, data ones go to a [`Source`].

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpu_time::ThreadTime;
use extism::{CurrentPlugin, Function, PTR, UserData, Val};
use serde::Deserialize;
use serde_json::{Map, Value};
use space_screener_check::{ColumnType, HOST_FUNCTIONS, HistoryKind, Manifest, SignalSource};

use crate::output::{Alert, Intent, LogLine, Output, Row, Stats, Status};
use crate::source::{Source, err, ok};

pub const MAX_ROWS: usize = 10_000;
pub const MAX_ROW_KEY: usize = 128;
pub const MAX_ROW_BYTES: usize = 16 * 1024;
pub const MAX_BUFFER_BYTES: usize = 32 * 1024 * 1024;
pub const KV_LIMIT_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH: usize = 1000;
pub const MAX_OPEN_MARKETS: usize = 16;
const ALERTS_PER_MINUTE: usize = 6;
const MAX_ALERT_TITLE: usize = 200;
const MAX_ALERT_BODY: usize = 1000;
const MAX_STATUS_TEXT: usize = 128;
const MAX_LOG_LINES: usize = 1000;
/// Bytes of one log line, as in the terminal.
const MAX_LOG_LINE: usize = 4096;
const MAX_WARNINGS: usize = 100;
/// Characters of one warning: they quote author-controlled keys and hosts.
const MAX_WARNING: usize = 300;
const MAX_UNDECLARED_HOSTS: usize = 20;
const MAX_ALERTS: usize = 1000;

/// CPU of the calling thread; `None` when the OS does not report it (then wall time counts).
fn thread_cpu() -> Option<Duration> {
    ThreadTime::try_now().ok().map(|time| time.as_duration())
}

/// CPU spent by the current plugin call. Host functions run on the same thread, so the
/// budget covers them too, as in the terminal.
#[derive(Debug, Default)]
pub(crate) struct Meter {
    started: Option<(Instant, Option<Duration>)>,
    budget: Duration,
    pub cut: bool,
}

impl Meter {
    pub fn begin(&mut self, budget: Duration) {
        self.started = Some((Instant::now(), thread_cpu()));
        self.budget = budget;
        self.cut = false;
    }

    pub fn spent(&self) -> Duration {
        match self.started {
            Some((_, Some(cpu))) => {
                thread_cpu().map_or(Duration::ZERO, |now| now.saturating_sub(cpu))
            }
            Some((wall, None)) => wall.elapsed(),
            None => Duration::ZERO,
        }
    }

    pub fn over_budget(&self) -> bool {
        self.started.is_some() && self.spent() > self.budget
    }
}

#[derive(Debug)]
struct StoredRow {
    row: Row,
    expires_at_ms: Option<i64>,
    seq: u64,
    weight: usize,
}

#[derive(Debug, Default)]
struct Rows {
    rows: HashMap<String, StoredRow>,
    bytes: usize,
    seq: u64,
}

impl Rows {
    fn upsert(&mut self, rows: Vec<(Row, Option<u32>, usize)>, replace: bool, now_ms: i64) {
        if replace {
            self.rows.clear();
            self.bytes = 0;
        }
        for (row, ttl_s, weight) in rows {
            self.seq += 1;
            let stored = StoredRow {
                expires_at_ms: ttl_s.map(|ttl| now_ms + i64::from(ttl) * 1000),
                seq: self.seq,
                weight,
                row,
            };
            self.bytes += weight;
            if let Some(old) = self.rows.insert(stored.row.key.clone(), stored) {
                self.bytes -= old.weight;
            }
        }
        self.evict_over_caps();
    }

    /// Drops the rows updated longest ago; sorted once, so a huge emit stays O(n log n).
    fn evict_over_caps(&mut self) {
        if self.rows.len() <= MAX_ROWS && self.bytes <= MAX_BUFFER_BYTES {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .rows
            .iter()
            .map(|(key, stored)| (stored.seq, key.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, key) in by_age {
            if self.rows.len() <= MAX_ROWS && self.bytes <= MAX_BUFFER_BYTES {
                break;
            }
            if let Some(old) = self.rows.remove(&key) {
                self.bytes -= old.weight;
            }
        }
    }

    fn expire<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        for key in keys {
            if let Some(old) = self.rows.remove(key) {
                self.bytes -= old.weight;
            }
        }
    }

    fn visible(&self, now_ms: i64) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .rows
            .values()
            .filter(|stored| stored.expires_at_ms.is_none_or(|at| at > now_ms))
            .map(|stored| stored.row.clone())
            .collect();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        rows
    }
}

pub(crate) struct State {
    pub manifest: Arc<Manifest>,
    pub source: Box<dyn Source>,
    pub now_ms: i64,
    pub meter: Meter,
    /// `Some(used)` while `on_click` runs.
    pub click: Option<bool>,
    rows: Rows,
    kv: BTreeMap<String, Value>,
    kv_bytes: usize,
    logs: VecDeque<LogLine>,
    alerts: Vec<Alert>,
    alert_times: VecDeque<i64>,
    status: Option<Status>,
    intents: Vec<Intent>,
    warnings: BTreeSet<String>,
    pub stats: Stats,
}

fn kv_entry_size(key: &str, value: &Value) -> usize {
    key.len() + serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

impl State {
    pub fn new(manifest: Arc<Manifest>, source: Box<dyn Source>) -> Self {
        Self {
            manifest,
            source,
            now_ms: 0,
            meter: Meter::default(),
            click: None,
            rows: Rows::default(),
            kv: BTreeMap::new(),
            kv_bytes: 0,
            logs: VecDeque::new(),
            alerts: Vec::new(),
            alert_times: VecDeque::new(),
            status: None,
            intents: Vec::new(),
            warnings: BTreeSet::new(),
            stats: Stats::default(),
        }
    }

    /// Seeds kv (a recording's kv at init). Entries over the 1 MiB total are dropped with a warning.
    pub fn seed_kv(&mut self, kv: BTreeMap<String, Value>) {
        for (key, value) in kv {
            if let Err(message) = self.kv_set(key, value) {
                self.warn(format!("recorded kv: {message}"));
            }
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.insert(truncate(message.into(), MAX_WARNING));
        }
    }

    pub fn host_log(&mut self, level: &str, msg: impl Into<String>) {
        let mut msg = msg.into();
        if msg.len() > MAX_LOG_LINE {
            let cut = (0..=MAX_LOG_LINE)
                .rev()
                .find(|&i| msg.is_char_boundary(i))
                .unwrap_or(0);
            msg.truncate(cut);
        }
        if self.logs.len() == MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(LogLine {
            t_ms: self.now_ms,
            level: truncate(level.to_string(), 16),
            msg,
        });
    }

    pub fn output(&self) -> Output {
        Output {
            rows: self.rows.visible(self.now_ms),
            logs: self.logs.iter().cloned().collect(),
            alerts: self.alerts.clone(),
            status: self.status.clone(),
            intents: self.intents.clone(),
            warnings: self.warnings.iter().cloned().collect(),
            stats: self.stats.clone(),
        }
    }

    fn kv_set(&mut self, key: String, value: Value) -> Result<(), String> {
        let old = self.kv.get(&key).map_or(0, |old| kv_entry_size(&key, old));
        if value.is_null() {
            if self.kv.remove(&key).is_some() {
                self.kv_bytes -= old;
            }
            return Ok(());
        }
        let size = self.kv_bytes - old + kv_entry_size(&key, &value);
        if size > KV_LIMIT_BYTES {
            return Err(format!(
                "kv would take {size} bytes, limit is {KV_LIMIT_BYTES}"
            ));
        }
        self.kv.insert(key, value);
        self.kv_bytes = size;
        Ok(())
    }

    /// One host function call: JSON in, reply envelope out.
    pub fn call(&mut self, name: &str, input: &Value) -> Value {
        self.stats.host_calls += 1;
        match name {
            "http" => self.http(input),
            "http_batch" => self.http_batch(input),
            "tickers" | "symbols" | "exchanges" => self.data(name, input),
            "history_cluster" => self.history(HistoryKind::Cluster, name, input),
            "history_replay" => self.history(HistoryKind::Replay, name, input),
            "signals" => self.signals(input),
            "now_ms" => {
                let now = self.now_ms;
                self.source
                    .call(name, input, now)
                    .unwrap_or_else(|| ok(Value::from(now)))
            }
            "kv_get" => reply(
                parse::<KvGet>(input)
                    .map(|request| self.kv.get(&request.key).cloned().unwrap_or(Value::Null)),
            ),
            "kv_set" => reply(parse::<KvSet>(input).and_then(|request| {
                self.kv_set(request.key, request.value)
                    .map(|()| Value::Null)
                    .map_err(|message| err("too_large", message))
            })),
            "emit_rows" => {
                reply(parse::<EmitRows>(input).and_then(|request| self.emit_rows(request)))
            }
            "expire" => reply(parse::<Expire>(input).map(|request| {
                self.rows.expire(request.keys.iter().map(String::as_str));
                Value::Null
            })),
            "emit_alert" => reply(parse::<AlertInput>(input).map(|alert| self.emit_alert(alert))),
            "set_status" => reply(parse::<StatusInput>(input).map(|status| {
                self.status = Some(Status {
                    text: truncate(status.text, MAX_STATUS_TEXT),
                    tone: status.tone.unwrap_or_else(|| "neutral".to_string()),
                });
                Value::Null
            })),
            "open_market" | "open_spread" => self.open(name, input),
            "open_markets" => match parse::<OpenMarkets>(input) {
                Ok(request) if (1..=MAX_OPEN_MARKETS).contains(&request.markets.len()) => {
                    self.open(name, input)
                }
                Ok(request) => err(
                    "bad_request",
                    format!(
                        "open_markets takes 1..={MAX_OPEN_MARKETS} markets, got {}",
                        request.markets.len()
                    ),
                ),
                Err(reply) => reply,
            },
            "log" => reply(parse::<LogInput>(input).map(|line| {
                self.host_log(&line.level, line.msg);
                Value::Null
            })),
            other => err("bad_request", format!("unknown host function {other}")),
        }
    }

    fn data(&mut self, name: &str, input: &Value) -> Value {
        let now = self.now_ms;
        match self.source.call(name, input, now) {
            Some(reply) => reply,
            None => {
                self.stats.misses += 1;
                err("transport", format!("{name}: not recorded"))
            }
        }
    }

    fn http(&mut self, input: &Value) -> Value {
        let url = input.get("url").and_then(Value::as_str).unwrap_or_default();
        match self.allowed_host(url) {
            Ok(()) => self.data("http", input),
            Err(reply) => reply,
        }
    }

    fn http_batch(&mut self, input: &Value) -> Value {
        let Some(requests) = input.get("requests").and_then(Value::as_array) else {
            return err("bad_request", "http_batch needs {requests: [...]}");
        };
        if requests.len() > MAX_BATCH {
            return err(
                "bad_request",
                format!("{} requests, the limit is {MAX_BATCH}", requests.len()),
            );
        }
        let hosts: Vec<String> = requests
            .iter()
            .filter_map(|request| request.get("url").and_then(Value::as_str))
            .filter_map(host_of)
            .collect();
        for host in hosts {
            self.declared(&host);
        }
        self.data("http_batch", input)
    }

    fn allowed_host(&mut self, url: &str) -> Result<(), Value> {
        let Some(host) = host_of(url) else {
            return Err(err("bad_request", "only https:// URLs are allowed"));
        };
        if self.declared(&host) {
            Ok(())
        } else {
            Err(err(
                "host_not_allowed",
                format!("{host} is not listed in manifest.yaml http"),
            ))
        }
    }

    /// Whether the manifest declares `host`; an undeclared one is reported once.
    fn declared(&mut self, host: &str) -> bool {
        if self.manifest.http.iter().any(|allowed| allowed == host) {
            return true;
        }
        if self.stats.undeclared_hosts.len() < MAX_UNDECLARED_HOSTS
            && !self.stats.undeclared_hosts.iter().any(|seen| seen == host)
        {
            self.stats
                .undeclared_hosts
                .push(truncate(host.to_string(), 253));
            self.warn(format!(
                "http to {host}, which manifest.yaml does not declare"
            ));
        }
        false
    }

    fn history(&mut self, kind: HistoryKind, name: &str, input: &Value) -> Value {
        if !self.manifest.allows_history(kind) {
            return err(
                "not_permitted",
                format!("{name} needs `history` in manifest.yaml"),
            );
        }
        self.data(name, input)
    }

    fn signals(&mut self, input: &Value) -> Value {
        let request = match parse::<SignalsInput>(input) {
            Ok(request) => request,
            Err(reply) => return reply,
        };
        if !self.manifest.allows_signals(request.source) {
            let source = request.source.as_str();
            return err(
                "not_permitted",
                format!("signals of {source} need `signals: [{source}]` in manifest.yaml"),
            );
        }
        self.data("signals", input)
    }

    fn emit_rows(&mut self, mut request: EmitRows) -> Result<Value, Value> {
        // Rows past the cap would be evicted right away as the oldest; skip their work.
        if request.rows.len() > MAX_ROWS {
            let extra = request.rows.len() - MAX_ROWS;
            request.rows.drain(..extra);
            self.host_log(
                "warn",
                format!("emit_rows: {extra} rows over the {MAX_ROWS}-row cap were dropped"),
            );
            self.warn(format!("one emit_rows call sent more than {MAX_ROWS} rows"));
        }
        let mut prepared = Vec::with_capacity(request.rows.len());
        let mut dropped = 0usize;
        for input in request.rows {
            let weight = serde_json::to_vec(&input).map_or(0, |bytes| bytes.len());
            if weight > MAX_ROW_BYTES {
                return Err(err(
                    "bad_request",
                    format!(
                        "row {} is about {weight} bytes, limit is {MAX_ROW_BYTES}",
                        input.key
                    ),
                ));
            }
            if input.key.is_empty() || input.key.chars().count() > MAX_ROW_KEY {
                dropped += 1;
                continue;
            }
            if let Some(market) = input.market.as_deref()
                && market != "spot"
                && market != "futures"
            {
                return Err(err(
                    "bad_request",
                    format!("row {}: market must be spot or futures", input.key),
                ));
            }
            self.check_cells(&input.cells);
            let row = Row {
                key: input.key,
                symbol: input.symbol,
                exchange: input
                    .exchange
                    .map(|exchange| exchange.trim().to_ascii_lowercase()),
                market: input.market,
                cells: input.cells,
                rank: input.rank,
            };
            prepared.push((row, input.ttl_s, weight));
        }
        if dropped > 0 {
            self.host_log(
                "warn",
                format!(
                    "emit_rows: {dropped} rows dropped, key must be 1..={MAX_ROW_KEY} characters"
                ),
            );
            self.warn(format!(
                "rows dropped: key must be 1..={MAX_ROW_KEY} characters"
            ));
        }
        let now = self.now_ms;
        self.rows.upsert(prepared, request.replace, now);
        Ok(Value::Null)
    }

    fn check_cells(&mut self, cells: &Map<String, Value>) {
        for (key, cell) in cells {
            let Some(column) = self
                .manifest
                .columns
                .iter()
                .find(|column| column.key == *key)
            else {
                self.warn(format!("cell `{key}` is not a declared column"));
                continue;
            };
            let value = match cell {
                Value::Object(styled) => styled.get("v").unwrap_or(&Value::Null),
                plain => plain,
            };
            let fits = match column.kind {
                _ if value.is_null() => true,
                ColumnType::Number
                | ColumnType::Integer
                | ColumnType::Percent
                | ColumnType::Usd
                | ColumnType::Price
                | ColumnType::Time
                | ColumnType::Duration
                | ColumnType::Countdown => value.is_number(),
                ColumnType::Text | ColumnType::Symbol | ColumnType::Exchange => value.is_string(),
                ColumnType::Bool => value.is_boolean(),
                ColumnType::Exchanges => value.as_array().is_some_and(|items| {
                    items
                        .iter()
                        .all(|item| item.get("exchange").is_some_and(Value::is_string))
                }),
            };
            if !fits {
                self.warn(format!(
                    "column `{key}` is {} but got {}",
                    column.kind.as_str(),
                    json_kind(value)
                ));
            }
        }
    }

    fn emit_alert(&mut self, alert: AlertInput) -> Value {
        let now = self.now_ms;
        while self
            .alert_times
            .front()
            .is_some_and(|at| now - *at >= 60_000)
        {
            self.alert_times.pop_front();
        }
        if self.alert_times.len() >= ALERTS_PER_MINUTE {
            self.host_log(
                "warn",
                format!("emit_alert dropped: more than {ALERTS_PER_MINUTE} alerts per minute"),
            );
            return Value::Null;
        }
        self.alert_times.push_back(now);
        if self.alerts.len() == MAX_ALERTS {
            self.alerts.remove(0);
        }
        self.alerts.push(Alert {
            t_ms: now,
            level: truncate(alert.level, 16),
            title: truncate(alert.title, MAX_ALERT_TITLE),
            body: truncate(alert.body, MAX_ALERT_BODY),
            row_key: alert.row_key,
        });
        Value::Null
    }

    fn open(&mut self, name: &str, input: &Value) -> Value {
        match self.click {
            Some(false) => {
                self.click = Some(true);
                self.intents.push(Intent {
                    t_ms: self.now_ms,
                    kind: name.to_string(),
                    target: input.clone(),
                });
                ok(Value::Null)
            }
            _ => err(
                "not_in_click",
                "open_* works only inside on_click, once per click",
            ),
        }
    }
}

/// Lower-cased host of an `https://` URL, without user info and port.
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .split(':')
        .next()
        .unwrap_or_default();
    Some(host.to_ascii_lowercase())
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a bool",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn truncate(mut text: String, max: usize) -> String {
    if let Some((cut, _)) = text.char_indices().nth(max) {
        text.truncate(cut);
    }
    text
}

fn parse<T: for<'de> Deserialize<'de>>(input: &Value) -> Result<T, Value> {
    T::deserialize(input).map_err(|e| err("bad_request", e.to_string()))
}

fn reply(result: Result<Value, Value>) -> Value {
    match result {
        Ok(value) => ok(value),
        Err(reply) => reply,
    }
}

#[derive(Deserialize)]
struct KvGet {
    key: String,
}

#[derive(Deserialize)]
struct KvSet {
    key: String,
    #[serde(default)]
    value: Value,
}

#[derive(Deserialize, serde::Serialize)]
struct RowInput {
    key: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    cells: Map<String, Value>,
    #[serde(default)]
    rank: i32,
    #[serde(default)]
    ttl_s: Option<u32>,
}

#[derive(Deserialize)]
struct EmitRows {
    rows: Vec<RowInput>,
    #[serde(default)]
    replace: bool,
}

#[derive(Deserialize)]
struct SignalsInput {
    source: SignalSource,
}

#[derive(Deserialize)]
struct OpenMarkets {
    markets: Vec<Value>,
}

#[derive(Deserialize)]
struct Expire {
    keys: Vec<String>,
}

#[derive(Deserialize)]
struct AlertInput {
    level: String,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    row_key: Option<String>,
}

#[derive(Deserialize)]
struct StatusInput {
    text: String,
    #[serde(default)]
    tone: Option<String>,
}

#[derive(Deserialize)]
struct LogInput {
    level: String,
    msg: String,
}

pub(crate) type Shared = UserData<State>;

/// Every ABI v1 host function, bound to one state; the module links the ones it imports.
pub(crate) fn functions(shared: &Shared) -> Vec<Function> {
    HOST_FUNCTIONS
        .iter()
        .map(|&name| {
            Function::new(
                name,
                [PTR],
                [PTR],
                shared.clone(),
                move |plugin, inputs, outputs, data| dispatch(plugin, inputs, outputs, data, name),
            )
        })
        .collect()
}

fn dispatch(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    data: Shared,
    name: &'static str,
) -> Result<(), extism::Error> {
    let state = data.get()?;
    let mut state = state
        .lock()
        .map_err(|_| extism::Error::msg("harness state is poisoned"))?;
    if state.meter.over_budget() {
        state.meter.cut = true;
        return Err(extism::Error::msg("screener call exceeded its CPU budget"));
    }
    let bytes: Vec<u8> = match inputs
        .first()
        .map(|val| plugin.memory_get_val::<&[u8]>(val))
    {
        Some(Ok(bytes)) => bytes.to_vec(),
        _ => Vec::new(),
    };
    let reply = if bytes.is_empty() {
        state.call(name, &Value::Null)
    } else {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(input) => state.call(name, &input),
            Err(e) => err("bad_request", e.to_string()),
        }
    };
    drop(state);
    let reply = serde_json::to_vec(&reply).unwrap_or_else(|_| {
        br#"{"err":{"code":"transport","message":"reply encoding failed"}}"#.to_vec()
    });
    let handle = plugin.memory_new(reply)?;
    if let Some(output) = outputs.first_mut() {
        *output = plugin.memory_to_val(handle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Offline;
    use serde_json::json;

    fn state() -> State {
        let manifest = Manifest::parse(
            "abi: 1\nid: test.state\nversion: 0.1.0\nname: {en: S}\nlang: rust\nmin_terminal: 0.1.0\n\
             http: [fapi.binance.com]\ncolumns: [{key: oi, type: usd}, {key: name, type: text}]\n",
        )
        .unwrap();
        State::new(Arc::new(manifest), Box::new(Offline))
    }

    fn code(reply: &Value) -> Option<&str> {
        reply.pointer("/err/code").and_then(Value::as_str)
    }

    #[test]
    fn kv_starts_from_the_seed_and_keeps_its_cap() {
        let mut s = state();
        s.seed_kv(BTreeMap::from([("seen".to_string(), json!(["BTC"]))]));
        assert_eq!(
            s.call("kv_get", &json!({"key": "seen"})),
            json!({"ok": ["BTC"]})
        );
        let big = "x".repeat(KV_LIMIT_BYTES);
        assert_eq!(
            code(&s.call("kv_set", &json!({"key": "big", "value": big}))),
            Some("too_large")
        );
        assert_eq!(
            s.call("kv_set", &json!({"key": "seen", "value": null})),
            json!({"ok": null})
        );
        assert_eq!(
            s.call("kv_get", &json!({"key": "seen"})),
            json!({"ok": null})
        );
    }

    #[test]
    fn http_outside_the_manifest_is_refused_and_reported() {
        let mut s = state();
        let reply = s.call(
            "http",
            &json!({"method": "GET", "url": "https://evil.example.com/x"}),
        );
        assert_eq!(code(&reply), Some("host_not_allowed"));
        assert_eq!(s.stats.undeclared_hosts, ["evil.example.com"]);
        let offline = s.call(
            "http",
            &json!({"method": "GET", "url": "https://FAPI.binance.com/fapi/v1/ping"}),
        );
        assert_eq!(code(&offline), Some("transport"));
        assert_eq!(
            code(&s.call("http", &json!({"url": "http://fapi.binance.com"}))),
            Some("bad_request")
        );
        let batch = s.call("http_batch", &json!({"requests": [{"url": "https://fapi.binance.com/a"}, {"url": "https://x.io/b"}]}));
        assert_eq!(batch["ok"].as_array().map(Vec::len), Some(2));
        assert!(s.stats.undeclared_hosts.contains(&"x.io".to_string()));
    }

    #[test]
    fn rows_are_checked_against_the_manifest() {
        let mut s = state();
        let long = "k".repeat(MAX_ROW_KEY + 1);
        let reply = s.call(
            "emit_rows",
            &json!({"rows": [
                {"key": "A", "exchange": " Binance ", "cells": {"oi": {"v": "12", "tone": "pos"}, "name": 5}},
                {"key": long, "cells": {}},
            ]}),
        );
        assert_eq!(reply, json!({"ok": null}));
        let out = s.output();
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].exchange.as_deref(), Some("binance"));
        let warnings = out.warnings.join("\n");
        assert!(
            warnings.contains("column `oi` is usd but got text"),
            "{warnings}"
        );
        assert!(
            warnings.contains("column `name` is text but got a number"),
            "{warnings}"
        );
        assert!(warnings.contains("rows dropped"), "{warnings}");
        let bad_market = s.call(
            "emit_rows",
            &json!({"rows": [{"key": "B", "market": "margin"}]}),
        );
        assert_eq!(code(&bad_market), Some("bad_request"));
        s.call("expire", &json!({"keys": ["A"]}));
        assert!(s.output().rows.is_empty());
    }

    #[test]
    fn alerts_are_rate_limited_on_the_virtual_clock() {
        let mut s = state();
        for _ in 0..8 {
            s.call("emit_alert", &json!({"level": "info", "title": "t"}));
        }
        assert_eq!(s.output().alerts.len(), ALERTS_PER_MINUTE);
        s.now_ms = 61_000;
        s.call("emit_alert", &json!({"level": "warn", "title": "later"}));
        assert_eq!(s.output().alerts.len(), ALERTS_PER_MINUTE + 1);
    }

    #[test]
    fn open_market_works_once_and_only_in_a_click() {
        let mut s = state();
        let target = json!({"exchange": "binance", "market": "futures", "symbol": "BTCUSDT"});
        assert_eq!(code(&s.call("open_market", &target)), Some("not_in_click"));
        s.click = Some(false);
        assert_eq!(s.call("open_market", &target), json!({"ok": null}));
        assert_eq!(
            code(&s.call("open_spread", &json!({"a": target, "b": target}))),
            Some("not_in_click")
        );
        assert_eq!(s.output().intents.len(), 1);
    }

    #[test]
    fn huge_emits_stay_fast_and_keep_the_newest_rows() {
        let mut s = state();
        let started = std::time::Instant::now();
        let rows: Vec<Value> = (0..120_000)
            .map(|i| json!({"key": format!("k{i}"), "cells": {"oi": i}}))
            .collect();
        assert_eq!(
            s.call("emit_rows", &json!({ "rows": rows })),
            json!({"ok": null})
        );
        let out = s.output();
        assert_eq!(out.rows.len(), MAX_ROWS);
        assert!(out.rows.iter().any(|row| row.key == "k119999"));
        assert!(!out.rows.iter().any(|row| row.key == "k0"));
        for batch in 0..30 {
            let rows: Vec<Value> = (0..1000)
                .map(|i| json!({"key": format!("b{batch}-{i}"), "cells": {"oi": i}}))
                .collect();
            s.call("emit_rows", &json!({ "rows": rows }));
        }
        let out = s.output();
        assert_eq!(out.rows.len(), MAX_ROWS);
        assert!(out.rows.iter().all(|row| row.key.starts_with('b')));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn log_lines_are_cut_at_a_char_boundary() {
        let mut s = state();
        let long = "ж".repeat(MAX_LOG_LINE);
        s.call("log", &json!({"level": "info", "msg": long}));
        let line = &s.output().logs[0].msg;
        assert!(line.len() <= MAX_LOG_LINE && line.len() > MAX_LOG_LINE - 4);
        assert!(line.chars().all(|c| c == 'ж'));
    }

    #[test]
    fn signals_need_the_manifest_and_start_empty_offline() {
        let mut s = state();
        assert_eq!(
            code(&s.call("signals", &json!({"source": "density"}))),
            Some("not_permitted")
        );
        assert_eq!(
            code(&s.call("signals", &json!({"source": "trades"}))),
            Some("bad_request")
        );
        let manifest = Manifest::parse(
            "abi: 1\nid: test.dens\nversion: 0.1.0\nname: {en: D}\nlang: rust\nmin_terminal: 0.104.72\n\
             signals: [density]\ncolumns: [{key: d, type: usd}]\n",
        )
        .unwrap();
        let mut s = State::new(Arc::new(manifest), Box::new(Offline));
        assert_eq!(
            s.call("signals", &json!({"source": "density", "since": 4})),
            json!({"ok": {"seq": 0, "reset": true, "connected": false, "upserts": [], "removed": []}})
        );
        assert_eq!(s.stats.misses, 0);
    }

    #[test]
    fn open_markets_takes_one_to_sixteen_markets_in_a_click() {
        let mut s = state();
        let market = json!({"exchange": "binance", "market": "futures", "symbol": "BTCUSDT"});
        s.click = Some(false);
        assert_eq!(
            code(&s.call("open_markets", &json!({"markets": []}))),
            Some("bad_request")
        );
        let many: Vec<Value> = (0..=MAX_OPEN_MARKETS).map(|_| market.clone()).collect();
        assert_eq!(
            code(&s.call("open_markets", &json!({ "markets": many }))),
            Some("bad_request")
        );
        assert_eq!(
            s.call("open_markets", &json!({"markets": [market, market]})),
            json!({"ok": null})
        );
        assert_eq!(
            code(&s.call("open_market", &market)),
            Some("not_in_click"),
            "one open per click"
        );
        assert_eq!(s.output().intents[0].kind, "open_markets");
    }

    #[test]
    fn history_needs_the_manifest_permission() {
        let mut s = state();
        assert_eq!(
            code(&s.call("history_cluster", &json!({}))),
            Some("not_permitted")
        );
    }
}
