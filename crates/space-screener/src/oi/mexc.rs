use std::collections::HashMap;

use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "mexc";
pub const HOST: &str = "contract.mexc.com";

pub fn ticker_request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/api/v1/contract/ticker"))
}

/// Rate limited to 1 request per 5 s — cache the result ([`super::Collector`] keeps it for an hour).
pub fn detail_request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/api/v1/contract/detail"))
}

#[derive(Deserialize)]
struct Response<T> {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    code: i64,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

impl<T> Response<T> {
    fn data(self) -> Result<Vec<T>> {
        if !self.success {
            return Err(Error::parse(format!("mexc code {}", self.code)));
        }
        Ok(self.data)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    symbol: String,
    #[serde(default)]
    quote_coin: String,
    #[serde(default)]
    settle_coin: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    contract_size: f64,
}

pub fn parse_contract_sizes(body: &str) -> Result<HashMap<String, f64>> {
    let resp: Response<Detail> = from_json(body)?;
    Ok(resp
        .data()?
        .into_iter()
        .filter(|d| d.quote_coin == "USDT" && d.settle_coin == "USDT")
        .map(|d| (d.symbol, d.contract_size))
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticker {
    symbol: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    hold_vol: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    fair_price: f64,
}

/// Assumption: `holdVol` counts both sides. For BTC it is about twice the one-side figures of
/// Bybit (`singleOpenInterest`) and Gate (`position_size`), so it is halved. Not confirmed by the docs.
pub fn parse(body: &str, contract_sizes: &HashMap<String, f64>) -> Result<Vec<OpenInterest>> {
    let resp: Response<Ticker> = from_json(body)?;
    Ok(resp
        .data()?
        .into_iter()
        .filter_map(|t| {
            let size = *contract_sizes.get(&t.symbol)?;
            let base = t.symbol.strip_suffix("_USDT")?.to_string();
            Some(OpenInterest::new(
                SLUG,
                t.symbol,
                &base,
                "USDT",
                t.hold_vol * size * t.fair_price / 2.0,
                t.fair_price,
            ))
        })
        .collect())
}
