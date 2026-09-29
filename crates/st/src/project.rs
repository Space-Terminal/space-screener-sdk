use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::imports::{self, Module};
use crate::manifest::{self, Manifest};

pub const WASM_TARGET: &str = "wasm32-unknown-unknown";
pub const WASM_FILE: &str = "screener.wasm";

pub struct Built {
    pub manifest_text: String,
    pub manifest: Manifest,
    pub wasm: Vec<u8>,
    pub module: Module,
}

pub fn check_manifest(dir: &Path) -> Result<(String, Manifest)> {
    let (text, manifest) = manifest::read(dir)?;
    let report = manifest::validate(&manifest);
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    if !report.errors.is_empty() {
        bail!("manifest.yaml:\n  {}", report.errors.join("\n  "));
    }
    Ok((text, manifest))
}

fn check_module(wasm: &[u8]) -> Result<Module> {
    let module = imports::inspect(wasm)?;
    let problems = imports::check(wasm, &module);
    if !problems.is_empty() {
        bail!("{WASM_FILE}:\n  {}", problems.join("\n  "));
    }
    Ok(module)
}

fn ensure_target(dir: &Path) -> Result<()> {
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .current_dir(dir)
        .output();
    let installed = match installed {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => bail!(
            "rustup target list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(_) => {
            eprintln!(
                "warning: rustup not found; assuming the {WASM_TARGET} target is installed for your toolchain"
            );
            return Ok(());
        }
    };
    if installed.lines().any(|l| l.trim() == WASM_TARGET) {
        return Ok(());
    }
    eprintln!("installing the {WASM_TARGET} target (one time)…");
    let status = Command::new("rustup")
        .args(["target", "add", WASM_TARGET])
        .current_dir(dir)
        .status()
        .context("cannot run rustup")?;
    if !status.success() {
        bail!("rustup target add {WASM_TARGET} failed");
    }
    Ok(())
}

#[derive(Deserialize)]
struct Metadata {
    target_directory: PathBuf,
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    manifest_path: PathBuf,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

fn artifact_path(dir: &Path) -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(dir)
        .output()
        .context("cannot run cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let meta: Metadata =
        serde_json::from_slice(&out.stdout).context("cannot parse cargo metadata")?;
    let own_manifest = dir
        .join("Cargo.toml")
        .canonicalize()
        .context("Cargo.toml not found")?;
    let lib = meta
        .packages
        .iter()
        .filter(|p| p.manifest_path.canonicalize().ok().as_ref() == Some(&own_manifest))
        .flat_map(|p| &p.targets)
        .find(|t| t.kind.iter().any(|k| k == "cdylib"))
        .context("Cargo.toml needs `[lib] crate-type = [\"cdylib\"]`")?;
    Ok(meta
        .target_directory
        .join(WASM_TARGET)
        .join("release")
        .join(format!("{}.wasm", lib.name.replace('-', "_"))))
}

pub fn build(dir: &Path) -> Result<Built> {
    let (manifest_text, manifest) = check_manifest(dir)?;
    ensure_target(dir)?;
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", WASM_TARGET])
        .current_dir(dir)
        .status()
        .context("cannot run cargo")?;
    if !status.success() {
        bail!("cargo build failed");
    }
    let artifact = artifact_path(dir)?;
    let wasm =
        std::fs::read(&artifact).with_context(|| format!("cannot read {}", artifact.display()))?;
    std::fs::write(dir.join(WASM_FILE), &wasm).context("cannot write screener.wasm")?;
    let module = check_module(&wasm)?;
    Ok(Built {
        manifest_text,
        manifest,
        wasm,
        module,
    })
}

pub fn validate(dir: &Path) -> Result<Built> {
    let (manifest_text, manifest) = check_manifest(dir)?;
    let path = dir.join(WASM_FILE);
    let wasm = std::fs::read(&path)
        .with_context(|| format!("cannot read {}; run `st build` first", path.display()))?;
    let module = check_module(&wasm)?;
    Ok(Built {
        manifest_text,
        manifest,
        wasm,
        module,
    })
}

pub fn print_summary(built: &Built) {
    println!(
        "{} {} — {} ({} KiB)",
        built.manifest.id,
        built.manifest.version,
        built.manifest.name.any(),
        built.wasm.len() / 1024
    );
    println!("imports:");
    for (module, name) in &built.module.imports {
        println!("  {module}::{name}");
    }
    let exports: Vec<&str> = built
        .module
        .exports
        .iter()
        .map(String::as_str)
        .filter(|e| ["init", "on_timer", "on_click", "on_params", "on_batch"].contains(e))
        .collect();
    println!("entry points: {}", exports.join(", "));
}

pub fn screener_id(dir: &Path, explicit: Option<String>) -> Result<String> {
    if let Some(id) = explicit {
        return Ok(id);
    }
    let (_, manifest) =
        manifest::read(dir).context("pass --id or run inside a screener project")?;
    Ok(manifest.id)
}
