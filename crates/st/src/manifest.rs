use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_yaml::Value;

pub const FILE: &str = "manifest.yaml";

const COLUMN_TYPES: &[&str] = &[
    "text",
    "number",
    "integer",
    "percent",
    "usd",
    "price",
    "time",
    "duration",
    "countdown",
    "symbol",
    "exchange",
    "exchanges",
    "bool",
];
const PARAM_TYPES: &[&str] = &["number", "integer", "bool", "text", "select"];
const HISTORY_KINDS: &[&str] = &["cluster", "replay"];
const LANGS: &[&str] = &["rust", "ts"];
const MIN_TIMER_MS: u64 = 250;
const MAX_TIMER_MS: u64 = 3_600_000;
const MAX_MEMORY_MB: u32 = 64;
const MAX_TEXT_PARAM: usize = 256;
const MAX_CPU_MS_PER_CALL: u32 = 1000;

#[derive(Debug, Default, Deserialize)]
pub struct Localized {
    #[serde(default)]
    pub ru: Option<String>,
    #[serde(default)]
    pub en: Option<String>,
}

impl Localized {
    fn is_blank(&self) -> bool {
        [&self.ru, &self.en]
            .into_iter()
            .all(|s| s.as_deref().is_none_or(|s| s.trim().is_empty()))
    }

