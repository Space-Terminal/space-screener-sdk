use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::abi::{
    AlertLevel, ClusterHistory, ClusterRequest, ExchangeInfo, HttpRequest, HttpResponse, LogLevel,
    Market, MarketRef, ReplayRequest, SpreadLayout, StatusTone, SymbolInfo, TickerSnapshot,
};
use crate::errors::{HostError, Result};
use crate::row::Row;

pub const HTTP_BATCH_MAX: usize = 1000;

#[derive(Debug, Clone, Copy)]
pub(crate) enum HostFn {
    Http,
    HttpBatch,
    Tickers,
    Symbols,
    Exchanges,
    HistoryCluster,
    HistoryReplay,
    KvGet,
    KvSet,
    EmitRows,
    Expire,
    EmitAlert,
    SetStatus,
    OpenMarket,
    OpenSpread,
    Log,
    NowMs,
}

#[cfg(target_arch = "wasm32")]
mod raw {
    use extism_pdk::Memory;

    use super::HostFn;
    use crate::errors::{Error, Result};

    #[link(wasm_import_module = "extism:host/user")]
    unsafe extern "C" {
        fn http(input: u64) -> u64;
        fn http_batch(input: u64) -> u64;
        fn tickers(input: u64) -> u64;
        fn symbols(input: u64) -> u64;
        fn exchanges(input: u64) -> u64;
        fn history_cluster(input: u64) -> u64;
        fn history_replay(input: u64) -> u64;
        fn kv_get(input: u64) -> u64;
        fn kv_set(input: u64) -> u64;
        fn emit_rows(input: u64) -> u64;
        fn expire(input: u64) -> u64;
        fn emit_alert(input: u64) -> u64;
        fn set_status(input: u64) -> u64;
        fn open_market(input: u64) -> u64;
        fn open_spread(input: u64) -> u64;
        fn log(input: u64) -> u64;
        fn now_ms(input: u64) -> u64;
    }

    // Input and output are freed right away: a batch answer can weigh megabytes and
    // Extism would otherwise keep it until the export returns.
    pub(super) fn call(f: HostFn, input: String) -> Result<String> {
        let mem =
            Memory::from_bytes(input.as_bytes()).map_err(|e| Error::Call(format!("{e:#}")))?;
        let offs = mem.offset();
        // SAFETY: every import takes one Extism memory offset holding UTF-8 JSON and returns one.
        let out_offs = unsafe {
            match f {
                HostFn::Http => http(offs),
                HostFn::HttpBatch => http_batch(offs),
                HostFn::Tickers => tickers(offs),
                HostFn::Symbols => symbols(offs),
                HostFn::Exchanges => exchanges(offs),
                HostFn::HistoryCluster => history_cluster(offs),
                HostFn::HistoryReplay => history_replay(offs),
                HostFn::KvGet => kv_get(offs),
                HostFn::KvSet => kv_set(offs),
                HostFn::EmitRows => emit_rows(offs),
                HostFn::Expire => expire(offs),
                HostFn::EmitAlert => emit_alert(offs),
                HostFn::SetStatus => set_status(offs),
                HostFn::OpenMarket => open_market(offs),
                HostFn::OpenSpread => open_spread(offs),
                HostFn::Log => log(offs),
                HostFn::NowMs => now_ms(offs),
            }
        };
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
    call(HostFn::Http, req)
}

/// Runs requests concurrently on the host (quotas still apply); results keep the request order.
pub fn http_batch(reqs: &[HttpRequest]) -> Result<Vec<Result<HttpResponse>>> {
    let mut all = Vec::with_capacity(reqs.len());
    for chunk in reqs.chunks(HTTP_BATCH_MAX) {
        let items: Vec<Envelope<HttpResponse>> =
            call(HostFn::HttpBatch, &HttpBatch { requests: chunk })?;
        all.extend(items.into_iter().map(Envelope::into_result));
    }
    Ok(all)
}

pub fn tickers(exchange: &str, market: Market) -> Result<TickerSnapshot> {
    call(
        HostFn::Tickers,
        &json!({ "exchange": exchange, "market": market }),
    )
}

pub fn symbols(exchange: &str, market: Market) -> Result<Vec<SymbolInfo>> {
    call(
        HostFn::Symbols,
        &json!({ "exchange": exchange, "market": market }),
    )
}

pub fn exchanges() -> Result<Vec<ExchangeInfo>> {
    call(HostFn::Exchanges, &Value::Null)
}

pub fn history_cluster(req: &ClusterRequest) -> Result<ClusterHistory> {
    call(HostFn::HistoryCluster, req)
}

/// The replay chunk shape follows the terminal's cloud replay and is not frozen in ABI v1.
pub fn history_replay(req: &ReplayRequest) -> Result<Value> {
    call(HostFn::HistoryReplay, req)
}

pub fn kv_get<T: DeserializeOwned>(key: &str) -> Result<Option<T>> {
    let value: Value = call(HostFn::KvGet, &json!({ "key": key }))?;
    match value {
        Value::Null => Ok(None),
        v => Ok(Some(serde_json::from_value(v)?)),
    }
}

pub fn kv_set<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<()> {
    call(HostFn::KvSet, &json!({ "key": key, "value": value }))
}

pub fn kv_delete(key: &str) -> Result<()> {
    call(HostFn::KvSet, &json!({ "key": key, "value": Value::Null }))
}

/// Upserts rows by `key`; rows not mentioned stay.
pub fn emit_rows(rows: impl IntoIterator<Item = Row>) -> Result<()> {
    let rows: Vec<Row> = rows.into_iter().collect();
    call(
        HostFn::EmitRows,
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
        HostFn::EmitRows,
        &EmitRows {
            rows: &rows,
            replace: true,
        },
    )
}

pub fn expire<K: Into<String>>(keys: impl IntoIterator<Item = K>) -> Result<()> {
    let keys: Vec<String> = keys.into_iter().map(Into::into).collect();
    call(HostFn::Expire, &json!({ "keys": keys }))
}

pub fn alert(level: AlertLevel, title: impl Into<String>, body: impl Into<String>) -> Result<()> {
    call(
        HostFn::EmitAlert,
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
        HostFn::EmitAlert,
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
        HostFn::SetStatus,
        &json!({ "text": text.into(), "tone": tone }),
    )
}

/// Only valid inside `on_click`; at most one open per click.
pub fn open_market(market: &MarketRef) -> Result<()> {
    call(HostFn::OpenMarket, market)
}

/// Only valid inside `on_click`; at most one open per click.
pub fn open_spread(a: &MarketRef, b: &MarketRef, layout: Option<SpreadLayout>) -> Result<()> {
    call(
        HostFn::OpenSpread,
        &json!({ "a": a, "b": b, "layout": layout }),
    )
}

/// Best effort: a failed log call is dropped.
pub fn log(level: LogLevel, msg: impl Into<String>) {
    let _ = call::<_, ()>(HostFn::Log, &json!({ "level": level, "msg": msg.into() }));
}

pub fn now_ms() -> Result<i64> {
    call(HostFn::NowMs, &Value::Null)
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
    fn outside_terminal_calls_fail_cleanly() {
        assert!(matches!(now_ms(), Err(Error::NotInTerminal)));
    }
}
