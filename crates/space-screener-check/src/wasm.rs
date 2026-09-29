//! Moderation of `screener.wasm` before it is compiled: a plugin cannot do anything that is
//! not in its import list, so moderation is an allowlist of imports.

use wasmparser::{ExternalKind, Parser, Payload, Validator};

use crate::report::{Code, Report};

pub const FILE: &str = "screener.wasm";
pub const MAX_WASM_BYTES: usize = 10 * 1024 * 1024;

pub const ENV_MODULE: &str = "extism:host/env";
pub const USER_MODULE: &str = "extism:host/user";

/// Extism kernel functions the PDK needs: memory, call input/output, config and vars.
/// `http_*` and `log_*` are not here — network and logs go through the terminal's host
/// functions only.
pub const ENV_IMPORTS: &[&str] = &[
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

/// Host functions of ABI v1 (`extism:host/user`).
pub const HOST_FUNCTIONS: &[&str] = &[
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

/// Code that wasmtime/Extism run at instantiation, before the host has a cancel handle: an
/// endless loop there cannot be interrupted. Refused under any export kind.
pub const FORBIDDEN_EXPORTS: &[&str] = &["_start", "_initialize", "__wasm_call_ctors", "hs_init"];

/// Entry points exported as functions. Only the ones the host acts on are named.
pub const ENTRY_POINTS: &[&str] = &["init", "on_timer", "on_click", "on_params", "on_batch"];

/// What the host needs to know about a module that passed moderation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasmInfo {
    pub on_click: bool,
    pub on_params: bool,
    pub imports: Vec<(String, String)>,
    /// Function exports only.
    pub exports: Vec<String>,
}

pub fn is_host_function(name: &str) -> bool {
    HOST_FUNCTIONS.contains(&name)
}

/// Checks size, validity, imports and exports. The report lists every problem found.
pub fn inspect(bytes: &[u8]) -> Result<WasmInfo, Report> {
    let mut r = Report::default();
    if bytes.len() > MAX_WASM_BYTES {
        r.error(
            Code::TooLarge,
            "",
            format!(
                "{FILE} is {} bytes, the limit is {MAX_WASM_BYTES}",
                bytes.len()
            ),
        );
        return Err(r);
    }
    if let Err(e) = Validator::new().validate_all(bytes) {
        r.error(
            Code::InvalidWasm,
            "",
            format!("module does not validate: {e}"),
        );
        return Err(r);
    }

    let mut info = WasmInfo::default();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = match payload {
            Ok(payload) => payload,
            Err(e) => {
                r.error(Code::InvalidWasm, "", e.to_string());
                return Err(r);
            }
        };
        match payload {
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = match import {
                        Ok(import) => import,
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    };
                    let allowed = match import.module {
                        ENV_MODULE => ENV_IMPORTS.contains(&import.name),
                        USER_MODULE => is_host_function(import.name),
                        _ => false,
                    };
                    if !allowed {
                        r.error(
                            Code::ForbiddenImport,
                            format!("import {}::{}", import.module, import.name),
                            format!(
                                "{}::{} is not available to screeners{}",
                                import.module,
                                import.name,
                                hint(import.module, import.name)
                            ),
                        );
                    }
                    info.imports
                        .push((import.module.to_string(), import.name.to_string()));
                }
            }
            Payload::StartSection { .. } => r.error(
                Code::ForbiddenExport,
                "start",
                "the module has a start section (code that runs at instantiation)",
            ),
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = match export {
                        Ok(export) => export,
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    };
                    if FORBIDDEN_EXPORTS.contains(&export.name) {
                        r.error(
                            Code::ForbiddenExport,
                            format!("export {}", export.name),
                            format!(
                                "`{}` is a WASI/C runtime entry point; build a cdylib for wasm32-unknown-unknown",
                                export.name
                            ),
                        );
                    }
                    if export.kind == ExternalKind::Func {
                        info.exports.push(export.name.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    for required in REQUIRED_EXPORTS {
        if !info.exports.iter().any(|export| export == required) {
            r.error(
                Code::InvalidWasm,
                format!("export {required}"),
                format!(
                    "missing function export `{required}` — did you call space_screener::export_screener!(YourType)?"
                ),
            );
        }
    }
    if !r.is_ok() {
        return Err(r);
    }
    info.on_click = info.exports.iter().any(|export| export == "on_click");
    info.on_params = info.exports.iter().any(|export| export == "on_params");
    Ok(info)
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
            " (Rust: build for wasm32-unknown-unknown, not wasip1 — std::time, std::fs and threads are unavailable; TypeScript: build with `st build`, which links stubs for js-pdk's WASI imports)"
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

    const ENTRY: &str = r#"(func (export "init") (result i32) i32.const 0) (func (export "on_timer") (result i32) i32.const 0)"#;

    fn module(body: &str) -> Vec<u8> {
        wat::parse_str(format!("(module {body})")).unwrap()
    }

    fn report(body: &str) -> String {
        inspect(&module(body)).unwrap_err().to_string()
    }

    fn first_code(body: &str) -> &'static str {
        inspect(&module(body)).unwrap_err().errors[0].code.as_str()
    }

    #[test]
    fn allows_kernel_and_host_imports_only() {
        let info = inspect(&module(&format!(
            r#"(import "extism:host/env" "alloc" (func (param i64) (result i64)))
               (import "extism:host/user" "http" (func (param i64) (result i64)))
               {ENTRY} (func (export "on_click") (result i32) i32.const 0)"#
        )))
        .unwrap();
        assert!(info.on_click);
        assert!(!info.on_params);
        assert_eq!(info.imports.len(), 2);

        let forbidden = |module_name: &str, name: &str| {
            first_code(&format!(
                r#"(import "{module_name}" "{name}" (func (param i64) (result i64))) {ENTRY}"#
            ))
        };
        assert_eq!(
            forbidden("extism:host/env", "http_request"),
            "forbidden_import"
        );
        assert_eq!(forbidden("extism:host/env", "log_info"), "forbidden_import");
        assert_eq!(
            forbidden("extism:host/user", "place_order"),
            "forbidden_import"
        );
        assert_eq!(
            forbidden("wasi_snapshot_preview1", "fd_write"),
            "forbidden_import"
        );
        assert_eq!(
            inspect(b"not wasm").unwrap_err().errors[0].code,
            Code::InvalidWasm
        );
    }

    #[test]
    fn every_problem_is_reported_with_hints() {
        let problems = report(
            r#"(import "extism:host/env" "http_request" (func (param i64 i64) (result i64)))
               (import "extism:host/env" "log_info" (func (param i64)))
               (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
               (import "extism:host/user" "place_order" (func (param i64) (result i64)))
               (func (export "init") (result i32) i32.const 0)"#,
        );
        for needle in [
            "extism:host/env::http_request",
            "extism:host/env::log_info",
            "info!/warn!",
            "wasi_snapshot_preview1::fd_write",
            "wasm32-unknown-unknown",
            "extism:host/user::place_order",
            "missing function export `on_timer`",
        ] {
            assert!(
                problems.contains(needle),
                "missing `{needle}` in:\n{problems}"
            );
        }
    }

    #[test]
    fn start_section_and_runtime_entry_points_are_refused() {
        assert_eq!(
            first_code(&format!(
                r#"(func $spin (loop $l (br $l))) (start $spin) {ENTRY}"#
            )),
            "forbidden_export"
        );
        let problems = report(&format!(
            r#"{ENTRY} (func (export "_initialize")) (func (export "__wasm_call_ctors"))"#
        ));
        assert!(problems.contains("export _initialize"), "{problems}");
        assert!(problems.contains("export __wasm_call_ctors"), "{problems}");
    }

    #[test]
    fn forbidden_names_are_refused_under_any_export_kind() {
        assert_eq!(
            first_code(&format!(
                r#"{ENTRY} (global (export "_start") i32 (i32.const 0))"#
            )),
            "forbidden_export"
        );
        assert_eq!(
            first_code(&format!(r#"{ENTRY} (memory (export "_initialize") 1)"#)),
            "forbidden_export"
        );
    }

    #[test]
    fn required_exports_must_be_functions() {
        let problems = report(
            r#"(global (export "init") i32 (i32.const 0))
               (func (export "on_timer") (result i32) i32.const 0)"#,
        );
        assert!(
            problems.contains("missing function export `init`"),
            "{problems}"
        );
    }

    #[test]
    fn oversized_module_is_too_large() {
        let big = vec![0u8; MAX_WASM_BYTES + 1];
        assert_eq!(inspect(&big).unwrap_err().errors[0].code, Code::TooLarge);
    }
}
