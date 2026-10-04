use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::params::{Param, ParamError, ParamProblem, ParamType};
use crate::report::{Code, Report};

pub const FILE: &str = "manifest.yaml";
pub const ABI_VERSION: u32 = 1;

pub const MIN_TIMER_MS: u64 = 250;
pub const DEFAULT_TIMER_MS: u64 = 1000;
pub const MAX_TIMER_MS: u64 = 3_600_000;
pub const MAX_MEMORY_MB: u32 = 64;
pub const MIN_CPU_MS: u64 = 50;
pub const DEFAULT_CPU_MS: u64 = 250;
pub const MAX_CPU_MS: u64 = 1000;
pub const MAX_COLUMN_WIDTH: f32 = 2000.0;

/// First terminal with ABI v1.1: `signals` in the manifest, host functions `signals` and
/// `open_markets`, `smart_level` of `open_market`.
pub const ABI_1_1_TERMINAL: semver::Version = semver::Version::new(0, 104, 72);

/// Ids that collide with static routes of the terminal's local API
/// (`/api/v1/screeners/install`, `/api/v1/screeners/sync`).
pub const RESERVED_IDS: &[&str] = &["install", "sync"];

pub const ID_RULE: &str = "^[a-z0-9][a-z0-9._-]{1,62}[a-z0-9]$, not `install` or `sync`, and no \
     segment between dots may be a Windows device name (con, prn, aux, nul, com1..com9, lpt1..lpt9)";

/// Folder names Windows refuses to create (`con`, `com1.x`, …); the id is a folder name on every OS.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Interface language of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ru,
    #[default]
    En,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    #[serde(default)]
    pub ru: String,
    #[serde(default)]
    pub en: String,
}

impl Localized {
    /// The text in `lang`, falling back to the other language.
    pub fn get(&self, lang: Lang) -> &str {
        let (first, second) = match lang {
            Lang::Ru => (&self.ru, &self.en),
            Lang::En => (&self.en, &self.ru),
        };
        if first.is_empty() { second } else { first }
    }

