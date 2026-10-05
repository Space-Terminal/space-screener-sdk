use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Market {
    Spot,
    Futures,
}

impl fmt::Display for Market {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Spot => "spot",
            Self::Futures => "futures",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarketRef {
    pub exchange: String,
    pub market: Market,
    pub symbol: String,
}

impl MarketRef {
    pub fn new(exchange: impl Into<String>, market: Market, symbol: impl Into<String>) -> Self {
        Self {
            exchange: exchange.into(),
            market,
            symbol: symbol.into(),
        }
    }
}

/// A smart level [`crate::host::open_market_with_level`] places at a density signal's price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartLevel {
    pub signal_id: String,
    /// Sound when the level is touched or eaten.
    #[serde(default)]
    pub sound: bool,
}

impl SmartLevel {
    pub fn new(signal_id: impl Into<String>) -> Self {
        Self {
            signal_id: signal_id.into(),
            sound: false,
        }
    }

    pub fn with_sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExchangeMarket {
    pub exchange: String,
    pub market: Market,
}

impl ExchangeMarket {
    pub fn new(exchange: impl Into<String>, market: Market) -> Self {
        Self {
            exchange: exchange.into(),
            market,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeInfo {
    pub exchange: String,
    pub market: Market,
    #[serde(default)]
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticker {
    pub symbol: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub last: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub change_pct: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub volume_quote: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickerSnapshot {
    #[serde(default)]
    pub ts_ms: i64,
    #[serde(default)]
    pub tickers: Vec<Ticker>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    #[serde(default)]
    pub base: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default)]
    pub trading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HttpRequest {
    #[serde(default)]
    pub method: Method,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }

    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            body: Some(body.into()),
            ..Self::default()
        }
        .header("content-type", "application/json")
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = Some(ms);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn error_for_status(self) -> crate::Result<Self> {
        if self.is_success() {
            return Ok(self);
        }
        let body = self.body.chars().take(512).collect();
        Err(crate::Error::Status {
            status: self.status,
            body,
        })
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> crate::Result<T> {
        serde_json::from_str(&self.body).map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterRequest {
    pub exchange: String,
    pub symbol: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub tf_s: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cells: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterCell {
    pub t_ms: i64,
    #[serde(deserialize_with = "de::f64_flex")]
    pub price: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub bid_vol: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub ask_vol: f64,
    #[serde(default)]
    pub trades: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterHistory {
    #[serde(default)]
    pub cells: Vec<ClusterCell>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRequest {
    pub exchange: String,
    pub symbol: String,
    pub from_ms: i64,
    pub to_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_trades: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertLevel {
    Info,
    Warn,
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusTone {
    Neutral,
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpreadLayout {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ru,
    #[default]
    En,
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(if s.eq_ignore_ascii_case("ru") {
            Self::Ru
        } else {
            Self::En
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitInput {
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub terminal: String,
    #[serde(default)]
    pub lang: Lang,
    #[serde(default)]
    pub now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimerInput {
    #[serde(default)]
    pub now_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParamsInput {
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct Modifiers {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub logo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ClickRow {
    pub key: String,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub exchange: Option<String>,
    #[serde(default)]
    pub market: Option<Market>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Click {
    pub row: ClickRow,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub button: MouseButton,
    #[serde(default)]
    pub modifiers: Modifiers,
}

impl Click {
    pub fn market_ref(&self) -> Option<MarketRef> {
        let symbol = self.row.symbol.clone()?;
        let exchange = self.row.exchange.clone()?;
        Some(MarketRef {
            exchange,
            market: self.row.market.unwrap_or(Market::Futures),
            symbol,
        })
    }
}

pub(crate) mod de {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub fn f64_flex<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Number(n) => n.as_f64().unwrap_or_default(),
            Value::String(s) => s.trim().parse().unwrap_or_default(),
            _ => 0.0,
        })
    }
}
