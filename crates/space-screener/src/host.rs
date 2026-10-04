use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::abi::{
    AlertLevel, ClusterHistory, ClusterRequest, ExchangeInfo, HttpRequest, HttpResponse, LogLevel,
    Market, MarketRef, ReplayRequest, SmartLevel, SpreadLayout, StatusTone, SymbolInfo,
    TickerSnapshot,
};
use crate::errors::{HostError, Result};
use crate::row::Row;
use crate::signals::{SignalSource, SignalsDelta};

pub const HTTP_BATCH_MAX: usize = 1000;
/// Markets one [`open_markets`] call may open.
pub const OPEN_MARKETS_MAX: usize = 16;

/// Each wrapper passes its own import to `raw::call`, so a plugin imports only the host
/// functions it calls: a function newer than the plugin's `min_terminal` would make older
/// terminals refuse the whole module.
macro_rules! host_imports {
    ($($name:ident),* $(,)?) => {
        #[cfg(target_arch = "wasm32")]
        mod import {
            #[link(wasm_import_module = "extism:host/user")]
            unsafe extern "C" {
                $(pub(super) fn $name(input: u64) -> u64;)*
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        mod import {
            $(pub(super) unsafe extern "C" fn $name(_: u64) -> u64 { 0 })*
        }
    };
}

host_imports!(
    http,
    http_batch,
    tickers,
    symbols,
    exchanges,
    history_cluster,
    history_replay,
    kv_get,
    kv_set,
    emit_rows,
    expire,
    emit_alert,
    set_status,
    open_market,
    open_markets,
    open_spread,
    signals,
    log,
    now_ms,
);

type HostFn = unsafe extern "C" fn(u64) -> u64;

#[cfg(target_arch = "wasm32")]
mod raw {
    use extism_pdk::Memory;

    use super::HostFn;
    use crate::errors::{Error, Result};

    // Input and output are freed right away: a batch answer can weigh megabytes and
    // Extism would otherwise keep it until the export returns.
    pub(super) fn call(f: HostFn, input: String) -> Result<String> {
        let mem =
            Memory::from_bytes(input.as_bytes()).map_err(|e| Error::Call(format!("{e:#}")))?;
        let offs = mem.offset();
        // SAFETY: every import takes one Extism memory offset holding UTF-8 JSON and returns one.
        let out_offs = unsafe { f(offs) };
        mem.free();
        let out =
            Memory::find(out_offs).ok_or_else(|| Error::Call("host returned no output".into()))?;
        let bytes = out.to_vec();
        out.free();
        String::from_utf8(bytes).map_err(|e| Error::Call(e.to_string()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod raw {
    use super::HostFn;
    use crate::errors::{Error, Result};

    pub(super) fn call(_: HostFn, _: String) -> Result<String> {
        Err(Error::NotInTerminal)
    }
}

#[derive(serde::Deserialize)]
struct Envelope<T> {
    ok: Option<T>,
    #[serde(default)]
    err: Option<HostError>,
}

impl<T: DeserializeOwned> Envelope<T> {
    fn into_result(self) -> Result<T> {
        if let Some(err) = self.err {
            return Err(err.into());
        }
        match self.ok {
            Some(v) => Ok(v),
            None => serde_json::from_value(Value::Null).map_err(Into::into),
        }
    }
}

#[derive(Serialize)]
struct OpenMarket<'a> {
    #[serde(flatten)]
    market: &'a MarketRef,
    smart_level: &'a SmartLevel,
}

#[derive(Serialize)]
struct OpenMarkets<'a> {
    markets: &'a [MarketRef],
}

#[derive(Serialize)]
struct SignalsRequest {
    source: SignalSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<u64>,
}

#[derive(Serialize)]
struct OpenSpread<'a> {
    a: &'a MarketRef,
    b: &'a MarketRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<SpreadLayout>,
}

#[derive(Serialize)]
struct HttpBatch<'a> {
    requests: &'a [HttpRequest],
}

#[derive(Serialize)]
struct EmitRows<'a> {
    rows: &'a [Row],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    replace: bool,
}

fn call<I: Serialize + ?Sized, O: DeserializeOwned>(f: HostFn, input: &I) -> Result<O> {
    let out = raw::call(f, serde_json::to_string(input)?)?;
    serde_json::from_str::<Envelope<O>>(&out)?.into_result()
}

pub fn http(req: &HttpRequest) -> Result<HttpResponse> {
    call(import::http, req)
}

/// Runs requests concurrently on the host (quotas still apply); results keep the request order.
pub fn http_batch(reqs: &[HttpRequest]) -> Result<Vec<Result<HttpResponse>>> {
    let mut all = Vec::with_capacity(reqs.len());
    for chunk in reqs.chunks(HTTP_BATCH_MAX) {
        let items: Vec<Envelope<HttpResponse>> =
            call(import::http_batch, &HttpBatch { requests: chunk })?;
        all.extend(items.into_iter().map(Envelope::into_result));
    }
    Ok(all)
}

pub fn tickers(exchange: &str, market: Market) -> Result<TickerSnapshot> {
    call(
        import::tickers,
        &json!({ "exchange": exchange, "market": market }),
    )
}

pub fn symbols(exchange: &str, market: Market) -> Result<Vec<SymbolInfo>> {
    call(
        import::symbols,
        &json!({ "exchange": exchange, "market": market }),
    )
}

pub fn exchanges() -> Result<Vec<ExchangeInfo>> {
    call(import::exchanges, &Value::Null)
}

pub fn history_cluster(req: &ClusterRequest) -> Result<ClusterHistory> {
    call(import::history_cluster, req)
}

/// The replay chunk shape follows the terminal's cloud replay and is not frozen in ABI v1.
pub fn history_replay(req: &ReplayRequest) -> Result<Value> {
    call(import::history_replay, req)
}

pub fn kv_get<T: DeserializeOwned>(key: &str) -> Result<Option<T>> {
    let value: Value = call(import::kv_get, &json!({ "key": key }))?;
    match value {
        Value::Null => Ok(None),
        v => Ok(Some(serde_json::from_value(v)?)),
    }
}

pub fn kv_set<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<()> {
    call(import::kv_set, &json!({ "key": key, "value": value }))
}

pub fn kv_delete(key: &str) -> Result<()> {
    call(import::kv_set, &json!({ "key": key, "value": Value::Null }))
}

/// Upserts rows by `key`; rows not mentioned stay.
pub fn emit_rows(rows: impl IntoIterator<Item = Row>) -> Result<()> {
    let rows: Vec<Row> = rows.into_iter().collect();
    call(
        import::emit_rows,
        &EmitRows {
            rows: &rows,
            replace: false,
        },
    )
}

/// Replaces the whole table with `rows`.
pub fn replace_rows(rows: impl IntoIterator<Item = Row>) -> Result<()> {
    let rows: Vec<Row> = rows.into_iter().collect();
    call(
        import::emit_rows,
        &EmitRows {
            rows: &rows,
            replace: true,
        },
    )
}

pub fn expire<K: Into<String>>(keys: impl IntoIterator<Item = K>) -> Result<()> {
    let keys: Vec<String> = keys.into_iter().map(Into::into).collect();
    call(import::expire, &json!({ "keys": keys }))
}

pub fn alert(level: AlertLevel, title: impl Into<String>, body: impl Into<String>) -> Result<()> {
    call(
        import::emit_alert,
        &json!({ "level": level, "title": title.into(), "body": body.into() }),
    )
}

pub fn alert_row(
    level: AlertLevel,
    title: impl Into<String>,
    body: impl Into<String>,
    row_key: impl Into<String>,
) -> Result<()> {
    call(
        import::emit_alert,
        &json!({
            "level": level,
            "title": title.into(),
            "body": body.into(),
            "row_key": row_key.into(),
        }),
    )
}

pub fn set_status(tone: StatusTone, text: impl Into<String>) -> Result<()> {
    call(
        import::set_status,
        &json!({ "text": text.into(), "tone": tone }),
    )
}

/// Only valid inside `on_click`; at most one open per click.
pub fn open_market(market: &MarketRef) -> Result<()> {
    call(import::open_market, market)
}

/// Opens the market and places a smart level at the price of a density signal (its exact
/// price and live metrics come from the terminal). Only valid inside `on_click`; at most one
/// open per click. A terminal older than 0.104.72 ignores the level and opens the market
/// without it.
pub fn open_market_with_level(market: &MarketRef, level: &SmartLevel) -> Result<()> {
    call(
        import::open_market,
        &OpenMarket {
            market,
            smart_level: level,
        },
    )
}

/// Opens 1..=[`OPEN_MARKETS_MAX`] markets at once. Only valid inside `on_click`; at most one
/// open per click. Needs terminal 0.104.72 or newer.
pub fn open_markets(markets: &[MarketRef]) -> Result<()> {
    call(import::open_markets, &OpenMarkets { markets })
}

/// Only valid inside `on_click`; at most one open per click.
pub fn open_spread(a: &MarketRef, b: &MarketRef, layout: Option<SpreadLayout>) -> Result<()> {
    call(import::open_spread, &OpenSpread { a, b, layout })
}

/// Signals of `source` changed after `since` (`None`: the whole snapshot, `reset` is set).
/// Needs the source in `signals` of manifest.yaml and terminal 0.104.72 or newer; see
/// [`crate::SignalFeed`] for a ready cursor.
pub fn signals(source: SignalSource, since: Option<u64>) -> Result<SignalsDelta> {
    call(import::signals, &SignalsRequest { source, since })
}

/// Best effort: a failed log call is dropped.
pub fn log(level: LogLevel, msg: impl Into<String>) {
    let _ = call::<_, ()>(import::log, &json!({ "level": level, "msg": msg.into() }));
}

pub fn now_ms() -> Result<i64> {
    call(import::now_ms, &Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn envelope_ok_null_is_unit() {
        let e: Envelope<()> = serde_json::from_str(r#"{"ok":null}"#).unwrap();
        assert!(e.into_result().is_ok());
    }

    #[test]
    fn envelope_err_is_typed() {
        let e: Envelope<HttpResponse> =
            serde_json::from_str(r#"{"err":{"code":"host_not_allowed","message":"x"}}"#).unwrap();
        let err = e.into_result().unwrap_err();
        assert_eq!(err.code(), Some("host_not_allowed"));
    }

    #[test]
    fn envelope_batch_items() {
        let items: Envelope<Vec<Envelope<HttpResponse>>> = serde_json::from_str(
            r#"{"ok":[{"ok":{"status":200,"headers":{},"body":"{}"}},{"err":{"code":"timeout","message":"t"}}]}"#,
        )
        .unwrap();
        let items: Vec<_> = items
            .into_result()
            .unwrap()
            .into_iter()
            .map(Envelope::into_result)
            .collect();
        assert_eq!(items[0].as_ref().unwrap().status, 200);
        assert_eq!(items[1].as_ref().unwrap_err().code(), Some("timeout"));
    }

    #[test]
    fn open_spread_omits_missing_layout() {
        let a = MarketRef::new("binance", Market::Futures, "BTCUSDT");
        let b = MarketRef::new("bybit", Market::Spot, "BTCUSDT");
        let json = serde_json::to_value(OpenSpread {
            a: &a,
            b: &b,
            layout: None,
        })
        .unwrap();
        assert!(json.get("layout").is_none());
    }

    #[test]
    fn open_market_with_level_flattens_the_market() {
        let market = MarketRef::new("binance", Market::Futures, "BTCUSDT");
        let level = SmartLevel::new("d1").with_sound(true);
        let json = serde_json::to_value(OpenMarket {
            market: &market,
            smart_level: &level,
        })
        .unwrap();
        assert_eq!(
            json,
            json!({"exchange": "binance", "market": "futures", "symbol": "BTCUSDT",
                   "smart_level": {"signal_id": "d1", "sound": true}})
        );
        let request = serde_json::to_value(SignalsRequest {
            source: SignalSource::Prints,
            since: None,
        })
        .unwrap();
        assert_eq!(request, json!({"source": "prints"}));
    }

    #[test]
    fn outside_terminal_calls_fail_cleanly() {
        assert!(matches!(now_ms(), Err(Error::NotInTerminal)));
    }
}
