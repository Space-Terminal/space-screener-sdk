use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use extism::{CompiledPlugin, Plugin, PluginBuilder, Wasm};
use serde_json::{Value, json};
use space_screener_check::{Lang, Manifest, Report, WasmInfo};

use crate::host::{self, Shared, State};
use crate::output::Output;
use crate::source::Source;

/// CPU budget of `init`, as in the terminal.
pub const INIT_BUDGET: Duration = Duration::from_millis(2000);
const PAGE_BYTES: u32 = 64 * 1024;
/// Exit code Extism reports for a wasm trap.
const TRAP_CODE: i32 = 134;
const RUNAWAY_FACTOR: u32 = 5;
const MIN_RUNAWAY: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("{0}")]
    Invalid(Report),
    #[error("failed to load: {0}")]
    Load(String),
    #[error("harness state is poisoned")]
    Poisoned,
}

/// How one plugin call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    Done {
        cpu: Duration,
    },
    /// The plugin returned an error (`FnResult::Err`); the instance keeps running.
    Failed {
        message: String,
    },
    /// A wasm trap (panic, out of bounds, out of memory); the instance must be recreated.
    Trapped {
        message: String,
    },
    /// Finished, but took more CPU than its budget.
    OverBudget {
        cpu: Duration,
        budget: Duration,
    },
    /// Ran out of CPU budget and was stopped at a host function call.
    Cut {
        budget: Duration,
    },
    /// Ran `max(5 × budget, 5 s)` without returning (an endless loop without host calls).
    TimedOut {
        limit: Duration,
    },
    /// The export is missing (`on_click` is optional).
    NoExport,
}

impl CallOutcome {
    /// The instance is unusable after this call and has to be recreated.
    pub fn needs_restart(&self) -> bool {
        matches!(
            self,
            Self::Trapped { .. } | Self::Cut { .. } | Self::TimedOut { .. }
        )
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::NoExport)
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Done { cpu } => format!("done in {} ms of CPU", cpu.as_millis()),
            Self::Failed { message } => format!("returned an error: {message}"),
            Self::Trapped { message } => format!("trapped: {message}"),
            Self::OverBudget { cpu, budget } => format!(
                "used {} ms of CPU, the limit is {} ms",
                cpu.as_millis(),
                budget.as_millis()
            ),
            Self::Cut { budget } => format!(
                "spent its {} ms CPU budget and was cut at a host call",
                budget.as_millis()
            ),
            Self::TimedOut { limit } => {
                format!("ran {} s without returning", limit.as_secs())
            }
            Self::NoExport => "the export is missing".to_string(),
        }
    }
}

fn runaway_limit(budget: Duration) -> Duration {
    (budget * RUNAWAY_FACTOR).max(MIN_RUNAWAY)
}

/// One plugin instance with the ABI v1 host functions and the terminal's limits, driven by
/// the caller with a virtual clock. Not thread-bound: any thread may drive it, one at a time.
pub struct Harness {
    compiled: CompiledPlugin,
    plugin: Plugin,
    shared: Shared,
    manifest: Arc<Manifest>,
    info: WasmInfo,
}

