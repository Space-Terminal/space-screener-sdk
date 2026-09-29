use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::Result;

pub const SLUG: &str = "gate";
pub const HOST: &str = "api.gateio.ws";

pub fn request() -> HttpRequest {
    HttpRequest::get(format!("https://{HOST}/api/v4/futures/usdt/contracts"))
}

#[derive(Deserialize)]
struct Contract {
    name: String,
    #[serde(default, deserialize_with = "de::f64_flex")]
    position_size: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    quanto_multiplier: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_price: f64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    in_delisting: bool,
}

/// `position_size` is one side in contracts (`tickers.total_size` counts both sides).
pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let contracts: Vec<Contract> = from_json(body)?;
    Ok(contracts
        .into_iter()
        .filter(|c| c.status == "trading" && !c.in_delisting)
        .filter_map(|c| {
            let base = c.name.strip_suffix("_USDT")?.to_string();
            Some(OpenInterest::new(
                SLUG,
                c.name,
                &base,
                "USDT",
                c.position_size * c.quanto_multiplier * c.mark_price,
                c.mark_price,
            ))
        })
        .collect())
}
