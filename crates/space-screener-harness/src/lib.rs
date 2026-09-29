//! Headless host for Space Terminal screener plugins.
//!
//! Runs a `screener.wasm` outside the terminal with the ABI v1 host functions and the
//! terminal's limits (memory from the manifest, CPU per call, a runaway guard) on a virtual
//! clock. Local host functions (kv, rows, alerts, status, logs, clicks) run for real; data
//! comes from a [`Source`]: a [`Replay`] of a recording made by `st record`, or the
//! network-free [`Offline`] set. `st test` replays recordings with [`replay`]; the Space
//! Market registry runs [`trial`] before a moderator looks at a new version.

mod harness;
mod host;
pub mod output;
mod run;
pub mod source;

pub use harness::{CallOutcome, Harness, HarnessError, INIT_BUDGET};
pub use output::{Alert, Intent, LogLine, Output, Row, Stats, Status};
pub use run::{TrialReport, Verdict, replay, trial};
pub use source::{Offline, Replay, Source};
