use serde::Deserialize;

use super::{OpenInterest, from_json};
use crate::abi::{HttpRequest, de};
use crate::errors::{Error, Result};

pub const SLUG: &str = "hyperliquid";
pub const HOST: &str = "api.hyperliquid.xyz";

pub fn request() -> HttpRequest {
    HttpRequest::post(
        format!("https://{HOST}/info"),
        r#"{"type":"metaAndAssetCtxs"}"#,
    )
}

#[derive(Deserialize)]
struct Response(Meta, Vec<AssetCtx>);

#[derive(Deserialize)]
struct Meta {
    universe: Vec<Asset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    name: String,
    #[serde(default)]
    is_delisted: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetCtx {
    #[serde(default, deserialize_with = "de::f64_flex")]
    open_interest: f64,
    #[serde(default, deserialize_with = "de::f64_flex")]
    mark_px: f64,
}

/// Contexts are aligned with `universe` by index; open interest is in coins, margin is USDC.
/// `kPEPE` stays `KPEPEUSDC` (1000 PEPE): that is how the terminal lists it.
pub fn parse(body: &str) -> Result<Vec<OpenInterest>> {
    let Response(meta, ctxs) = from_json(body)?;
    if meta.universe.len() != ctxs.len() {
        return Err(Error::parse(
            "hyperliquid: universe and contexts differ in length",
        ));
    }
    Ok(meta
        .universe
        .into_iter()
        .zip(ctxs)
        .filter(|(asset, _)| !asset.is_delisted)
        .map(|(asset, ctx)| {
            OpenInterest::new(
                SLUG,
                asset.name.clone(),
                &asset.name,
                "USDC",
                ctx.open_interest * ctx.mark_px,
                ctx.mark_px,
            )
        })
        .collect())
}
