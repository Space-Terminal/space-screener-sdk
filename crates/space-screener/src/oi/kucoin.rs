use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "kucoin";
pub const HOST: &str = "api-futures.kucoin.com";

pub fn request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/api/v1/contracts/active"))
}

#[derive(Deserialize)]
struct Response {
    code: String,
    #[serde(default)]
    data: Vec<Contract>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    symbol: String,
    #[serde(default)]
    base_currency: String,
    #[serde(default)]
    quote_currency: String,
    #[serde(default)]
    settle_currency: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    is_inverse: bool,
    #[serde(default, deserialize_with = "de::f64_flex")]
    multiplier: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_price: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    open_interest: f64,
}

/// `openInterest` is in lots; a lot is `multiplier` base coins. `FFWCSX` = perpetual.
pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let resp: Response = from_json(body)?;
    if resp.code != "200000" {
        return Err(Error::parse(format!("kucoin code {}", resp.code)));
    }
    Ok(resp
        .data
        .into_iter()
        .filter(|c| {
            c.settle_currency == "USDT"
                && c.quote_currency == "USDT"
                && c.kind == "FFWCSX"
                && c.status == "Open"
                && !c.is_inverse
        })
        .map(|c| OpenInterest {
            exchange: SLUG,
            base: normalize_base(&c.base_currency),
            quote: c.quote_currency,
            oi_usd: c.open_interest * c.multiplier * c.mark_price,
            price: c.mark_price,
            symbol: c.symbol,
        })
        .collect())
}

fn normalize_base(base: &str) -> String {
    match base {
        "XBT" => "BTC".into(),
        other => other.into(),
    }
}
