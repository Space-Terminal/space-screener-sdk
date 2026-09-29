use std::sync::{Mutex, MutexGuard};

use crate::abi::{Click, Lang};
use crate::errors::BoxError;
use crate::host;
use crate::params::Params;

pub type ScreenerResult = std::result::Result<(), BoxError>;

pub struct Init {
    pub params: Params,
    pub terminal: String,
    pub lang: Lang,
    pub now_ms: i64,
}

/// A screener is a pure function of host data and timer ticks to table rows.
///
/// Export it with [`export_screener!`](crate::export_screener). The terminal calls
/// `init` once per (re)start, `on_timer` every `timer_ms` after the previous call
/// returned and `on_params` when the user changes parameters. `params()` always
/// returns the current parameters.
///
/// Clicks: with `export_screener!(T)` the terminal opens the clicked row's market
/// itself, immediately. Only `export_screener!(T, on_click)` routes clicks to
/// [`Screener::on_click`]; such clicks wait while `on_timer` runs, so keep rounds short.
pub trait Screener: Default + Send + 'static {
    fn init(&mut self, _init: &Init) -> ScreenerResult {
        Ok(())
    }

    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult;

    /// Called only when exported with `export_screener!(T, on_click)`.
    /// Default: open the clicked row's market (`symbol`, `exchange`, `market` of the row).
    fn on_click(&mut self, click: &Click) -> ScreenerResult {
        if let Some(market) = click.market_ref() {
            host::open_market(&market)?;
        }
        Ok(())
    }

    fn on_params(&mut self, _params: &Params) -> ScreenerResult {
        Ok(())
    }
}

struct Context {
    params: Params,
    terminal: String,
    lang: Lang,
}

static CONTEXT: Mutex<Option<Context>> = Mutex::new(None);

fn context() -> MutexGuard<'static, Option<Context>> {
    CONTEXT.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn params() -> Params {
    context()
        .as_ref()
        .map(|c| c.params.clone())
        .unwrap_or_default()
}

pub fn lang() -> Lang {
    context().as_ref().map(|c| c.lang).unwrap_or_default()
}

pub fn terminal_version() -> String {
    context()
        .as_ref()
        .map(|c| c.terminal.clone())
        .unwrap_or_default()
}

#[doc(hidden)]
pub mod __private {
    use std::sync::{Mutex, MutexGuard};

    use super::{CONTEXT, Context, Init, Screener, context};
    use crate::abi::{Click, InitInput, ParamsInput, TimerInput};
    use crate::params::Params;

    pub use std::sync::Mutex as StateMutex;

    type Outcome = std::result::Result<(), String>;

    fn lock<T>(state: &Mutex<Option<T>>) -> MutexGuard<'_, Option<T>> {
        state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn parse<I: serde::de::DeserializeOwned>(input: &[u8]) -> std::result::Result<I, String> {
        serde_json::from_slice(input).map_err(|e| format!("bad input: {e}"))
    }

    fn with_screener<T: Screener>(
        state: &Mutex<Option<T>>,
        f: impl FnOnce(&mut T) -> super::ScreenerResult,
    ) -> Outcome {
        let mut guard = lock(state);
        let screener = guard.as_mut().ok_or("init was not called")?;
        f(screener).map_err(|e| e.to_string())
    }

    pub fn init<T: Screener>(state: &Mutex<Option<T>>, input: &[u8]) -> Outcome {
        let input: InitInput = parse(input)?;
        let init = Init {
            params: Params::new(input.params),
            terminal: input.terminal,
            lang: input.lang,
            now_ms: input.now_ms,
        };
        *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Context {
            params: init.params.clone(),
            terminal: init.terminal.clone(),
            lang: init.lang,
        });
        let mut screener = T::default();
        screener.init(&init).map_err(|e| e.to_string())?;
        *lock(state) = Some(screener);
        Ok(())
    }

    pub fn on_timer<T: Screener>(state: &Mutex<Option<T>>, input: &[u8]) -> Outcome {
        let input: TimerInput = parse(input)?;
        with_screener(state, |s| s.on_timer(input.now_ms))
    }

    pub fn on_click<T: Screener>(state: &Mutex<Option<T>>, input: &[u8]) -> Outcome {
        let click: Click = parse(input)?;
        with_screener(state, |s| s.on_click(&click))
    }

    pub fn on_params<T: Screener>(state: &Mutex<Option<T>>, input: &[u8]) -> Outcome {
        let input: ParamsInput = parse(input)?;
        let params = Params::new(input.params);
        if let Some(ctx) = context().as_mut() {
            ctx.params = params.clone();
        }
        with_screener(state, |s| s.on_params(&params))
    }

    // A panic traps the instance; the hook sends message and location to the plugin log
    // first, so `st logs` shows why the terminal restarted the plugin.
    #[cfg(target_arch = "wasm32")]
    fn install_panic_hook() {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                crate::host::log(crate::abi::LogLevel::Error, format!("panic: {info}"));
            }));
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(f: impl FnOnce(&[u8]) -> Outcome) -> i32 {
        install_panic_hook();
        let input = extism_pdk::input_bytes();
        match f(&input) {
            Ok(()) => 0,
            Err(msg) => {
                if let Ok(mem) = extism_pdk::Memory::from_bytes(msg.as_bytes()) {
                    // SAFETY: `mem` is a live Extism allocation; the host reads it after we return.
                    unsafe { extism_pdk::extism::error_set(mem.offset()) };
                }
                -1
            }
        }
    }
}

