use std::collections::HashMap;

use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::Result;

pub const SLUG: &str = "binance";
pub const HOST: &str = "fapi.binance.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpSymbol {
    pub symbol: String,
    pub base: String,
    pub quote: String,
}

pub fn exchange_info_request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/fapi/v1/exchangeInfo"))
}

pub fn premium_index_request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/fapi/v1/premiumIndex"))
}

/// Binance has no bulk open interest endpoint: one request per symbol (weight 1).
pub fn open_interest_request(symbol: &str) -> HttpRequest {
    HttpRequest::get(format!(
        "https://{HOST}/fapi/v1/openInterest?symbol={symbol}"
    ))
}

#[derive(Deserialize)]
struct ExchangeInfo {
    symbols: Vec<InfoSymbol>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoSymbol {
    symbol: String,
    #[serde(default)]
    contract_type: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    base_asset: String,
    #[serde(default)]
    quote_asset: String,
}

pub fn parse_exchange_info(body: &str) -> Result<Vec<PerpSymbol>> {
    let info: ExchangeInfo = from_json(body)?;
    Ok(info
        .symbols
        .into_iter()
        .filter(|s| {
            s.contract_type == "PERPETUAL" && s.quote_asset == "USDT" && s.status == "TRADING"
        })
        .map(|s| PerpSymbol {
            symbol: s.symbol,
            base: s.base_asset,
            quote: s.quote_asset,
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Premium {
    symbol: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_price: f64,
}

pub fn parse_premium_index(body: &str) -> Result<HashMap<String, f64>> {
    let items: Vec<Premium> = from_json(body)?;
    Ok(items
        .into_iter()
        .map(|p| (p.symbol, p.mark_price))
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Oi {
    symbol: String,
    #[serde(deserialize_with = "de::f64_flex")]
    open_interest: f64,
}

/// Returns `(symbol, open interest in base asset)`.
pub fn parse_open_interest(body: &str) -> Result<(String, f64)> {
    let oi: Oi = from_json(body)?;
    Ok((oi.symbol, oi.open_interest))
}

pub fn assemble(
    symbols: &[PerpSymbol],
    marks: &HashMap<String, f64>,
    open_interest: impl IntoIterator<Item = (String, f64)>,
) -> Vec<OpenInterest> {
    let by_symbol: HashMap<&str, &PerpSymbol> =
        symbols.iter().map(|s| (s.symbol.as_str(), s)).collect();
    open_interest
        .into_iter()
        .filter_map(|(symbol, oi)| {
            let info = by_symbol.get(symbol.as_str())?;
            let price = *marks.get(&symbol)?;
            Some(OpenInterest::new(
                SLUG,
                symbol.clone(),
                &info.base,
                &info.quote,
                oi * price,
                price,
            ))
        })
        .collect()
}
