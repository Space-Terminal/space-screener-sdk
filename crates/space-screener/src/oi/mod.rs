//! Open interest of USDT-margined perpetuals (USDC for Hyperliquid) on 8 exchanges, one side, in USD.
//!
//! Every exchange module exposes request builders and a pure parser, so parsers can be tested
//! natively. [`Collector`] does the HTTP through the host with two `http_batch` calls per round.

pub mod binance;
pub mod bitget;
pub mod bybit;
pub mod gate;
pub mod hyperliquid;
pub mod kucoin;
pub mod mexc;
pub mod okx;

use std::collections::HashMap;

use serde::de::DeserializeOwned;

use crate::abi::{HttpRequest, HttpResponse, Market, MarketRef};
use crate::errors::{Error, Result};
use crate::host;
use crate::series::hours;

/// Exchange slugs in the terminal's notation, in [`Collector`] order.
pub const EXCHANGES: [&str; 8] = [
    binance::SLUG,
    bybit::SLUG,
    okx::SLUG,
    bitget::SLUG,
    gate::SLUG,
    mexc::SLUG,
    kucoin::SLUG,
    hyperliquid::SLUG,
];

/// Hosts to list under `http:` in `manifest.yaml`.
pub const HOSTS: [&str; 8] = [
    binance::HOST,
    bybit::HOST,
    okx::HOST,
    bitget::HOST,
    gate::HOST,
    mexc::HOST,
    kucoin::HOST,
    hyperliquid::HOST,
];

const METADATA_TTL_MS: i64 = hours(1);

/// Binance symbols polled per round by default: top by 24h quote volume.
pub const BINANCE_TOP_N: usize = 200;
/// Upper bound of the Binance top: 600 symbols ≈ 120 s at 5 requests/s, which covers every
/// USDT perpetual and stays well inside the 600 s wall limit of one plugin call.
pub const BINANCE_TOP_N_MAX: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct OpenInterest {
    pub exchange: &'static str,
    /// Canonical `BASEQUOTE` in upper case (`BTCUSDT`, Hyperliquid `BTCUSDC`, KuCoin `XBT` → `BTCUSDT`):
    /// use it for rows and `open_market`.
    pub symbol: String,
    /// The exchange's own symbol (`BTC-USDT-SWAP`, `BTC_USDT`, `XBTUSDTM`, `BTC`): use it in exchange requests.
    pub native_symbol: String,
    pub base: String,
    pub quote: String,
    pub oi_usd: f64,
    pub price: f64,
}

impl OpenInterest {
    pub(crate) fn new(
        exchange: &'static str,
        native_symbol: String,
        base: &str,
        quote: &str,
        oi_usd: f64,
        price: f64,
    ) -> Self {
        let base = base.to_ascii_uppercase();
        let quote = quote.to_ascii_uppercase();
        Self {
            exchange,
            symbol: format!("{base}{quote}"),
            native_symbol,
            base,
            quote,
            oi_usd,
            price,
        }
    }

    pub fn market_ref(&self) -> MarketRef {
        MarketRef::new(self.exchange, Market::Futures, self.symbol.clone())
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub exchange: &'static str,
    /// Items that arrived; for per-symbol exchanges (Binance) it holds the successful part.
    pub result: Result<Vec<OpenInterest>>,
    /// Per-symbol requests of this round; 0 for exchanges with a bulk endpoint.
    pub requested: usize,
    /// How many of `requested` failed.
    pub failed: usize,
    /// The first per-symbol failure, to log the reason.
    pub first_error: Option<Error>,
}

impl Snapshot {
    fn bulk(exchange: &'static str, result: Result<Vec<OpenInterest>>) -> Self {
        Self {
            exchange,
            result,
            requested: 0,
            failed: 0,
            first_error: None,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.result.is_ok() && self.failed == 0
    }

    /// Short status: `okx`, `binance 180/200`, `mexc: no data`.
    pub fn summary(&self) -> String {
        match (&self.result, self.requested) {
            (Err(_), 0) => format!("{}: no data", self.exchange),
            (_, 0) => self.exchange.to_string(),
            _ => format!(
                "{} {}/{}",
                self.exchange,
                self.requested - self.failed,
                self.requested
            ),
        }
    }
}

struct Cached<T> {
    at_ms: i64,
    value: T,
}

impl<T> Cached<T> {
    fn fresh(slot: &Option<Self>, now_ms: i64) -> bool {
        slot.as_ref()
            .is_some_and(|c| now_ms - c.at_ms < METADATA_TTL_MS)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    BinanceInfo,
    BinanceTicker,
    Bybit,
    Okx,
    Bitget,
    Gate,
    MexcTicker,
    MexcDetail,
    Kucoin,
    Hyperliquid,
}

/// Polls open interest; keeps slow-changing metadata (Binance symbols, MEXC contract sizes) for an hour.
///
/// Binance needs one request per symbol, and the terminal allows 5 requests/s to
/// `fapi.binance.com`, so a round of N Binance symbols takes about N/5 s. Only the top
/// [`BINANCE_TOP_N`] by 24h volume are polled unless [`Collector::set_binance_top_n`] says otherwise
/// (1..=[`BINANCE_TOP_N_MAX`]).
pub struct Collector {
    exchanges: Vec<&'static str>,
    binance_top_n: usize,
    binance_symbols: Option<Cached<Vec<binance::PerpSymbol>>>,
    mexc_sizes: Option<Cached<HashMap<String, f64>>>,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Self {
        Self::only(&EXCHANGES)
    }

