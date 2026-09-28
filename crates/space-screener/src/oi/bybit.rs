use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "bybit";
pub const HOST: &str = "api.bybit.com";

pub fn request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/v5/market/tickers?category=linear"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    ret_code: i64,
    #[serde(default)]
    ret_msg: String,
    #[serde(default)]
    result: Option<ResultList>,
}

#[derive(Deserialize)]
struct ResultList {
    #[serde(default)]
    list: Vec<Ticker>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticker {
    symbol: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_price: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    single_open_interest_value: f64,
}

/// `openInterestValue` counts both sides; `singleOpenInterestValue` is one side, in USD.
pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let resp: Response = from_json(body)?;
    if resp.ret_code != 0 {
        return Err(Error::parse(format!(
            "bybit {}: {}",
            resp.ret_code, resp.ret_msg
        )));
    }
    Ok(resp
        .result
        .map(|r| r.list)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !t.symbol.contains('-'))
        .filter_map(|t| {
            let base = t.symbol.strip_suffix("USDT")?.to_string();
            Some(OpenInterest::new(
                SLUG,
                t.symbol,
                &base,
                "USDT",
                t.single_open_interest_value,
                t.mark_price,
            ))
        })
        .collect())
}
