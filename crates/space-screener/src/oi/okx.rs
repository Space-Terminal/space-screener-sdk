use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "okx";
/// `www.okx.com` is blocked in parts of the CIS; `app.okx.com` serves the same API.
pub const HOST: &str = "app.okx.com";

pub fn request() -> HttpRequest {
    HttpRequest::get(format!(
        "https://{HOST}/api/v5/public/open-interest?instType=SWAP"
    ))
}

#[derive(Deserialize)]
struct Response {
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    inst_id: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    oi_ccy: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    oi_usd: f64,
}

pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let resp: Response = from_json(body)?;
    if resp.code != "0" {
        return Err(Error::parse(format!("okx {}: {}", resp.code, resp.msg)));
    }
    Ok(resp
        .data
        .into_iter()
        .filter_map(|item| {
            let mut parts = item.inst_id.split('-');
            let base = parts.next()?.to_string();
            let quote = parts.next()?;
            if quote != "USDT" {
                return None;
            }
            let price = if item.oi_ccy > 0.0 {
                item.oi_usd / item.oi_ccy
            } else {
                0.0
            };
            Some(OpenInterest {
                exchange: SLUG,
                base,
                quote: quote.to_string(),
                oi_usd: item.oi_usd,
                price,
                symbol: item.inst_id,
            })
        })
        .collect())
}
