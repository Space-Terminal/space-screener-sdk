//! Moderation of `screener.wasm` before it is compiled: a plugin cannot do anything that is
//! not in its import list, so moderation is an allowlist of imports.

use std::collections::BTreeSet;

use wasmparser::{
    CompositeInnerType, Encoding, ExternalKind, Operator, Parser, Payload, TypeRef, ValType,
    Validator, WasmFeatures,
};

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

/// WebAssembly 2.0 — what Rust (wasm32-unknown-unknown) and the extism-js QuickJS build emit.
/// Left out: GC (objects outside the linear-memory limit), threads, memory64, multi-memory,
/// exceptions and components, which the terminal neither needs nor accounts for.
pub const FEATURES: WasmFeatures = WasmFeatures::WASM2;

/// Tables live outside the linear memory, so the memory limit does not cover them. Real
/// modules have one or two (Rust: 1 table of ~100 functions, QuickJS: 2 tables, ~960 elements
/// in total); the caps leave ×10 and ×50 of headroom.
pub const MAX_TABLES: usize = 20;
pub const MAX_TABLE_ELEMENTS: u64 = 50_000;

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
    if is_component(bytes) {
        r.error(
            Code::InvalidWasm,
            "",
            "screener.wasm is a WebAssembly component; build a core module (a cdylib for wasm32-unknown-unknown)",
        );
        return Err(r);
    }
    if let Err(e) = Validator::new_with_features(FEATURES).validate_all(bytes) {
        r.error(
            Code::InvalidWasm,
            "",
            format!(
                "module does not validate with WebAssembly 2.0 features (no GC, threads, memory64, multi-memory or exceptions): {e}"
            ),
        );
        return Err(r);
    }

    let mut info = WasmInfo::default();
    let mut func_types: Vec<Option<(Vec<ValType>, Vec<ValType>)>> = Vec::new();
    let mut imported_tables = 0u32;
    let mut tables: Vec<(u64, Option<u64>)> = Vec::new();
    let mut grown: BTreeSet<u32> = BTreeSet::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = match payload {
            Ok(payload) => payload,
            Err(e) => {
                r.error(Code::InvalidWasm, "", e.to_string());
                return Err(r);
            }
        };
        match payload {
            Payload::TypeSection(reader) => {
                for group in reader {
                    let group = match group {
                        Ok(group) => group,
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    };
                    for sub in group.into_types() {
                        func_types.push(match sub.composite_type.inner {
                            CompositeInnerType::Func(ty) => {
                                Some((ty.params().to_vec(), ty.results().to_vec()))
                            }
                            _ => None,
                        });
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = match import {
                        Ok(import) => import,
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    };
                    let path = format!("import {}::{}", import.module, import.name);
                    let allowed = match import.module {
                        ENV_MODULE => ENV_IMPORTS.contains(&import.name),
                        USER_MODULE => is_host_function(import.name),
                        _ => false,
                    };
                    if !allowed {
                        r.error(
                            Code::ForbiddenImport,
                            &path,
                            format!(
                                "{}::{} is not available to screeners{}",
                                import.module,
                                import.name,
                                hint(import.module, import.name)
                            ),
                        );
                    }
                    match import.ty {
                        TypeRef::Func(index) => {
                            let host_signature = func_types
                                .get(index as usize)
                                .and_then(Option::as_ref)
                                .is_some_and(|(params, results)| {
                                    params.as_slice() == [ValType::I64]
                                        && results.as_slice() == [ValType::I64]
                                });
                            if allowed && import.module == USER_MODULE && !host_signature {
                                r.error(
                                    Code::ForbiddenImport,
                                    &path,
                                    "host functions take and return one i64 (a memory offset): (func (param i64) (result i64))",
                                );
                            }
                        }
                        other => {
                            if let TypeRef::Table(_) = other {
                                imported_tables += 1;
                            }
                            if allowed {
                                r.error(
                                    Code::ForbiddenImport,
                                    &path,
                                    format!(
                                        "imports a {}; screeners may import functions only",
                                        import_kind(&other)
                                    ),
                                );
                            }
                        }
                    }
                    info.imports
                        .push((import.module.to_string(), import.name.to_string()));
                }
            }
            Payload::TableSection(reader) => {
                for table in reader {
                    match table {
                        Ok(table) => tables.push((table.ty.initial, table.ty.maximum)),
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut operators = match body.get_operators_reader() {
                    Ok(operators) => operators,
                    Err(e) => {
                        r.error(Code::InvalidWasm, "", e.to_string());
                        return Err(r);
                    }
                };
                while !operators.eof() {
                    match operators.read() {
                        Ok(Operator::TableGrow { table }) => {
                            grown.insert(table);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            r.error(Code::InvalidWasm, "", e.to_string());
                            return Err(r);
                        }
                    }
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
    check_tables(&tables, imported_tables, &grown, &mut r);

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

fn is_component(bytes: &[u8]) -> bool {
    matches!(
        Parser::new(0).parse_all(bytes).next(),
        Some(Ok(Payload::Version {
            encoding: Encoding::Component,
            ..
        }))
    )
}

fn import_kind(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Func(_) | TypeRef::FuncExact(_) => "function",
        TypeRef::Table(_) => "table",
        TypeRef::Memory(_) => "memory",
        TypeRef::Global(_) => "global",
        TypeRef::Tag(_) => "tag",
    }
}

/// A table without a maximum stays at its initial size unless code runs `table.grow` on it.
fn check_tables(
    tables: &[(u64, Option<u64>)],
    imported: u32,
    grown: &BTreeSet<u32>,
    r: &mut Report,
) {
    if tables.len() > MAX_TABLES {
        r.error(
            Code::InvalidWasm,
            "tables",
            format!(
                "the module declares {} tables, the limit is {MAX_TABLES}",
                tables.len()
            ),
        );
    }
    let mut total = 0u64;
    for (i, (initial, maximum)) in tables.iter().enumerate() {
        let index = imported + u32::try_from(i).unwrap_or(u32::MAX);
        let bound = match maximum {
            Some(maximum) => *maximum,
            None if !grown.contains(&index) => *initial,
            None => {
                r.error(
                    Code::InvalidWasm,
                    format!("table {index}"),
                    "table.grow on a table without a declared maximum; declare one",
                );
                continue;
            }
        };
        total = total.saturating_add(bound);
    }
    if total > MAX_TABLE_ELEMENTS {
        r.error(
            Code::InvalidWasm,
            "tables",
            format!(
                "tables may hold {total} elements, the limit is {MAX_TABLE_ELEMENTS} (tables are not covered by limits.memory_mb)"
            ),
        );
    }
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
    fn tables_are_capped_because_the_memory_limit_does_not_cover_them() {
        let many_elements: String = (0..10)
            .map(|i| format!("(table $t{i} 10000000 funcref)"))
            .collect();
        let problems = report(&format!("{many_elements} {ENTRY}"));
        assert!(
            problems.contains("tables may hold 100000000 elements"),
            "{problems}"
        );
        let many_tables: String = (0..=MAX_TABLES)
            .map(|i| format!("(table $t{i} 1 1 funcref)"))
            .collect();
        assert!(report(&format!("{many_tables} {ENTRY}")).contains("declares 21 tables"));
        let growable = format!(
            r#"(table $t 1 funcref) {ENTRY}
               (func (export "grow") (result i32) (table.grow $t (ref.null func) (i32.const 1000000000)))"#
        );
        assert!(report(&growable).contains("table.grow on a table without a declared maximum"));
        let bounded = format!(
            r#"(table $t 1 {MAX_TABLE_ELEMENTS} funcref) {ENTRY}
               (func (export "grow") (result i32) (table.grow $t (ref.null func) (i32.const 10)))"#
        );
        assert!(inspect(&module(&bounded)).is_ok());
        // What the extism-js QuickJS build declares: a fixed table and one without a maximum
        // that no code grows.
        assert!(
            inspect(&module(&format!(
                "(table 943 943 funcref) (table 17 funcref) {ENTRY}"
            )))
            .is_ok()
        );
    }

    #[test]
    fn only_webassembly_2_modules_are_accepted() {
        let component = wat::parse_str(
            r#"(component (core module
                 (func (export "init") (result i32) i32.const 0)
                 (func (export "on_timer") (result i32) i32.const 0)))"#,
        )
        .unwrap();
        let problems = inspect(&component).unwrap_err().to_string();
        assert!(
            problems.contains("is a WebAssembly component"),
            "{problems}"
        );
        let gc = report(&format!(
            r#"(type $a (array (mut i64))) {ENTRY}
               (func (export "big") (result i32) (drop (array.new $a (i64.const 1) (i32.const 100000000))) i32.const 0)"#
        ));
        assert!(gc.starts_with("invalid_wasm"), "{gc}");
        let shared = report(&format!("(memory 1 1 shared) {ENTRY}"));
        assert!(shared.starts_with("invalid_wasm"), "{shared}");
    }

    #[test]
    fn only_functions_may_be_imported() {
        for import in [
            r#"(import "extism:host/env" "alloc" (memory 1))"#,
            r#"(import "extism:host/env" "free" (table 1 funcref))"#,
            r#"(import "extism:host/user" "http" (global i64))"#,
        ] {
            let problems = report(&format!("{import} {ENTRY}"));
            assert!(
                problems.contains("screeners may import functions only"),
                "{problems}"
            );
        }
    }

    #[test]
    fn host_functions_must_have_the_abi_signature() {
        let problems = report(&format!(
            r#"(import "extism:host/user" "http" (func (param i32) (result i32))) {ENTRY}"#
        ));
        assert!(problems.contains("take and return one i64"), "{problems}");
        assert!(
            inspect(&module(&format!(
                r#"(import "extism:host/user" "http" (func (param i64) (result i64))) {ENTRY}"#
            )))
            .is_ok()
        );
    }

    #[test]
    fn oversized_module_is_too_large() {
        let big = vec![0u8; MAX_WASM_BYTES + 1];
        assert_eq!(inspect(&big).unwrap_err().errors[0].code, Code::TooLarge);
    }
}