/// Exports a [`Screener`] type as the plugin entry points `init`, `on_timer`, `on_params`.
///
/// `export_screener!(MyScreener, on_click)` also exports `on_click`, routing row clicks to
/// [`Screener::on_click`]. Without it the terminal opens the clicked row's market itself,
/// right away; with it clicks wait for a running `on_timer`.
///
/// ```ignore
/// #[derive(Default)]
/// struct MyScreener;
/// impl Screener for MyScreener { /* ... */ }
/// space_screener::export_screener!(MyScreener);
/// ```
#[macro_export]
macro_rules! export_screener {
    (@export $screener:ty, $($on_click:ident)?) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            static STATE: $crate::__private::StateMutex<::core::option::Option<$screener>> =
                $crate::__private::StateMutex::new(::core::option::Option::None);

            #[unsafe(no_mangle)]
            pub extern "C" fn init() -> i32 {
                $crate::__private::run(|input| $crate::__private::init(&STATE, input))
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn on_timer() -> i32 {
                $crate::__private::run(|input| $crate::__private::on_timer(&STATE, input))
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn on_params() -> i32 {
                $crate::__private::run(|input| $crate::__private::on_params(&STATE, input))
            }

            $(
                #[unsafe(no_mangle)]
                pub extern "C" fn $on_click() -> i32 {
                    $crate::__private::run(|input| $crate::__private::on_click(&STATE, input))
                }
            )?
        };
    };
    ($screener:ty) => {
        $crate::export_screener!(@export $screener,);
    };
    ($screener:ty, on_click) => {
        $crate::export_screener!(@export $screener, on_click);
    };
}

#[cfg(test)]
mod tests {
    use super::__private;
    use super::*;

    #[derive(Default)]
    struct Probe {
        threshold: f64,
        ticks: Vec<i64>,
    }

    impl Screener for Probe {
        fn init(&mut self, init: &Init) -> ScreenerResult {
            self.threshold = init.params.f64_or("threshold", 1.0);
            Ok(())
        }

        fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
            if now_ms < 0 {
                return Err("negative time".into());
            }
            self.ticks.push(now_ms);
            Ok(())
        }

        fn on_params(&mut self, params: &Params) -> ScreenerResult {
            self.threshold = params.f64_or("threshold", self.threshold);
            Ok(())
        }
    }

    #[test]
    fn lifecycle_through_private_entry_points() {
        let state = Mutex::new(None::<Probe>);
        assert_eq!(
            __private::on_timer(&state, br#"{"now_ms":1}"#),
            Err("init was not called".to_string())
        );
        __private::init(
            &state,
            br#"{"params":{"threshold":5},"terminal":"0.104.70","lang":"ru","now_ms":0}"#,
        )
        .unwrap();
        assert_eq!(lang(), Lang::Ru);
        assert_eq!(terminal_version(), "0.104.70");
        __private::on_timer(&state, br#"{"now_ms":7}"#).unwrap();
        assert!(__private::on_timer(&state, br#"{"now_ms":-1}"#).is_err());
        __private::on_params(&state, br#"{"params":{"threshold":9}}"#).unwrap();
        assert_eq!(params().f64("threshold"), Some(9.0));
        let guard = state.lock().unwrap();
        let probe = guard.as_ref().unwrap();
        assert_eq!(probe.ticks, vec![7]);
        assert_eq!(probe.threshold, 9.0);
    }
}
