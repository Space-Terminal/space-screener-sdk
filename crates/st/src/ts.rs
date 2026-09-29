//! TypeScript screeners: esbuild bundles `src/index.ts` with the TS PDK, extism-js (QuickJS)
//! compiles the bundle, then binaryen links in stubs for the WASI and Extism imports js-pdk
//! brings along and strips the extra exports, so the module passes the same checks as Rust.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use space_screener_check::HOST_FUNCTIONS;

use crate::toolchain::{self, Tools};

pub const PDK_FILE: &str = "pdk/index.ts";
pub const SOURCE: &str = "src/index.ts";
pub const PDK: &str = include_str!("../ts/pdk/index.ts");
const PDK_IMPORT: &str = "@space-terminal/screener";
const OUT_DIR: &str = "dist";
const WASI_SHIM: &str = include_str!("../ts/shims/wasi.wat");
const ENV_SHIM: &str = include_str!("../ts/shims/env.wat");
/// Exports a TypeScript screener may have; anything else would shift js-pdk's export table.
const ENTRY_POINTS: &[&str] = &["init", "on_timer", "on_click", "on_params"];
const REQUIRED: &[&str] = &["init", "on_timer"];
const WASM_FEATURES: &[&str] = &["--enable-reference-types", "--enable-bulk-memory"];

fn node_tool(dir: &Path, name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    };
    dir.join("node_modules").join(".bin").join(file)
}

fn run(command: &mut Command, what: &str) -> Result<Output> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("cannot run {what}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("{what} failed:\n{}{}", stdout.trim_end(), stderr.trim_end());
    }
    Ok(output)
}

fn ensure_node_modules(dir: &Path) -> Result<()> {
    if node_tool(dir, "esbuild").is_file() {
        return Ok(());
    }
    if !dir.join("package.json").is_file() {
        bail!(
            "{} has no package.json; create TypeScript projects with `st init --lang ts`",
            dir.display()
        );
    }
    eprintln!("installing esbuild and typescript (npm install, one time)…");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let status = Command::new(npm)
        .arg("install")
        .current_dir(dir)
        .status()
        .with_context(|| "cannot run npm; install Node.js 18 or newer (https://nodejs.org)")?;
    if !status.success() || !node_tool(dir, "esbuild").is_file() {
        bail!("npm install did not provide esbuild; check package.json");
    }
    Ok(())
}

fn type_check(dir: &Path) -> Result<()> {
    let tsc = node_tool(dir, "tsc");
    if !tsc.is_file() {
        return Ok(());
    }
    let output = Command::new(&tsc)
        .args(["--noEmit", "-p", "."])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .context("cannot run tsc")?;
    if !output.status.success() {
        bail!(
            "type errors:\n{}",
            String::from_utf8_lossy(&output.stdout).trim_end()
        );
    }
    Ok(())
}

#[derive(Deserialize)]
struct Metafile {
    outputs: std::collections::BTreeMap<String, MetaOutput>,
}

#[derive(Deserialize)]
struct MetaOutput {
    #[serde(default)]
    exports: Vec<String>,
}

fn esbuild(dir: &Path, args: &[&str]) -> Result<()> {
    let alias = format!("--alias:{PDK_IMPORT}=./{PDK_FILE}");
    run(
        Command::new(node_tool(dir, "esbuild"))
            .args(args)
            .args(["--bundle", "--log-level=warning", alias.as_str()])
            .current_dir(dir),
        "esbuild",
    )?;
    Ok(())
}

/// The functions `src/index.ts` exports, as esbuild sees them.
fn exports(dir: &Path) -> Result<Vec<String>> {
    let meta_path = format!("{OUT_DIR}/meta.json");
    esbuild(
        dir,
        &[
            SOURCE,
            "--format=esm",
            &format!("--metafile={meta_path}"),
            &format!("--outfile={OUT_DIR}/probe.js"),
        ],
    )?;
    let meta: Metafile = serde_json::from_slice(
        &std::fs::read(dir.join(&meta_path)).context("esbuild wrote no metafile")?,
    )
    .context("cannot parse the esbuild metafile")?;
    let exports = meta
        .outputs
        .into_values()
        .next()
        .map(|o| o.exports)
        .unwrap_or_default();
    check_exports(&exports)?;
    Ok(exports)
}

