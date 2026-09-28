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

#[derive(Debug, Clone, PartialEq)]
pub struct OpenInterest {
    pub exchange: &'static str,
    /// Native exchange symbol; pass it to `open_market` as is.
    pub symbol: String,
    /// Normalized base asset (`XBT` → `BTC`, Hyperliquid `kPEPE` → `1000PEPE`) for cross-exchange grouping.
    pub base: String,
    pub quote: String,
    pub oi_usd: f64,
    pub price: f64,
}

impl OpenInterest {
    pub fn market_ref(&self) -> MarketRef {
        MarketRef::new(self.exchange, Market::Futures, self.symbol.clone())
    }

    pub fn pair(&self) -> String {
        format!("{}{}", self.base, self.quote)
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub exchange: &'static str,
    pub result: Result<Vec<OpenInterest>>,
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
    BinancePremium,
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
pub struct Collector {
    exchanges: Vec<&'static str>,
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
            binance_symbols: None,
            mexc_sizes: None,
        }
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
            reqs.push((Slot::BinancePremium, binance::premium_index_request()));
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

    /// One round: returns a snapshot per exchange in [`EXCHANGES`] order.
    /// `Err` only when the host refused the batch itself.
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
                binance::SLUG => self.collect_binance(take(Slot::BinancePremium)),
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
            out.push(Snapshot { exchange, result });
        }
        Ok(out)
    }

    fn collect_binance(&self, premium: Result<String>) -> Result<Vec<OpenInterest>> {
        let symbols = &self
            .binance_symbols
            .as_ref()
            .ok_or_else(|| Error::parse("Binance symbols are unavailable"))?
            .value;
        let marks = binance::parse_premium_index(&premium?)?;
        let wanted: Vec<&binance::PerpSymbol> = symbols
            .iter()
            .filter(|s| marks.contains_key(&s.symbol))
            .collect();
        let reqs: Vec<HttpRequest> = wanted
            .iter()
            .map(|s| binance::open_interest_request(&s.symbol))
            .collect();
        let ois = host::http_batch(&reqs)?.into_iter().filter_map(|resp| {
            body(resp)
                .and_then(|b| binance::parse_open_interest(&b))
                .ok()
        });
        Ok(binance::assemble(symbols, &marks, ois))
    }
}

fn body(resp: Result<HttpResponse>) -> Result<String> {
    Ok(resp?.error_for_status()?.body)
}

pub(crate) fn from_json<T: DeserializeOwned>(body: &str) -> Result<T> {
    serde_json::from_str(body).map_err(Into::into)
}
