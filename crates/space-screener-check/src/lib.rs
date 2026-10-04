//! Shared rules for Space Terminal screener plugins.
//!
//! One source of truth for what a plugin may be: the terminal checks a plugin with it on
//! install, `st` before building and publishing, and the Space Market registry on publish.
//! See `ABI.md` in the SDK repository for the contract these rules implement.

pub mod manifest;
pub mod params;
pub mod recording;
pub mod registry;
pub mod release;
mod report;
pub mod wasm;

pub use manifest::{
    ABI_1_1_TERMINAL, ABI_VERSION, Column, ColumnType, HistoryKind, Lang, Limits, Localized,
    Manifest, PluginLang, SignalSource, SortDir, TerminalTooOld, is_valid_id, normalize_host,
};
pub use params::{Param, ParamError, ParamProblem, ParamType, decimal_from_number};
pub use recording::Recording;
pub use registry::RegistryInfo;
pub use release::Release;
pub use report::{Code, Issue, Report};
pub use wasm::{
    ENV_IMPORTS, HOST_FUNCTIONS, HOST_FUNCTIONS_SINCE, MAX_WASM_BYTES, WasmInfo, check_imports,
    host_functions_for, inspect,
};
