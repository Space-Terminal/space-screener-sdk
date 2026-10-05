//! Signals of the Space screener aggregator the terminal shares with plugins (ABI v1.1).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::abi::{ExchangeMarket, Market, MarketRef, de};
use crate::errors::Result;
use crate::host;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalSource {
    Activity,
    Density,
    Prints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityTag {
    Yorsh,
    NonYorsh,
    UniqueTicker,
    Flat,
    HasFutures,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookSide {
    Bid,
    Ask,
}

/// The aggressor of a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityStatus {
    Alive,
    Reduced,
    Dead,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityEvent {
    New,
    Update,
    Touched,
    Reduced,
    Dead,
    Reappeared,
    #[serde(other)]
    Other,
}

/// Trading activity of a ticker across exchanges (the Brush screener).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivitySignal {
    #[serde(default)]
    pub tags: Vec<ActivityTag>,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub spread_pct: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub volume_per_min_usd: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub pnl_per_min_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trades_per_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity_up_10pct_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity_down_10pct_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_age_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub print_gaps: Vec<f64>,
}

/// A large order resting in one order book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DensitySignal {
    pub exchange: String,
    pub market: Market,
    pub side: BookSide,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub price: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub qty: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub notional_initial_usd: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub notional_current_usd: f64,
    /// Average level notional of the symbol's book when the signal was updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notional_avg_usd: Option<f64>,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub eaten_pct: f64,
    /// Signed distance from the price to the level, in percent.
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub distance_pct: f64,
    #[serde(default)]
    pub touch_count: u32,
    #[serde(default)]
    pub lifetime_s: u64,
    pub status: DensityStatus,
    pub event: DensityEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_lifetime_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade_qty: Option<f64>,
}

/// A series of identical prints: the trace of a large iceberg execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintsSignal {
    pub exchange: String,
    pub market: Market,
    pub side: TradeSide,
    #[serde(default, deserialize_with = "de::f64_flex")]
    pub volume_usd: f64,
    #[serde(default)]
    pub batches: u32,
    #[serde(default)]
    pub prints_per_batch: u32,
}

/// One aggregator signal; exactly one of `activity`, `density`, `prints` is set, matching
/// `source`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub id: String,
    pub source: SignalSource,
    #[serde(default)]
    pub ts_ms: i64,
    /// Canonical `BASEQUOTE`.
    #[serde(default)]
    pub symbol: String,
    /// Where the symbol trades; exchanges the terminal does not know are left out.
    #[serde(default)]
    pub exchanges: Vec<ExchangeMarket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    // Boxed: a snapshot holds up to ~20000 signals with one payload each; inline, every
    // signal would be as large as all three payloads together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<Box<ActivitySignal>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<Box<DensitySignal>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prints: Option<Box<PrintsSignal>>,
}

impl Signal {
    /// The one market of a density or prints signal.
    pub fn market_ref(&self) -> Option<MarketRef> {
        let (exchange, market) = match (&self.density, &self.prints) {
            (Some(density), _) => (&density.exchange, density.market),
            (None, Some(prints)) => (&prints.exchange, prints.market),
            (None, None) => return None,
        };
        Some(MarketRef::new(
            exchange.clone(),
            market,
            self.symbol.clone(),
        ))
    }

    /// Every market of `exchanges`.
    pub fn markets(&self) -> Vec<MarketRef> {
        self.exchanges
            .iter()
            .map(|at| MarketRef::new(at.exchange.clone(), at.market, self.symbol.clone()))
            .collect()
    }
}

/// The answer of [`host::signals`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalsDelta {
    /// Pass it as `since` next time.
    #[serde(default)]
    pub seq: u64,
    /// `upserts` is the whole current snapshot: drop everything else.
    #[serde(default)]
    pub reset: bool,
    /// The terminal's connection to the aggregator for this source is alive: data or a server
    /// keepalive within the last 90 s. A sparse source can stay silent for long while it is `true`.
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub upserts: Vec<Signal>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// What one [`SignalFeed::poll`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedUpdate {
    pub reset: bool,
    /// Ids added or changed.
    pub upserted: Vec<String>,
    /// Ids gone, including those a reset left out.
    pub removed: Vec<String>,
}

impl FeedUpdate {
    pub fn is_empty(&self) -> bool {
        self.upserted.is_empty() && self.removed.is_empty()
    }
}