    pub fn any(&self) -> &str {
        self.en
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.ru.as_deref())
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
pub struct Column {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: Localized,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub width: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Param {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: Localized,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub cpu_ms_per_call: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub abi: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub name: Localized,
    /// Parsed only so that shapes the terminal rejects fail here too.
    #[serde(default, rename = "description")]
    _description: Option<Localized>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub min_terminal: Option<String>,
    #[serde(default)]
    pub http: Vec<String>,
    #[serde(default)]
    pub history: Vec<String>,
    #[serde(default)]
    pub timer_ms: Option<u64>,
    #[serde(default)]
    pub feeds: Vec<Value>,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub limits: Option<Limits>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    fn error(&mut self, code: &str, msg: impl AsRef<str>) {
        self.errors.push(format!("{code}: {}", msg.as_ref()));
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

pub fn read(dir: &Path) -> Result<(String, Manifest)> {
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let manifest = parse(&text)?;
    Ok((text, manifest))
}

pub fn parse(text: &str) -> Result<Manifest> {
    serde_yaml::from_str(text).context("invalid_manifest: manifest.yaml does not parse")
}

pub const ID_RULE: &str = "^[a-z0-9][a-z0-9._-]{1,62}[a-z0-9]$, not `install`, no segment between dots \
     may be a Windows device name (con, prn, aux, nul, com1..com9, lpt1..lpt9)";

// The id is a folder name on every OS, so Windows device names are refused in any dot segment.
fn is_windows_device(segment: &str) -> bool {
    matches!(segment, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            segment
                .strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
        })
}

pub fn is_valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let edge = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (3..=64).contains(&bytes.len())
        && bytes.first().is_some_and(edge)
        && bytes.last().is_some_and(edge)
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
        && !id.split('.').any(is_windows_device)
        // `/api/v1/screeners/install` is the install route, not a screener.
        && id != "install"
}

/// Mirrors the terminal: the host is trimmed and lower-cased before the check; a numeric TLD,
/// an IP literal and localhost are refused.
fn is_valid_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let labels_ok = host.split('.').all(|label| {
        !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    let numeric_tld = host
        .rsplit('.')
        .next()
        .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit()));
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && labels_ok
        && !numeric_tld
        && host.parse::<std::net::IpAddr>().is_err()
        && host != "localhost"
        && !host.ends_with(".localhost")
}

pub fn validate(m: &Manifest) -> Report {
    let mut r = Report::default();
    if m.abi != 1 {
        r.error("invalid_manifest", format!("abi must be 1, got {}", m.abi));
    }
    if !is_valid_id(&m.id) {
        r.error(
            "invalid_manifest",
            format!("id `{}` must match {ID_RULE}", m.id),
        );
    }
    if semver::Version::parse(&m.version).is_err() {
        r.error(
            "invalid_manifest",
            format!("version `{}` is not semver", m.version),
        );
    }
    if m.name.is_blank() {
        r.error("invalid_manifest", "name needs `ru` or `en`");
    } else if m.name.ru.is_none() || m.name.en.is_none() {
        r.warn("name has only one language; the terminal falls back to it");
    }
    match m.lang.as_deref() {
        Some(lang) if LANGS.contains(&lang) => {}
        Some(lang) => r.error(
            "invalid_manifest",
            format!("lang `{lang}` must be rust or ts"),
        ),
        None => r.error("invalid_manifest", "lang is required (rust or ts)"),
    }
    match m.min_terminal.as_deref() {
        Some(v) if semver::Version::parse(v).is_ok() => {}
        Some(v) => r.error(
            "invalid_manifest",
            format!("min_terminal `{v}` is not semver"),
        ),
        None => r.error("invalid_manifest", "min_terminal is required"),
    }
    for host in &m.http {
        if !is_valid_host(host) {
            r.error(
                "invalid_manifest",
                format!(
                    "http host `{host}` must be a bare host name like fapi.binance.com (no scheme, port, IP address or localhost)"
                ),
            );
        }
    }
    for kind in &m.history {
        if !HISTORY_KINDS.contains(&kind.as_str()) {
            r.error(
                "invalid_manifest",
                format!("history `{kind}` must be cluster or replay"),
            );
        }
    }
    if let Some(ms) = m.timer_ms
        && !(MIN_TIMER_MS..=MAX_TIMER_MS).contains(&ms)
    {
        r.error(
            "invalid_manifest",
            format!("timer_ms must be within {MIN_TIMER_MS}..={MAX_TIMER_MS}"),
        );
    }
    if !m.feeds.is_empty() {
        r.error(
            "unsupported_feed",
            "feeds are not available in ABI v1; poll with http/tickers in on_timer",
        );
    }
    validate_columns(&m.columns, &mut r);
    validate_params(&m.params, &mut r);
    if let Some(limits) = &m.limits {
        if limits
            .memory_mb
            .is_some_and(|mb| mb == 0 || mb > MAX_MEMORY_MB)
        {
            r.error(
                "invalid_manifest",
                format!("limits.memory_mb must be 1..={MAX_MEMORY_MB}"),
            );
        }
        if limits
            .cpu_ms_per_call
            .is_some_and(|ms| ms == 0 || ms > MAX_CPU_MS_PER_CALL)
        {
            r.error(
                "invalid_manifest",
                format!("limits.cpu_ms_per_call must be 1..={MAX_CPU_MS_PER_CALL}"),
            );
        }
    }
    r
}

fn validate_columns(columns: &[Column], r: &mut Report) {
    if columns.is_empty() {
        r.error("invalid_manifest", "columns must list at least one column");
    }
    let mut seen = HashSet::new();
    for c in columns {
        if c.key.is_empty() {
            r.error("invalid_manifest", "columns: empty key");
        } else if !seen.insert(c.key.as_str()) {
            r.error(
                "invalid_manifest",
                format!("column `{}` is declared twice", c.key),
            );
        }
        if !COLUMN_TYPES.contains(&c.kind.as_str()) {
            r.error(
                "invalid_manifest",
                format!("column `{}` has unknown type `{}`", c.key, c.kind),
            );
        }
        if let Some(sort) = c.sort.as_deref()
            && sort != "asc"
            && sort != "desc"
        {
            r.error(
                "invalid_manifest",
                format!("column `{}` sort must be asc or desc", c.key),
            );
        }
        if c.width.is_some_and(|w| w <= 0.0) {
            r.error(
                "invalid_manifest",
                format!("column `{}` width must be positive", c.key),
            );
        }
        if c.title.is_blank() {
            r.warn(format!("column `{}` has no title; the key is shown", c.key));
        }
    }
}

fn validate_params(params: &[Param], r: &mut Report) {
    let mut seen = HashSet::new();
    for p in params {
        if p.key.is_empty() {
            r.error("invalid_manifest", "params: empty key");
        } else if !seen.insert(p.key.as_str()) {
            r.error(
                "invalid_manifest",
                format!("param `{}` is declared twice", p.key),
            );
        }
        if !PARAM_TYPES.contains(&p.kind.as_str()) {
            r.error(
                "invalid_manifest",
                format!("param `{}` has unknown type `{}`", p.key, p.kind),
            );
            continue;
        }
        if let (Some(min), Some(max)) = (p.min, p.max)
            && min > max
        {
            r.error(
                "invalid_manifest",
                format!("param `{}` has min > max", p.key),
            );
        }
        let Some(default) = &p.default else {
            r.error(
                "invalid_manifest",
                format!("param `{}` needs a default", p.key),
            );
            continue;
        };
        let fits = match p.kind.as_str() {
            "number" => default.as_f64().is_some(),
            "integer" => default.as_f64().is_some_and(|v| v.fract() == 0.0),
            "bool" => default.as_bool().is_some(),
            "text" => default
                .as_str()
                .is_some_and(|t| t.chars().count() <= MAX_TEXT_PARAM),
            "select" => default
                .as_str()
                .is_some_and(|d| p.options.iter().any(|o| o == d)),
            _ => false,
        };
        if !fits {
            r.error(
                "invalid_manifest",
                format!("param `{}` default does not fit type `{}`", p.key, p.kind),
            );
        } else if let Some(v) = default.as_f64()
            && (p.min.is_some_and(|min| v < min) || p.max.is_some_and(|max| v > max))
        {
            r.error(
                "invalid_manifest",
                format!("param `{}` default {v} is outside [min, max]", p.key),
            );
        }
        if p.title.is_blank() {
            r.warn(format!("param `{}` has no title; the key is shown", p.key));
        }
        if p.kind == "select" && p.options.is_empty() {
            r.error(
                "invalid_manifest",
                format!("param `{}` select needs options", p.key),
            );
        }
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

    #[test]
    fn good_manifest_passes_and_ignores_unknown_fields() {
        let report = validate(&parse(GOOD).unwrap());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
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
        let report = validate(&parse(text).unwrap());
        let all = report.errors.join("\n");
        for needle in [
            "abi must be 1",
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
            "default does not fit",
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
            let text = base.replace(from, to);
            let valid = parse(&text).is_ok_and(|m| validate(&m).errors.is_empty());
            assert_eq!(valid, ok, "{to}");
        }
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
        assert!(is_valid_id("ivan.install"));
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
        assert!(is_valid_host("fapi.binance.com"));
        assert!(is_valid_host("api-1.example.io"));
        assert!(is_valid_host("api.1inch.io"));
        for bad in [
            "127.0.0.1",
            "10.1",
            "api.123",
            "localhost",
            "app.localhost",
            "[::1]",
            "fapi.binance.com:443",
        ] {
            assert!(!is_valid_host(bad), "{bad}");
        }
    }
}