fn check_exports(exports: &[String]) -> Result<()> {
    let unknown: Vec<&str> = exports
        .iter()
        .map(String::as_str)
        .filter(|e| !ENTRY_POINTS.contains(e))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "{SOURCE} exports {}; a screener module exports only {} — move helpers to another file",
            unknown.join(", "),
            ENTRY_POINTS.join(", ")
        );
    }
    for required in REQUIRED {
        if !exports.iter().any(|e| e == required) {
            bail!("{SOURCE} must export function {required}");
        }
    }
    Ok(())
}

/// The wrapper `st` bundles: each export goes through the PDK's `entry`.
fn entry_module(exports: &[String]) -> String {
    let mut out = format!(
        "import * as screener from \"../src/index\";\nimport {{ entry }} from \"{PDK_IMPORT}\";\n"
    );
    for name in exports {
        out.push_str(&format!(
            "export const {name} = entry(\"{name}\", screener.{name});\n"
        ));
    }
    out
}

/// extism-js interface: exactly the bundled exports (a mismatch calls the wrong function), the
/// ABI host functions, and `error_set` for async errors.
fn interface(exports: &[String]) -> String {
    let mut out = String::from("declare module \"main\" {\n");
    for name in exports {
        out.push_str(&format!("  export function {name}(): I32;\n"));
    }
    out.push_str("}\ndeclare module \"extism:host\" {\n  interface user {\n");
    for name in HOST_FUNCTIONS {
        out.push_str(&format!("    {name}(ptr: I64): I64;\n"));
    }
    out.push_str("  }\n}\ndeclare module \"extism:host\" {\n  interface env {\n    error_set(ptr: I64);\n  }\n}\n");
    out
}

/// wasm-metadce keeps what the roots reach: memory and the entry points.
fn dce_graph(exports: &[String]) -> Result<String> {
    let names: Vec<String> = std::iter::once("memory".to_string())
        .chain(exports.iter().cloned())
        .collect();
    let mut graph = vec![serde_json::json!({
        "name": "root",
        "root": true,
        "reaches": names.iter().map(|n| format!("e_{n}")).collect::<Vec<_>>(),
    })];
    for name in &names {
        graph.push(serde_json::json!({ "name": format!("e_{name}"), "export": name }));
    }
    serde_json::to_string(&graph).context("cannot encode the dce graph")
}

fn with_binaryen_path(command: &mut Command, tools: &Tools) -> Result<()> {
    let mut paths = vec![tools.binaryen_bin.clone()];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    command.env(
        "PATH",
        std::env::join_paths(paths).context("cannot extend PATH with binaryen")?,
    );
    Ok(())
}

