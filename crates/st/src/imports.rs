use anyhow::{Context, Result};
use wasmparser::{Parser, Payload, Validator};

pub const ENV_MODULE: &str = "extism:host/env";
pub const USER_MODULE: &str = "extism:host/user";

/// Extism kernel functions the PDK needs for memory, input/output, config and vars.
pub const ENV_ALLOWED: &[&str] = &[
    "alloc",
    "free",
    "length",
    "length_unsafe",
    "load_u8",
    "load_u64",
    "store_u8",
    "store_u64",
    "input_length",
    "input_load_u8",
    "input_load_u64",
    "output_set",
    "error_set",
    "config_get",
    "var_get",
    "var_set",
];

/// Host functions of ABI v1.
pub const USER_ALLOWED: &[&str] = &[
    "http",
    "http_batch",
    "tickers",
    "symbols",
    "exchanges",
    "history_cluster",
    "history_replay",
    "kv_get",
    "kv_set",
    "emit_rows",
    "expire",
    "emit_alert",
    "set_status",
    "open_market",
    "open_spread",
    "log",
    "now_ms",
];

pub const REQUIRED_EXPORTS: &[&str] = &["init", "on_timer"];
pub const MAX_WASM_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct Module {
    pub imports: Vec<(String, String)>,
    pub exports: Vec<String>,
}

pub fn inspect(wasm: &[u8]) -> Result<Module> {
    Validator::new()
        .validate_all(wasm)
        .context("invalid_wasm: module does not validate")?;
    let mut module = Module::default();
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.context("invalid_wasm")? {
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.context("invalid_wasm")?;
                    module
                        .imports
                        .push((import.module.to_string(), import.name.to_string()));
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    module
                        .exports
                        .push(export.context("invalid_wasm")?.name.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(module)
}

/// Same rules as the terminal applies on install. Returns human-readable problems.
pub fn check(wasm: &[u8], module: &Module) -> Vec<String> {
    let mut problems = Vec::new();
    if wasm.len() > MAX_WASM_BYTES {
        problems.push(format!(
            "too_large: screener.wasm is {} bytes, the limit is {MAX_WASM_BYTES}",
            wasm.len()
        ));
    }
    for (module_name, name) in &module.imports {
        let allowed = match module_name.as_str() {
            ENV_MODULE => ENV_ALLOWED.contains(&name.as_str()),
            USER_MODULE => USER_ALLOWED.contains(&name.as_str()),
            _ => false,
        };
        if !allowed {
            problems.push(format!(
                "forbidden_import: {module_name}::{name}{}",
                hint(module_name, name)
            ));
        }
    }
    for required in REQUIRED_EXPORTS {
        if !module.exports.iter().any(|e| e == required) {
            problems.push(format!(
                "invalid_wasm: missing export `{required}` — did you call space_screener::export_screener!(YourType)?"
            ));
        }
    }
    problems
}

fn hint(module: &str, name: &str) -> &'static str {
    match (module, name) {
        (ENV_MODULE, n) if n.starts_with("log_") || n == "get_log_level" => {
            " (use space_screener::info!/warn! instead of extism_pdk logging)"
        }
        (ENV_MODULE, n) if n.starts_with("http_") => {
            " (use space_screener::host::http; enable extism-pdk with default-features = false)"
        }
        (m, _) if m.starts_with("wasi") => {
            " (build for wasm32-unknown-unknown, not wasip1; std::time, std::fs and threads are unavailable)"
        }
        ("__wbindgen_placeholder__", _) | ("wbg", _) => {
            " (wasm-bindgen crates such as getrandom/js or chrono/wasmbind do not run in the terminal)"
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(wat: &str) -> (Vec<u8>, Module) {
        let wasm = wat::parse_str(wat).unwrap();
        let module = inspect(&wasm).unwrap();
        (wasm, module)
    }

    #[test]
    fn allowed_imports_and_exports_pass() {
        let (wasm, m) = module(
            r#"(module
                (import "extism:host/env" "alloc" (func (param i64) (result i64)))
                (import "extism:host/user" "http" (func (param i64) (result i64)))
                (func (export "init") (result i32) i32.const 0)
                (func (export "on_timer") (result i32) i32.const 0))"#,
        );
        assert!(check(&wasm, &m).is_empty());
    }

    #[test]
    fn forbidden_imports_and_missing_exports_are_reported() {
        let (wasm, m) = module(
            r#"(module
                (import "extism:host/env" "http_request" (func (param i64 i64) (result i64)))
                (import "extism:host/env" "log_info" (func (param i64)))
                (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
                (import "extism:host/user" "place_order" (func (param i64) (result i64)))
                (func (export "init") (result i32) i32.const 0))"#,
        );
        let problems = check(&wasm, &m).join("\n");
        for needle in [
            "extism:host/env::http_request",
            "extism:host/env::log_info",
            "wasi_snapshot_preview1::fd_write",
            "extism:host/user::place_order",
            "missing export `on_timer`",
        ] {
            assert!(
                problems.contains(needle),
                "missing `{needle}` in:\n{problems}"
            );
        }
    }
}