    pub fn is_blank(&self) -> bool {
        self.ru.trim().is_empty() && self.en.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginLang {
    Rust,
    Ts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryKind {
    Cluster,
    Replay,
}

/// Signal feed of the Space screener aggregator the terminal shares with plugins (ABI v1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalSource {
    Activity,
    Density,
    Prints,
}

impl SignalSource {
    pub const ALL: &[Self] = &[Self::Activity, Self::Density, Self::Prints];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Density => "density",
            Self::Prints => "prints",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|source| source.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    Text,
    Number,
    Integer,
    Percent,
    Usd,
    Price,
    Time,
    Duration,
    Countdown,
    Symbol,
    Exchange,
    Exchanges,
    Bool,
}

impl ColumnType {
    pub const ALL: &[Self] = &[
        Self::Text,
        Self::Number,
        Self::Integer,
        Self::Percent,
        Self::Usd,
        Self::Price,
        Self::Time,
        Self::Duration,
        Self::Countdown,
        Self::Symbol,
        Self::Exchange,
        Self::Exchanges,
        Self::Bool,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Percent => "percent",
            Self::Usd => "usd",
            Self::Price => "price",
            Self::Time => "time",
            Self::Duration => "duration",
            Self::Countdown => "countdown",
            Self::Symbol => "symbol",
            Self::Exchange => "exchange",
            Self::Exchanges => "exchanges",
            Self::Bool => "bool",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: ColumnType,
    #[serde(default)]
    pub title: Localized,
    #[serde(default)]
    pub sort: Option<SortDir>,
    #[serde(default)]
    pub width: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub memory_mb: u32,
    pub cpu_per_call: Duration,
}

/// A checked `manifest.yaml`. Registry-only fields (`categories`, `source`) are read by
/// [`crate::registry::check`]; the terminal ignores them.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub version: semver::Version,
    pub name: Localized,
    pub description: Option<Localized>,
    pub lang: PluginLang,
    pub min_terminal: semver::Version,
    /// Trimmed and lower-cased.
    pub http: Vec<String>,
    pub history: Vec<HistoryKind>,
    pub signals: Vec<SignalSource>,
    pub timer: Duration,
    pub columns: Vec<Column>,
    pub params: Vec<Param>,
    pub limits: Limits,
}

/// The plugin needs a newer terminal than the one asking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("screener requires terminal {required}, this is {current}")]
pub struct TerminalTooOld {
    pub required: semver::Version,
    pub current: semver::Version,
}

impl Manifest {
    /// Parses and checks every rule; on failure the report lists all problems.
    pub fn parse(yaml: &str) -> Result<Self, Report> {
        let (manifest, report) = Self::check(yaml);
        manifest.filter(|_| report.is_ok()).ok_or(report)
    }

    /// Like [`Manifest::parse`], but also returns the warnings (one-language name, column
    /// without a title) of a manifest that passed.
    pub fn check(yaml: &str) -> (Option<Self>, Report) {
        check(yaml, None)
    }

    /// The terminal's entry point: `terminal_too_old` comes first when the plugin needs a newer
    /// terminal, since a newer terminal may accept what this one reports as broken.
    pub fn parse_for_terminal(yaml: &str, terminal: &semver::Version) -> Result<Self, Report> {
        let (manifest, report) = check(yaml, Some(terminal));
        manifest.filter(|_| report.is_ok()).ok_or(report)
    }

    pub fn check_terminal(&self, terminal: &semver::Version) -> Result<(), TerminalTooOld> {
        if self.min_terminal > *terminal {
            Err(TerminalTooOld {
                required: self.min_terminal.clone(),
                current: terminal.clone(),
            })
        } else {
            Ok(())
        }
    }

    /// The first column with `sort` is the pane's default order.
    pub fn default_sort(&self) -> Option<(&Column, SortDir)> {
        self.columns
            .iter()
            .find_map(|column| column.sort.map(|dir| (column, dir)))
    }

    pub fn default_params(&self) -> BTreeMap<String, Value> {
        self.params
            .iter()
            .map(|param| (param.key.clone(), param.default.clone()))
            .collect()
    }

    /// Manifest defaults over saved values; unknown keys and values that no longer fit
    /// (the manifest changed) are dropped.
    pub fn merge_params(&self, saved: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
        self.params
            .iter()
            .map(|param| {
                let value = saved
                    .get(&param.key)
                    .and_then(|value| param.validate(value).ok())
                    .unwrap_or_else(|| param.default.clone());
                (param.key.clone(), value)
            })
            .collect()
    }

    /// Checks user values over the defaults: every key must be declared in the manifest.
    pub fn validate_params(
        &self,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Vec<ParamError>> {
        let mut merged = self.default_params();
        let mut problems = Vec::new();
        for (key, value) in values {
            match self.params.iter().find(|param| &param.key == key) {
                Some(param) => match param.validate(value) {
                    Ok(value) => {
                        merged.insert(key.clone(), value);
                    }
                    Err(problem) => problems.push(problem),
                },
                None => problems.push(ParamError {
                    key: key.clone(),
                    problem: ParamProblem::Unknown,
                }),
            }
        }
        if problems.is_empty() {
            Ok(merged)
        } else {
            Err(problems)
        }
    }

    pub fn allows_history(&self, kind: HistoryKind) -> bool {
        self.history.contains(&kind)
    }

    pub fn allows_signals(&self, source: SignalSource) -> bool {
        self.signals.contains(&source)
    }
}

/// `^[a-z0-9][a-z0-9._-]{1,62}[a-z0-9]$`, no Windows device name in any dot segment and no
/// local API route name.
pub fn is_valid_id(id: &str) -> bool {
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let inner = |c: char| edge(c) || matches!(c, '.' | '_' | '-');
    (3..=64).contains(&id.len())
        && id.chars().all(inner)
        && id.chars().next().is_some_and(edge)
        && id.chars().last().is_some_and(edge)
        && !RESERVED_IDS.contains(&id)
        && !id
            .split('.')
            .any(|segment| WINDOWS_RESERVED.contains(&segment))
}

/// The allowlist form of an `http` entry: trimmed, lower-cased, a public name — not an IP
/// literal, not localhost, no scheme or port.
pub fn normalize_host(host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    let labels_ok = host.split('.').all(|label| {
        !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    let numeric_tld = host
        .rsplit('.')
        .next()
        .is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit()));
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && labels_ok
        && !numeric_tld
        && host.parse::<std::net::IpAddr>().is_err()
        && host != "localhost"
        && !host.ends_with(".localhost");
    valid.then_some(host)
}

#[derive(Debug, Default, Deserialize)]
struct RawLocalized {
    #[serde(default)]
    ru: Option<String>,
    #[serde(default)]
    en: Option<String>,
}

impl RawLocalized {
    fn into_localized(self) -> Localized {
        Localized {
            ru: self.ru.unwrap_or_default(),
            en: self.en.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawColumn {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: Option<RawLocalized>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    width: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawParam {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: Option<RawLocalized>,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    min: Option<Decimal>,
    #[serde(default)]
    max: Option<Decimal>,
    #[serde(default)]
    options: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLimits {
    #[serde(default)]
    memory_mb: Option<u32>,
    #[serde(default)]
    cpu_ms_per_call: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawManifest {
    #[serde(default)]
    abi: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    name: Option<RawLocalized>,
    #[serde(default)]
    description: Option<RawLocalized>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    min_terminal: Option<String>,
    #[serde(default)]
    http: Vec<String>,
    #[serde(default)]
    history: Vec<String>,
    #[serde(default)]
    signals: Vec<String>,
    #[serde(default)]
    timer_ms: Option<u64>,
    #[serde(default)]
    feeds: Vec<serde_yaml::Value>,
    #[serde(default)]
    columns: Vec<RawColumn>,
    #[serde(default)]
    params: Vec<RawParam>,
    #[serde(default)]
    limits: RawLimits,
}

fn check(yaml: &str, terminal: Option<&semver::Version>) -> (Option<Manifest>, Report) {
    let mut r = Report::default();
    let raw: RawManifest = match serde_yaml::from_str(yaml) {
        Ok(raw) => raw,
        Err(e) => {
            r.error(
                Code::InvalidManifest,
                "",
                format!("{FILE} does not parse: {e}"),
            );
            return (None, r);
        }
    };

    match raw.abi {
        Some(ABI_VERSION) => {}
        Some(abi) => r.error(
            Code::InvalidManifest,
            "abi",
            format!("abi {abi} is not supported, the terminal speaks abi {ABI_VERSION}"),
        ),
        None => r.error(Code::InvalidManifest, "abi", "abi is required (1)"),
    }

    let id = raw.id.unwrap_or_default();
    if id.is_empty() {
        r.error(Code::InvalidManifest, "id", "id is required");
    } else if !is_valid_id(&id) {
        r.error(
            Code::InvalidManifest,
            "id",
            format!("id `{id}` must match {ID_RULE}"),
        );
    }

    let version = required_semver(&mut r, "version", raw.version);
    let min_terminal = required_semver(&mut r, "min_terminal", raw.min_terminal);
    if let (Some(terminal), Some(required)) = (terminal, &min_terminal)
        && required > terminal
    {
        r.errors.insert(
            0,
            crate::report::Issue {
                code: Code::TerminalTooOld,
                path: "min_terminal".to_string(),
                message: format!("screener requires terminal {required}, this is {terminal}"),
            },
        );
    }

    let name = raw.name.unwrap_or_default().into_localized();
    if name.is_blank() {
        r.error(Code::InvalidManifest, "name", "name needs `ru` or `en`");
    } else if name.ru.trim().is_empty() || name.en.trim().is_empty() {
        r.warn("name has only one language; the terminal falls back to it");
    }
    let description = raw.description.map(RawLocalized::into_localized);

    let lang = match raw.lang.as_deref() {
        Some("rust") => Some(PluginLang::Rust),
        Some("ts") => Some(PluginLang::Ts),
        Some(other) => {
            r.error(
                Code::InvalidManifest,
                "lang",
                format!("lang `{other}` must be rust or ts"),
            );
            None
        }
        None => {
            r.error(
                Code::InvalidManifest,
                "lang",
                "lang is required (rust or ts)",
            );
            None
        }
    };

    let mut http = Vec::with_capacity(raw.http.len());
    for (i, host) in raw.http.iter().enumerate() {
        match normalize_host(host) {
            Some(host) if http.contains(&host) => r.error(
                Code::InvalidManifest,
                format!("http[{i}]"),
                format!("`{host}` is declared twice"),
            ),
            Some(host) => http.push(host),
            None => r.error(
                Code::InvalidManifest,
                format!("http[{i}]"),
                format!(
                    "`{host}` must be a bare host name like fapi.binance.com (no scheme, port, IP address or localhost)"
                ),
            ),
        }
    }

    let mut history = Vec::with_capacity(raw.history.len());
    for (i, kind) in raw.history.iter().enumerate() {
        match kind.as_str() {
            "cluster" => history.push(HistoryKind::Cluster),
            "replay" => history.push(HistoryKind::Replay),
            other => r.error(
                Code::InvalidManifest,
                format!("history[{i}]"),
                format!("`{other}` must be cluster or replay"),
            ),
        }
    }

    let mut signals = Vec::with_capacity(raw.signals.len());
    for (i, source) in raw.signals.iter().enumerate() {
        match SignalSource::parse(source) {
            Some(source) if signals.contains(&source) => r.error(
                Code::InvalidManifest,
                format!("signals[{i}]"),
                format!("`{}` is declared twice", source.as_str()),
            ),
            Some(source) => signals.push(source),
            None => r.error(
                Code::InvalidManifest,
                format!("signals[{i}]"),
                format!("`{source}` must be activity, density or prints"),
            ),
        }
    }
    if !signals.is_empty()
        && let Some(required) = &min_terminal
        && *required < ABI_1_1_TERMINAL
    {
        r.error(
            Code::InvalidManifest,
            "min_terminal",
            format!("signals need min_terminal >= {ABI_1_1_TERMINAL}"),
        );
    }

    let timer_ms = raw.timer_ms.unwrap_or(DEFAULT_TIMER_MS);
    if !(MIN_TIMER_MS..=MAX_TIMER_MS).contains(&timer_ms) {
        r.error(
            Code::InvalidManifest,
            "timer_ms",
            format!("timer_ms must be within {MIN_TIMER_MS}..={MAX_TIMER_MS}"),
        );
    }

    if !raw.feeds.is_empty() {
        r.error(
            Code::UnsupportedFeed,
            "feeds",
            "feeds are not available in ABI v1; poll with http/tickers in on_timer",
        );
    }

    let columns = check_columns(raw.columns, &mut r);
    let params = check_params(raw.params, &mut r);

    let memory_mb = raw.limits.memory_mb.unwrap_or(MAX_MEMORY_MB);
    if !(1..=MAX_MEMORY_MB).contains(&memory_mb) {
        r.error(
            Code::InvalidManifest,
            "limits.memory_mb",
            format!("limits.memory_mb must be within 1..={MAX_MEMORY_MB}"),
        );
    }
    let cpu_ms = raw.limits.cpu_ms_per_call.unwrap_or(DEFAULT_CPU_MS);
    if !(MIN_CPU_MS..=MAX_CPU_MS).contains(&cpu_ms) {
        r.error(
            Code::InvalidManifest,
            "limits.cpu_ms_per_call",
            format!("limits.cpu_ms_per_call must be within {MIN_CPU_MS}..={MAX_CPU_MS}"),
        );
    }

    if terminal.is_some() {
        // The old host stopped at the first problem in this order: a newer terminal may accept
        // feeds and anything else this one reports.
        r.errors.sort_by_key(|issue| match issue.code {
            Code::TerminalTooOld => 0,
            Code::UnsupportedFeed => 1,
            _ => 2,
        });
    }
    if !r.is_ok() {
        return (None, r);
    }
    let (Some(version), Some(min_terminal), Some(lang)) = (version, min_terminal, lang) else {
        return (None, r);
    };
    let manifest = Manifest {
        id,
        version,
        name,
        description,
        lang,
        min_terminal,
        http,
        history,
        signals,
        timer: Duration::from_millis(timer_ms),
        columns,
        params,
        limits: Limits {
            memory_mb,
            cpu_per_call: Duration::from_millis(cpu_ms),
        },
    };
    (Some(manifest), r)
}

fn required_semver(r: &mut Report, field: &str, value: Option<String>) -> Option<semver::Version> {
    let Some(value) = value else {
        r.error(Code::InvalidManifest, field, format!("{field} is required"));
        return None;
    };
    match semver::Version::parse(&value) {
        Ok(version) => Some(version),
        Err(e) => {
            r.error(
                Code::InvalidManifest,
                field,
                format!("{field} `{value}` is not semver: {e}"),
            );
            None
        }
    }
}

fn check_columns(raw: Vec<RawColumn>, r: &mut Report) -> Vec<Column> {
    if raw.is_empty() {
        r.error(
            Code::InvalidManifest,
            "columns",
            "columns must list at least one column",
        );
    }
    let mut seen = HashSet::new();
    let mut columns = Vec::with_capacity(raw.len());
    for (i, c) in raw.into_iter().enumerate() {
        let path = format!("columns[{i}]");
        if c.key.is_empty() {
            r.error(Code::InvalidManifest, &path, "empty key");
        } else if !seen.insert(c.key.clone()) {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("column `{}` is declared twice", c.key),
            );
        }
        let kind = ColumnType::parse(&c.kind);
        if kind.is_none() {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("column `{}` has unknown type `{}`", c.key, c.kind),
            );
        }
        let sort = match c.sort.as_deref() {
            None => None,
            Some("asc") => Some(SortDir::Asc),
            Some("desc") => Some(SortDir::Desc),
            Some(_) => {
                r.error(
                    Code::InvalidManifest,
                    &path,
                    format!("column `{}` sort must be asc or desc", c.key),
                );
                None
            }
        };
        let width = c.width.map(|w| w as f32);
        if let Some(width) = width
            && !(width.is_finite() && width > 0.0 && width <= MAX_COLUMN_WIDTH)
        {
            r.error(
                Code::InvalidManifest,
                &path,
                format!(
                    "column `{}` width must be 1..={MAX_COLUMN_WIDTH} pixels",
                    c.key
                ),
            );
        }
        let title = c.title.unwrap_or_default().into_localized();
        if title.is_blank() {
            r.warn(format!("column `{}` has no title; the key is shown", c.key));
        }
        if let Some(kind) = kind {
            columns.push(Column {
                key: c.key,
                kind,
                title,
                sort,
                width,
            });
        }
    }
    columns
}

fn check_params(raw: Vec<RawParam>, r: &mut Report) -> Vec<Param> {
    let mut seen = HashSet::new();
    let mut params = Vec::with_capacity(raw.len());
    for (i, p) in raw.into_iter().enumerate() {
        let path = format!("params[{i}]");
        if p.key.is_empty() {
            r.error(Code::InvalidManifest, &path, "empty key");
        } else if !seen.insert(p.key.clone()) {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("param `{}` is declared twice", p.key),
            );
        }
        let Some(kind) = ParamType::parse(&p.kind) else {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("param `{}` has unknown type `{}`", p.key, p.kind),
            );
            continue;
        };
        if let (Some(min), Some(max)) = (p.min, p.max)
            && min > max
        {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("param `{}` has min > max", p.key),
            );
        }
        if kind == ParamType::Select && p.options.is_empty() {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("param `{}` select needs options", p.key),
            );
        }
        let title = p.title.unwrap_or_default().into_localized();
        if title.is_blank() {
            r.warn(format!("param `{}` has no title; the key is shown", p.key));
        }
        let Some(default) = p.default else {
            r.error(
                Code::InvalidManifest,
                &path,
                format!("param `{}` needs a default", p.key),
            );
            continue;
        };
        let mut param = Param {
            key: p.key,
            kind,
            title,
            default,
            min: p.min,
            max: p.max,
            options: p.options,
        };
        match param.validate(&param.default) {
            Ok(coerced) => param.default = coerced,
            Err(e) => r.error(
                Code::InvalidManifest,
                &path,
                format!("default of {e} (type `{}`)", p.kind),
            ),
        }
        params.push(param);
    }
    params
}

impl fmt::Display for PluginLang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rust => "rust",
            Self::Ts => "ts",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
abi: 1
id: local.oi-8
version: 0.1.0
name: {ru: "OI", en: "OI"}
lang: rust
min_terminal: 0.104.70
http: [fapi.binance.com]
timer_ms: 60000
columns:
  - {key: symbol, type: symbol, title: {en: Symbol}}
  - {key: oi, type: usd, title: {en: OI}, sort: desc}
params:
  - {key: min_oi, type: number, title: {en: Min OI}, default: 1000000, min: 0}
  - {key: side, type: select, title: {en: Side}, default: both, options: [both, long]}
pricing: {model: free}
"#;

    const TERMINAL: &str = "0.104.71";

    fn terminal() -> semver::Version {
        semver::Version::parse(TERMINAL).unwrap()
    }

    fn host_manifest(extra: &str) -> String {
        format!(
            "abi: 1\nid: ivan.oi-spike\nversion: 0.1.0\nname: {{ru: Всплеск OI, en: OI spike}}\n\
             lang: rust\nmin_terminal: 0.104.0\n\
             http: [fapi.binance.com]\ncolumns:\n  - {{key: symbol, type: symbol}}\n  \
             - {{key: oi, type: usd, sort: desc}}\nparams: [{{key: threshold, type: number, default: 5, min: 1, max: 50}}]\n{extra}"
        )
    }

    fn first_code(yaml: &str) -> &'static str {
        Manifest::parse_for_terminal(yaml, &terminal())
            .unwrap_err()
            .first()
            .unwrap()
            .code
            .as_str()
    }

    fn is_valid(yaml: &str) -> bool {
        Manifest::parse(yaml).is_ok()
    }

    #[test]
    fn good_manifest_passes_and_ignores_unknown_fields() {
        let manifest = Manifest::parse(GOOD).unwrap();
        assert_eq!(manifest.http, ["fapi.binance.com"]);
        assert_eq!(manifest.timer, Duration::from_secs(60));
        assert_eq!(
            manifest.default_sort().map(|(c, _)| c.key.as_str()),
            Some("oi")
        );
    }

    #[test]
    fn bad_manifest_reports_every_problem() {
        let text = r#"
abi: 2
id: Bad_ID
version: one
name: {}
feeds: [{kind: trades}]
http: ["https://fapi.binance.com"]
timer_ms: 10
columns:
  - {key: a, type: money}
  - {key: a, type: text, sort: up}
params:
  - {key: p, type: select, default: x, options: [y]}
limits: {memory_mb: 128}
"#;
        let report = Manifest::parse(text).unwrap_err();
        let all = report.to_string();
        for needle in [
            "abi 2 is not supported",
            "id `Bad_ID`",
            "not semver",
            "name needs",
            "lang is required",
            "min_terminal is required",
            "unsupported_feed",
            "bare host name",
            "timer_ms",
            "unknown type `money`",
            "declared twice",
            "sort must be",
            "not one of the options",
            "memory_mb",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    #[test]
    fn host_rules_match_the_terminal() {
        let base = GOOD.replace("timer_ms: 60000", "timer_ms: 3600000");
        let cases = [
            (
                "http: [fapi.binance.com]",
                "http: [\" FAPI.Binance.com \"]",
                true,
            ),
            (
                "http: [fapi.binance.com]",
                "http: [fapi..binance.com]",
                false,
            ),
            ("timer_ms: 3600000", "timer_ms: 3600001", false),
            ("default: 1000000, min: 0", "default: -1, min: 0", false),
            ("default: both", "default: none", false),
            (
                "pricing: {model: free}",
                "limits: {cpu_ms_per_call: 49}",
                false,
            ),
            (
                "pricing: {model: free}",
                "limits: {cpu_ms_per_call: 50}",
                true,
            ),
            (
                "pricing: {model: free}",
                "limits: {cpu_ms_per_call: 1001}",
                false,
            ),
            (
                "pricing: {model: free}",
                "description: {en: \"Open interest\"}",
                true,
            ),
            (
                "pricing: {model: free}",
                "description: \"just text\"",
                false,
            ),
            (
                "type: number, title: {en: Min OI}, default: 1000000",
                "type: integer, title: {en: Min OI}, default: 5.0",
                true,
            ),
            (
                "type: number, title: {en: Min OI}, default: 1000000",
                "type: integer, title: {en: Min OI}, default: 5.5",
                false,
            ),
        ];
        for (from, to, ok) in cases {
            assert_eq!(is_valid(&base.replace(from, to)), ok, "{to}");
        }
    }

    #[test]
    fn validates_manifest_and_reports_canonical_codes() {
        let ok =
            Manifest::parse_for_terminal(&host_manifest("timer_ms: 5000\n"), &terminal()).unwrap();
        assert_eq!(ok.timer, Duration::from_secs(5));
        assert_eq!(
            ok.limits.cpu_per_call,
            Duration::from_millis(DEFAULT_CPU_MS)
        );
        assert_eq!(ok.limits.memory_mb, MAX_MEMORY_MB);

        let code = |extra: &str| first_code(&host_manifest(extra));
        assert_eq!(
            code("feeds: [{kind: trades, exchange: binance}]\n"),
            "unsupported_feed"
        );
        assert_eq!(
            first_code(&host_manifest("").replace("0.104.0", "0.200.0")),
            "terminal_too_old"
        );
        assert_eq!(
            first_code(&host_manifest("").replace("lang: rust\n", "")),
            "invalid_manifest"
        );
        assert_eq!(code("timer_ms: 100\n"), "invalid_manifest");
        assert_eq!(code("limits: {memory_mb: 128}\n"), "invalid_manifest");
        assert_eq!(code("limits: {cpu_ms_per_call: 20}\n"), "invalid_manifest");
        for bad_id in [
            "Ivan/Bad", "ivan.oi-", "ivan.con", "nul", "ab", "install", "sync",
        ] {
            assert_eq!(
                first_code(&host_manifest("").replace("ivan.oi-spike", bad_id)),
                "invalid_manifest",
                "{bad_id}"
            );
        }
        for bad_host in [
            "127.0.0.1",
            "10.1",
            "localhost",
            "api.localhost",
            "https://fapi.binance.com",
        ] {
            assert_eq!(
                first_code(&host_manifest("").replace("fapi.binance.com", bad_host)),
                "invalid_manifest",
                "{bad_host}"
            );
        }
    }

    #[test]
    fn terminal_too_old_comes_first_even_with_other_problems() {
        let yaml = host_manifest("feeds: [{kind: trades}]\n").replace("0.104.0", "0.200.0");
        let report = Manifest::parse_for_terminal(&yaml, &terminal()).unwrap_err();
        assert_eq!(report.errors[0].code, Code::TerminalTooOld);
        assert_eq!(report.errors[1].code, Code::UnsupportedFeed);
        let manifest = Manifest::parse(&host_manifest("").replace("0.104.0", "0.200.0")).unwrap();
        let too_old = manifest.check_terminal(&terminal()).unwrap_err();
        assert_eq!(too_old.required.to_string(), "0.200.0");
    }

    #[test]
    fn blank_name_is_refused() {
        let yaml =
            host_manifest("").replace("{ru: Всплеск OI, en: OI spike}", "{ru: \"  \", en: \" \"}");
        assert!(!is_valid(&yaml));
        let one = host_manifest("").replace("{ru: Всплеск OI, en: OI spike}", "{en: OI spike}");
        let (manifest, report) = Manifest::check(&one);
        assert!(manifest.is_some());
        assert!(report.warnings.iter().any(|w| w.contains("one language")));
    }

    #[test]
    fn column_width_must_be_positive() {
        for (width, ok) in [
            ("0", false),
            ("-5", false),
            (".nan", false),
            ("1e-50", false),
            ("1e39", false),
            ("2001", false),
            ("120", true),
            ("2000", true),
        ] {
            let yaml = host_manifest("").replace(
                "{key: oi, type: usd, sort: desc}",
                &format!("{{key: oi, type: usd, sort: desc, width: {width}}}"),
            );
            assert_eq!(is_valid(&yaml), ok, "width {width}");
        }
    }

    #[test]
    fn unsupported_feed_comes_before_other_problems_for_the_terminal() {
        let yaml = host_manifest("feeds: [{kind: trades}]\ntimer_ms: 100\n");
        assert_eq!(first_code(&yaml), "unsupported_feed");
        let report = Manifest::parse(&yaml).unwrap_err();
        assert_eq!(
            report.errors[0].code,
            Code::InvalidManifest,
            "document order without a terminal"
        );
    }

    #[test]
    fn signals_list_known_sources_once_and_need_abi_1_1() {
        let yaml = |extra: &str| host_manifest(extra).replace("0.104.0", "0.104.72");
        let manifest = Manifest::parse(&yaml("signals: [density, prints]\n")).unwrap();
        assert_eq!(
            manifest.signals,
            [SignalSource::Density, SignalSource::Prints]
        );
        assert!(manifest.allows_signals(SignalSource::Density));
        assert!(!manifest.allows_signals(SignalSource::Activity));

        let report = Manifest::parse(&yaml("signals: [density, trades, density]\n")).unwrap_err();
        assert_eq!(report.errors[0].path, "signals[1]");
        assert!(
            report.errors[0]
                .message
                .contains("activity, density or prints")
        );
        assert_eq!(report.errors[1].path, "signals[2]");
        assert!(report.errors[1].message.contains("declared twice"));

        let old = Manifest::parse(&host_manifest("signals: [activity]\n")).unwrap_err();
        assert_eq!(old.errors[0].code, Code::InvalidManifest);
        assert_eq!(old.errors[0].path, "min_terminal");
        assert!(old.errors[0].message.contains("0.104.72"), "{old}");
    }

    #[test]
    fn http_hosts_are_unique_after_normalization() {
        let yaml = host_manifest("").replace(
            "http: [fapi.binance.com]",
            "http: [fapi.binance.com, \" FAPI.Binance.com \"]",
        );
        let report = Manifest::parse(&yaml).unwrap_err();
        assert_eq!(report.errors[0].path, "http[1]");
        assert!(report.errors[0].message.contains("declared twice"));
    }

    #[test]
    fn integer_default_is_coerced() {
        let yaml =
            host_manifest("").replace("type: number, default: 5", "type: integer, default: 5.0");
        let manifest = Manifest::parse(&yaml).unwrap();
        assert_eq!(manifest.params[0].default, serde_json::json!(5));
    }

    #[test]
    fn params_are_validated_and_merged_over_defaults() {
        let manifest = Manifest::parse(&host_manifest("")).unwrap();
        let threshold = &manifest.params[0];
        assert_eq!(
            threshold
                .validate(&serde_json::json!(99))
                .unwrap_err()
                .problem,
            ParamProblem::OutOfRange {
                min: threshold.min,
                max: threshold.max
            }
        );
        let rows = Param {
            key: "rows".to_string(),
            kind: ParamType::Integer,
            title: Localized::default(),
            default: serde_json::json!(1),
            min: None,
            max: None,
            options: Vec::new(),
        };
        assert_eq!(
            rows.validate(&serde_json::json!(5.0)),
            Ok(serde_json::json!(5))
        );
        let bad = BTreeMap::from([
            ("threshold".to_string(), serde_json::json!(99)),
            ("nope".to_string(), serde_json::json!(1)),
        ]);
        let problems = manifest.validate_params(&bad).unwrap_err();
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|p| p.problem == ParamProblem::Unknown));
        let saved = BTreeMap::from([
            ("threshold".to_string(), serde_json::json!(99)),
            ("gone".to_string(), serde_json::json!(1)),
        ]);
        assert_eq!(
            manifest.merge_params(&saved),
            BTreeMap::from([("threshold".to_string(), serde_json::json!(5))])
        );
    }

    #[test]
    fn id_rules() {
        assert!(is_valid_id("ivan.vol-spike"));
        assert!(is_valid_id("0ab"));
        assert!(is_valid_id("ivan.com10"));
        assert!(is_valid_id("ivan.console"));
        assert!(!is_valid_id("ab"));
        assert!(!is_valid_id("-ab"));
        assert!(!is_valid_id("ab-"));
        assert!(!is_valid_id("ivan.oi."));
        assert!(!is_valid_id("Ab.c"));
        assert!(!is_valid_id(&"a".repeat(65)));
        assert!(is_valid_id(&"a".repeat(64)));
        assert!(!is_valid_id("install"));
        assert!(!is_valid_id("sync"));
        assert!(is_valid_id("ivan.install"));
        assert!(is_valid_id("ivan.sync"));
        for reserved in [
            "con.oi",
            "ivan.nul",
            "ivan.com1.x",
            "lpt9.screener",
            "ivan.aux",
        ] {
            assert!(!is_valid_id(reserved), "{reserved}");
        }
    }

    #[test]
    fn http_hosts_refuse_ip_literals_and_localhost() {
        assert_eq!(
            normalize_host(" FAPI.binance.com ").as_deref(),
            Some("fapi.binance.com")
        );
        assert!(normalize_host("api-1.example.io").is_some());
        assert!(normalize_host("api.1inch.io").is_some());
        for bad in [
            "127.0.0.1",
            "10.1",
            "api.123",
            "localhost",
            "app.localhost",
            "[::1]",
            "fapi.binance.com:443",
        ] {
            assert!(normalize_host(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn broken_yaml_is_one_invalid_manifest() {
        let report = Manifest::parse("abi: [").unwrap_err();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].code, Code::InvalidManifest);
    }
}
