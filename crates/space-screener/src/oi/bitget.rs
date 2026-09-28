use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "bitget";
pub const HOST: &str = "api.bitget.com";

pub fn request() -> HttpRequest {
    HttpRequest::get(format!(
        "https://{HOST}/api/v2/mix/market/tickers?productType=USDT-FUTURES"
    ))
}

#[derive(Deserialize)]
struct Response {
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Vec<Ticker>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticker {
    symbol: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    holding_amount: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_price: f64,
}

/// `holdingAmount` is in base coins.
pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let resp: Response = from_json(body)?;
    if resp.code != "00000" {
        return Err(Error::parse(format!("bitget {}: {}", resp.code, resp.msg)));
    }
    Ok(resp
        .data
        .into_iter()
        .filter_map(|t| {
            let base = t.symbol.strip_suffix("USDT")?.to_string();
            Some(OpenInterest {
                exchange: SLUG,
                base,
                quote: "USDT".into(),
                oi_usd: t.holding_amount * t.mark_price,
                price: t.mark_price,
                symbol: t.symbol,
            })
        })
        .collect())
}
