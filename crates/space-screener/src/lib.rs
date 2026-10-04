//! Rust PDK for Space Terminal screener plugins.
//!
//! A screener is a `wasm32-unknown-unknown` module run by the terminal in an Extism sandbox.
//! It has no sockets, files or clock of its own: data, HTTP, time and output all go through
//! the host functions in [`host`]. See `ABI.md` in the SDK repository for the wire contract.

pub mod abi;
mod errors;
mod export;
pub mod host;
pub mod oi;
mod params;
mod row;
mod series;
mod signals;

pub use abi::{
    AlertLevel, Click, ClickRow, ClusterCell, ClusterHistory, ClusterRequest, ExchangeInfo,
    ExchangeMarket, HttpRequest, HttpResponse, Lang, LogLevel, Market, MarketRef, Method,
    Modifiers, MouseButton, ReplayRequest, SmartLevel, SpreadLayout, StatusTone, SymbolInfo,
    Ticker, TickerSnapshot,
};
pub use errors::{BoxError, Error, Result};
#[doc(hidden)]
pub use export::__private;
pub use export::{Init, Screener, ScreenerResult, lang, params, terminal_version};
pub use params::Params;
pub use row::{Cell, CellValue, Row, Tone};
pub use series::{Series, hours, mins, secs};
pub use signals::{
    ActivitySignal, ActivityTag, BookSide, DensityEvent, DensitySignal, DensityStatus, FeedUpdate,
    PrintsSignal, Signal, SignalFeed, SignalSource, SignalsDelta, TradeSide,
};

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::host::log($crate::LogLevel::Debug, format!($($arg)*)) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::host::log($crate::LogLevel::Info, format!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::host::log($crate::LogLevel::Warn, format!($($arg)*)) };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::host::log($crate::LogLevel::Error, format!($($arg)*)) };
}

pub mod prelude {
    pub use crate::host::{
        alert, alert_row, emit_rows, exchanges, expire, history_cluster, history_replay, http,
        http_batch, kv_delete, kv_get, kv_set, now_ms, open_market, open_market_with_level,
        open_markets, open_spread, replace_rows, set_status, signals, symbols, tickers,
    };
    pub use crate::{
        AlertLevel, BoxError, Cell, Click, ExchangeMarket, HttpRequest, HttpResponse, Init, Lang,
        Market, MarketRef, Params, Row, Screener, ScreenerResult, Series, Signal, SignalFeed,
        SignalSource, SmartLevel, SpreadLayout, StatusTone, Tone, debug, error, export_screener,
        hours, info, lang, mins, params, secs, warn,
    };
}