    /// Unknown slugs are ignored.
    pub fn only(exchanges: &[&str]) -> Self {
        Self {
            exchanges: EXCHANGES
                .into_iter()
                .filter(|e| exchanges.contains(e))
                .collect(),
            binance_top_n: BINANCE_TOP_N,
            binance_symbols: None,
            mexc_sizes: None,
        }
    }

    /// Clamped to 1..=[`BINANCE_TOP_N_MAX`]; the maximum covers every Binance USDT perpetual.
    pub fn with_binance_top_n(mut self, top_n: usize) -> Self {
        self.set_binance_top_n(top_n);
        self
    }

    /// Clamped to 1..=[`BINANCE_TOP_N_MAX`].
    pub fn set_binance_top_n(&mut self, top_n: usize) {
        self.binance_top_n = top_n.clamp(1, BINANCE_TOP_N_MAX);
    }

    fn wants(&self, slug: &str) -> bool {
        self.exchanges.contains(&slug)
    }

    fn bulk_requests(&self, now_ms: i64) -> Vec<(Slot, HttpRequest)> {
        let mut reqs = Vec::new();
        if self.wants(binance::SLUG) {
            if !Cached::fresh(&self.binance_symbols, now_ms) {
                reqs.push((Slot::BinanceInfo, binance::exchange_info_request()));
            }
            reqs.push((Slot::BinanceTicker, binance::ticker_24h_request()));
        }
        if self.wants(bybit::SLUG) {
            reqs.push((Slot::Bybit, bybit::request()));
        }
        if self.wants(okx::SLUG) {
            reqs.push((Slot::Okx, okx::request()));
        }
        if self.wants(bitget::SLUG) {
            reqs.push((Slot::Bitget, bitget::request()));
        }
        if self.wants(gate::SLUG) {
            reqs.push((Slot::Gate, gate::request()));
        }
        if self.wants(mexc::SLUG) {
            reqs.push((Slot::MexcTicker, mexc::ticker_request()));
            if !Cached::fresh(&self.mexc_sizes, now_ms) {
                reqs.push((Slot::MexcDetail, mexc::detail_request()));
            }
        }
        if self.wants(kucoin::SLUG) {
            reqs.push((Slot::Kucoin, kucoin::request()));
        }
        if self.wants(hyperliquid::SLUG) {
            reqs.push((Slot::Hyperliquid, hyperliquid::request()));
        }
        reqs
    }

    /// One round: returns a snapshot per exchange in [`EXCHANGES`] order; a failure of one exchange,
    /// including a refused per-symbol batch, stays in its snapshot.
    /// `Err` only when the host refused the shared bulk batch, which carries every exchange.
    pub fn collect(&mut self, now_ms: i64) -> Result<Vec<Snapshot>> {
        let (slots, reqs): (Vec<Slot>, Vec<HttpRequest>) =
            self.bulk_requests(now_ms).into_iter().unzip();
        let mut bodies: HashMap<Slot, Result<String>> = slots
            .into_iter()
            .zip(host::http_batch(&reqs)?)
            .map(|(slot, resp)| (slot, body(resp)))
            .collect();
        let mut take = |slot: Slot| {
            bodies
                .remove(&slot)
                .unwrap_or_else(|| Err(Error::parse("no response")))
        };

        if let Some(Ok(info)) = self.wants(binance::SLUG).then(|| take(Slot::BinanceInfo)) {
            if let Ok(symbols) = binance::parse_exchange_info(&info) {
                self.binance_symbols = Some(Cached {
                    at_ms: now_ms,
                    value: symbols,
                });
            }
        }
        if let Some(Ok(detail)) = self.wants(mexc::SLUG).then(|| take(Slot::MexcDetail)) {
            if let Ok(sizes) = mexc::parse_contract_sizes(&detail) {
                self.mexc_sizes = Some(Cached {
                    at_ms: now_ms,
                    value: sizes,
                });
            }
        }

        let mut out = Vec::with_capacity(self.exchanges.len());
        for &exchange in &self.exchanges {
            let result = match exchange {
                binance::SLUG => {
                    out.push(self.collect_binance(take(Slot::BinanceTicker)));
                    continue;
                }
                bybit::SLUG => take(Slot::Bybit).and_then(|b| bybit::parse(&b)),
                okx::SLUG => take(Slot::Okx).and_then(|b| okx::parse(&b)),
                bitget::SLUG => take(Slot::Bitget).and_then(|b| bitget::parse(&b)),
                gate::SLUG => take(Slot::Gate).and_then(|b| gate::parse(&b)),
                mexc::SLUG => match &self.mexc_sizes {
                    Some(sizes) => {
                        take(Slot::MexcTicker).and_then(|b| mexc::parse(&b, &sizes.value))
                    }
                    None => Err(Error::parse("MEXC contract sizes are unavailable")),
                },
                kucoin::SLUG => take(Slot::Kucoin).and_then(|b| kucoin::parse(&b)),
                hyperliquid::SLUG => take(Slot::Hyperliquid).and_then(|b| hyperliquid::parse(&b)),
                _ => continue,
            };
            out.push(Snapshot::bulk(exchange, result));
        }
        Ok(out)
    }