/// The current signals of one source, kept in sync with the terminal: call
/// [`SignalFeed::poll`] in `on_timer`.
#[derive(Debug, Clone)]
pub struct SignalFeed {
    source: SignalSource,
    seq: u64,
    connected: bool,
    signals: BTreeMap<String, Signal>,
}

impl SignalFeed {
    pub fn new(source: SignalSource) -> Self {
        Self {
            source,
            seq: 0,
            connected: false,
            signals: BTreeMap::new(),
        }
    }

    pub fn poll(&mut self) -> Result<FeedUpdate> {
        let delta = host::signals(self.source, (self.seq > 0).then_some(self.seq))?;
        Ok(self.apply(delta))
    }

    pub fn apply(&mut self, delta: SignalsDelta) -> FeedUpdate {
        self.seq = delta.seq;
        self.connected = delta.connected;
        let mut update = FeedUpdate {
            reset: delta.reset,
            ..FeedUpdate::default()
        };
        if delta.reset {
            // The previous snapshot goes before the new one is indexed: two snapshots of
            // ~20000 signals at once do not fit the plugin's memory.
            let fresh: HashSet<&str> = delta.upserts.iter().map(|s| s.id.as_str()).collect();
            update.removed = std::mem::take(&mut self.signals)
                .into_keys()
                .filter(|id| !fresh.contains(id.as_str()))
                .collect();
        }
        for signal in delta.upserts {
            update.upserted.push(signal.id.clone());
            self.signals.insert(signal.id.clone(), signal);
        }
        if !delta.reset {
            for id in delta.removed {
                if self.signals.remove(&id).is_some() {
                    update.removed.push(id);
                }
            }
        }
        update
    }

    pub fn source(&self) -> SignalSource {
        self.source
    }

    /// `connected` of the last poll: the connection to the aggregator is alive (data or a
    /// keepalive within 90 s), not that new signals arrived.
    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn get(&self, id: &str) -> Option<&Signal> {
        self.signals.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Signal> {
        self.signals.values()
    }

    pub fn len(&self) -> usize {
        self.signals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn density(id: &str, status: &str) -> serde_json::Value {
        json!({
            "id": id, "source": "density", "ts_ms": 1, "symbol": "BTCUSDT",
            "exchanges": [{"exchange": "binance", "market": "futures"}],
            "density": {
                "exchange": "binance", "market": "futures", "side": "bid", "price": 60000.5,
                "qty": 12, "notional_initial_usd": 900000, "notional_current_usd": 720000,
                "eaten_pct": 20, "distance_pct": -0.4, "touch_count": 2, "lifetime_s": 90,
                "status": status, "event": "touched", "future_field": 1
            }
        })
    }

    #[test]
    fn signals_parse_with_unknown_fields_and_values() {
        let delta: SignalsDelta = serde_json::from_value(json!({
            "seq": 7, "reset": true, "connected": true,
            "upserts": [density("d1", "alive"), density("d2", "frozen")],
            "removed": []
        }))
        .unwrap();
        let first = delta.upserts[0].density.as_ref().unwrap();
        assert_eq!(first.side, BookSide::Bid);
        assert_eq!(first.price, 60000.5);
        assert_eq!(
            delta.upserts[1].density.as_ref().unwrap().status,
            DensityStatus::Other
        );
        assert_eq!(
            delta.upserts[0].market_ref(),
            Some(MarketRef::new("binance", Market::Futures, "BTCUSDT"))
        );
    }

    #[test]
    fn feed_follows_resets_and_deltas() {
        let mut feed = SignalFeed::new(SignalSource::Density);
        let delta =
            |value: serde_json::Value| serde_json::from_value::<SignalsDelta>(value).unwrap();
        let update = feed.apply(delta(json!({
            "seq": 3, "reset": true, "connected": true,
            "upserts": [density("a", "alive"), density("b", "alive")]
        })));
        assert_eq!(update.upserted, ["a", "b"]);
        assert!(feed.connected());

        let update = feed.apply(delta(json!({
            "seq": 5, "upserts": [density("c", "alive")], "removed": ["a", "zz"]
        })));
        assert_eq!(update.upserted, ["c"]);
        assert_eq!(update.removed, ["a"]);
        assert_eq!(feed.len(), 2);
        assert!(!feed.connected());

        let update = feed.apply(delta(json!({
            "seq": 1, "reset": true, "upserts": [density("c", "dead")]
        })));
        assert!(update.reset);
        assert_eq!(update.removed, ["b"]);
        assert_eq!(feed.iter().count(), 1);
        assert_eq!(
            feed.get("c").unwrap().density.as_ref().unwrap().status,
            DensityStatus::Dead
        );
    }
}