impl Harness {
    /// Moderates the module ([`space_screener_check::inspect`]) and instantiates it.
    pub fn new(
        manifest: &Manifest,
        wasm: &[u8],
        source: Box<dyn Source>,
    ) -> Result<Self, HarnessError> {
        let info = space_screener_check::inspect(wasm).map_err(HarnessError::Invalid)?;
        let manifest = Arc::new(manifest.clone());
        let shared: Shared = Shared::new(State::new(manifest.clone(), source));
        let pages = manifest.limits.memory_mb * (1024 * 1024 / PAGE_BYTES);
        let timeout = runaway_limit(manifest.limits.cpu_per_call.max(INIT_BUDGET));
        let extism_manifest = extism::Manifest::new([Wasm::data(wasm.to_vec())])
            .with_memory_max(pages)
            .with_timeout(timeout)
            .disallow_all_hosts();
        let compiled = PluginBuilder::new(extism_manifest)
            .with_wasi(false)
            .with_functions(host::functions(&shared))
            .with_cache_disabled()
            .compile()
            .map_err(|e| HarnessError::Load(format!("{e:#}")))?;
        let plugin = Plugin::new_from_compiled(&compiled)
            .map_err(|e| HarnessError::Load(format!("{e:#}")))?;
        Ok(Self {
            compiled,
            plugin,
            shared,
            manifest,
            info,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn wasm_info(&self) -> &WasmInfo {
        &self.info
    }

    /// A fresh instance after a trap; rows, kv, logs and stats are kept, as in the terminal.
    pub fn restart(&mut self) -> Result<(), HarnessError> {
        self.plugin = Plugin::new_from_compiled(&self.compiled)
            .map_err(|e| HarnessError::Load(format!("{e:#}")))?;
        Ok(())
    }

    /// The plugin's kv before `init` (kv survives restarts in the terminal).
    pub fn seed_kv(&mut self, kv: BTreeMap<String, Value>) -> Result<(), HarnessError> {
        self.with_state(|state| state.seed_kv(kv))
    }

    /// Adds a warning to the output (e.g. a truncated recording).
    pub fn warn(&mut self, message: impl Into<String>) -> Result<(), HarnessError> {
        let message = message.into();
        self.with_state(|state| state.warn(message))
    }

    /// `init` with the input the terminal would build.
    pub fn init(
        &mut self,
        params: &BTreeMap<String, Value>,
        terminal: &str,
        lang: Lang,
        now_ms: i64,
    ) -> Result<CallOutcome, HarnessError> {
        let input =
            json!({ "params": params, "terminal": terminal, "lang": lang, "now_ms": now_ms });
        self.init_with(&input, now_ms)
    }

    /// `init` with a recorded input.
    pub fn init_with(&mut self, input: &Value, now_ms: i64) -> Result<CallOutcome, HarnessError> {
        self.call("init", input, now_ms, INIT_BUDGET)
    }

    pub fn timer(&mut self, now_ms: i64) -> Result<CallOutcome, HarnessError> {
        self.timer_with(&json!({ "now_ms": now_ms }), now_ms)
    }

    pub fn timer_with(&mut self, input: &Value, now_ms: i64) -> Result<CallOutcome, HarnessError> {
        let budget = self.manifest.limits.cpu_per_call;
        self.call("on_timer", input, now_ms, budget)
    }

    /// `on_click`; without the export the terminal opens the row's market itself, so this is
    /// [`CallOutcome::NoExport`].
    pub fn click(&mut self, input: &Value, now_ms: i64) -> Result<CallOutcome, HarnessError> {
        if !self.info.on_click {
            return Ok(CallOutcome::NoExport);
        }
        let budget = self.manifest.limits.cpu_per_call;
        self.with_state(|state| state.click = Some(false))?;
        let outcome = self.call("on_click", input, now_ms, budget);
        self.with_state(|state| state.click = None)?;
        outcome
    }

    pub fn output(&self) -> Result<Output, HarnessError> {
        let state = self.shared.get().map_err(|_| HarnessError::Poisoned)?;
        let state = state.lock().map_err(|_| HarnessError::Poisoned)?;
        Ok(state.output())
    }

    /// Advances the virtual clock without a call (row TTLs are checked against it).
    pub fn set_now(&mut self, now_ms: i64) -> Result<(), HarnessError> {
        self.with_state(|state| state.now_ms = now_ms)
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> Result<T, HarnessError> {
        let state = self.shared.get().map_err(|_| HarnessError::Poisoned)?;
        let mut state = state.lock().map_err(|_| HarnessError::Poisoned)?;
        Ok(f(&mut state))
    }

    fn call(
        &mut self,
        name: &str,
        input: &Value,
        now_ms: i64,
        budget: Duration,
    ) -> Result<CallOutcome, HarnessError> {
        if !self.plugin.function_exists(name) {
            return Ok(CallOutcome::NoExport);
        }
        let bytes = serde_json::to_vec(input).map_err(|e| HarnessError::Load(e.to_string()))?;
        self.with_state(|state| {
            state.now_ms = now_ms;
            state.meter.begin(budget);
        })?;
        let result = self
            .plugin
            .call_get_error_code::<&[u8], &[u8]>(name, &bytes)
            .map(|_| ())
            .map_err(|(error, code)| (format!("{error:#}"), code));
        self.with_state(|state| {
            let cpu = state.meter.spent();
            state.stats.cpu_ms_max = state
                .stats
                .cpu_ms_max
                .max(u64::try_from(cpu.as_millis()).unwrap_or(u64::MAX));
            let outcome = match result {
                Ok(()) if cpu > budget => CallOutcome::OverBudget { cpu, budget },
                Ok(()) => CallOutcome::Done { cpu },
                Err(_) if state.meter.cut => CallOutcome::Cut { budget },
                Err((message, code)) => match (code, message.as_str()) {
                    (TRAP_CODE, "timeout") => CallOutcome::TimedOut {
                        limit: runaway_limit(self.manifest.limits.cpu_per_call.max(INIT_BUDGET)),
                    },
                    (TRAP_CODE, _) | (_, "oom") => CallOutcome::Trapped { message },
                    _ => CallOutcome::Failed { message },
                },
            };
            match &outcome {
                CallOutcome::Done { .. } | CallOutcome::NoExport => {}
                CallOutcome::Failed { message } => {
                    state.stats.failures += 1;
                    state.host_log("error", format!("{name} failed: {message}"));
                }
                CallOutcome::Trapped { .. } | CallOutcome::TimedOut { .. } => {
                    state.stats.traps += 1;
                    state.host_log("error", format!("{name} {}", outcome.describe()));
                }
                CallOutcome::OverBudget { .. } | CallOutcome::Cut { .. } => {
                    state.stats.over_budget += 1;
                    state.host_log("warn", format!("{name} {}", outcome.describe()));
                }
            }
            outcome
        })
    }
}