fn compile(dir: &Path, exports: &[String], tools: &Tools) -> Result<Vec<u8>> {
    let out = dir.join(OUT_DIR);
    let file = |name: &str| out.join(name);
    std::fs::write(
        file("wasi.wasm"),
        wat::parse_str(WASI_SHIM).context("WASI stub")?,
    )?;
    std::fs::write(
        file("env.wasm"),
        wat::parse_str(ENV_SHIM).context("env stub")?,
    )?;
    std::fs::write(file("graph.json"), dce_graph(exports)?)?;

    let mut extism_js = Command::new(&tools.extism_js);
    extism_js
        .arg(file("bundle.js"))
        .arg("-i")
        .arg(file("bundle.d.ts"))
        .arg("-o")
        .arg(file("raw.wasm"))
        .current_dir(dir);
    with_binaryen_path(&mut extism_js, tools)?;
    if let Err(e) = run(&mut extism_js, "extism-js") {
        let text = format!("{e:#}");
        if text.contains("Wizer") || text.contains("cannot call") {
            bail!(
                "{text}\nhint: top-level code runs at build time — move host calls (tickers(), http(), kvGet(), …) into init/on_timer"
            );
        }
        return Err(e);
    }

    run(
        Command::new(tools.binaryen("wasm-merge"))
            .arg(file("raw.wasm"))
            .arg("main")
            .arg(file("env.wasm"))
            .arg("extism:host/env")
            .arg(file("wasi.wasm"))
            .arg("wasi_snapshot_preview1")
            .arg("-o")
            .arg(file("merged.wasm"))
            .args(WASM_FEATURES),
        "wasm-merge",
    )?;
    run(
        Command::new(tools.binaryen("wasm-metadce"))
            .arg(file("merged.wasm"))
            .arg("-f")
            .arg(file("graph.json"))
            .arg("-o")
            .arg(file("dce.wasm"))
            .args(WASM_FEATURES),
        "wasm-metadce",
    )?;
    run(
        Command::new(tools.binaryen("wasm-opt"))
            .arg("-O3")
            .arg("--strip")
            .arg("--duplicate-import-elimination")
            .args(WASM_FEATURES)
            .arg(file("dce.wasm"))
            .arg("-o")
            .arg(file("screener.wasm")),
        "wasm-opt",
    )?;
    std::fs::read(file("screener.wasm")).context("wasm-opt wrote no module")
}

/// Builds the module of a `lang: ts` project; the caller checks it like any other.
pub fn build(dir: &Path) -> Result<Vec<u8>> {
    if !dir.join(SOURCE).is_file() {
        bail!("{} not found", dir.join(SOURCE).display());
    }
    if !dir.join(PDK_FILE).is_file() {
        write_pdk(dir)?;
    }
    ensure_node_modules(dir)?;
    type_check(dir)?;
    let out = dir.join(OUT_DIR);
    std::fs::create_dir_all(&out).with_context(|| format!("cannot create {}", out.display()))?;

    let exports = exports(dir)?;
    std::fs::write(out.join("entry.ts"), entry_module(&exports))?;
    esbuild(
        dir,
        &[
            &format!("{OUT_DIR}/entry.ts"),
            "--format=cjs",
            "--target=es2020",
            &format!("--outfile={OUT_DIR}/bundle.js"),
        ],
    )?;
    std::fs::write(out.join("bundle.d.ts"), interface(&exports))?;
    let tools = toolchain::ensure()?;
    compile(dir, &exports, &tools)
}

pub fn write_pdk(dir: &Path) -> Result<()> {
    let path = dir.join(PDK_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, PDK).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_entry_points_may_be_exported() {
        assert!(check_exports(&names(&["init", "on_timer", "on_click"])).is_ok());
        let helper = check_exports(&names(&["init", "on_timer", "helper"])).unwrap_err();
        assert!(helper.to_string().contains("helper"), "{helper}");
        assert!(check_exports(&names(&["init"])).is_err());
    }

    #[test]
    fn interface_lists_exports_and_every_host_function() {
        let d_ts = interface(&names(&["init", "on_timer"]));
        assert!(d_ts.contains("export function init(): I32;"));
        assert!(!d_ts.contains("on_click"));
        for name in HOST_FUNCTIONS {
            assert!(
                d_ts.contains(&format!("    {name}(ptr: I64): I64;")),
                "{name}"
            );
        }
        assert!(d_ts.contains("error_set(ptr: I64);"));
        assert!(
            entry_module(&names(&["init"]))
                .contains("export const init = entry(\"init\", screener.init);")
        );
    }

    #[test]
    fn shims_parse_and_graph_roots_the_exports() {
        assert!(wat::parse_str(WASI_SHIM).is_ok());
        assert!(wat::parse_str(ENV_SHIM).is_ok());
        let graph: serde_json::Value =
            serde_json::from_str(&dce_graph(&names(&["init", "on_timer"])).unwrap()).unwrap();
        assert_eq!(
            graph[0]["reaches"],
            serde_json::json!(["e_memory", "e_init", "e_on_timer"])
        );
        assert_eq!(graph[1]["export"], "memory");
    }
}