    fn collect_binance(&self, ticker: Result<String>) -> Snapshot {
        let prepared = self
            .binance_symbols
            .as_ref()
            .ok_or_else(|| Error::parse("Binance symbols are unavailable"))
            .and_then(|symbols| {
                let days = binance::parse_ticker_24h(&ticker?)?;
                Ok((&symbols.value, days))
            });
        let (symbols, days) = match prepared {
            Ok(prepared) => prepared,
            Err(e) => return Snapshot::bulk(binance::SLUG, Err(e)),
        };
        let wanted = binance::select_top(symbols, &days, self.binance_top_n);
        let reqs: Vec<HttpRequest> = wanted
            .iter()
            .map(|s| binance::open_interest_request(&s.symbol))
            .collect();
        let responses = match host::http_batch(&reqs) {
            Ok(responses) => responses,
            Err(e) => {
                return Snapshot {
                    exchange: binance::SLUG,
                    result: Err(e),
                    requested: reqs.len(),
                    failed: reqs.len(),
                    first_error: None,
                };
            }
        };
        let mut failed = 0;
        let mut first_error = None;
        let mut ois = Vec::with_capacity(reqs.len());
        for resp in responses {
            match body(resp).and_then(|b| binance::parse_open_interest(&b)) {
                Ok(oi) => ois.push(oi),
                Err(e) => {
                    failed += 1;
                    first_error.get_or_insert(e);
                }
            }
        }
        let result = match (ois.is_empty(), first_error.take()) {
            (true, Some(e)) => Err(e),
            (_, e) => {
                first_error = e;
                Ok(binance::assemble(symbols, &days, ois))
            }
        };
        Snapshot {
            exchange: binance::SLUG,
            result,
            requested: reqs.len(),
            failed,
            first_error,
        }
    }
}

fn body(resp: Result<HttpResponse>) -> Result<String> {
    Ok(resp?.error_for_status()?.body)
}

pub(crate) fn from_json<T: DeserializeOwned>(body: &str) -> Result<T> {
    serde_json::from_str(body).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_summary_shows_partial_success() {
        let partial = Snapshot {
            exchange: binance::SLUG,
            result: Ok(Vec::new()),
            requested: 200,
            failed: 20,
            first_error: Some(Error::parse("timeout")),
        };
        assert_eq!(partial.summary(), "binance 180/200");
        assert!(!partial.is_complete());

        let bulk = Snapshot::bulk(okx::SLUG, Ok(Vec::new()));
        assert_eq!(bulk.summary(), "okx");
        assert!(bulk.is_complete());

        let failed = Snapshot::bulk(mexc::SLUG, Err(Error::parse("x")));
        assert_eq!(failed.summary(), "mexc: no data");
    }

    #[test]
    fn binance_top_n_is_clamped() {
        assert_eq!(Collector::new().with_binance_top_n(0).binance_top_n, 1);
        assert_eq!(
            Collector::new().with_binance_top_n(10_000).binance_top_n,
            BINANCE_TOP_N_MAX
        );
    }

    // Natively every host call fails, which is exactly a refused per-symbol batch.
    #[test]
    fn refused_binance_batch_stays_in_its_snapshot() {
        let info = include_str!("../../tests/fixtures/binance_exchange_info.json");
        let ticker = include_str!("../../tests/fixtures/binance_ticker_24hr.json");
        let mut collector = Collector::new();
        collector.binance_symbols = Some(Cached {
            at_ms: 0,
            value: binance::parse_exchange_info(info).unwrap(),
        });
        let snapshot = collector.collect_binance(Ok(ticker.to_string()));
        assert!(snapshot.result.is_err());
        assert_eq!((snapshot.requested, snapshot.failed), (2, 2));
        assert_eq!(snapshot.summary(), "binance 0/2");
    }
}
