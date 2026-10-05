use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use space_screener_check::wasm::ENTRY_POINTS;
use space_screener_check::{Lang, Manifest, PluginLang, Report, WasmInfo};

use crate::{manifest, ts};

pub const WASM_TARGET: &str = "wasm32-unknown-unknown";
pub const WASM_FILE: &str = space_screener_check::wasm::FILE;

pub struct Built {
    pub manifest_text: String,
    pub manifest: Manifest,
    pub wasm: Vec<u8>,
    pub module: WasmInfo,
}

pub fn problems(report: &Report) -> String {
    report
        .errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n  ")
}

pub fn check_manifest(dir: &Path) -> Result<(String, Manifest)> {
    let text = manifest::read(dir)?;
    let (manifest, report) = Manifest::check(&text);
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    match manifest {
        Some(manifest) if report.is_ok() => Ok((text, manifest)),
        _ => bail!("{}:\n  {}", manifest::FILE, problems(&report)),
    }
}

/// Shape limits of `inspect` (functions, body size, nesting, br_table) name the function.
fn shape_hint(report: &Report) -> &'static str {
    if report
        .errors
        .iter()
        .any(|issue| issue.path == "functions" || issue.path.starts_with("function "))
    {
        "\nhint: the terminal caps functions at 256 KiB of code each (10 000 functions, nesting 2 000, \
         br_table 10 000 targets); an oversized one is usually a giant match or a table built inline — \
         move the data into a static or split the function"
    } else {
        ""
    }
}

fn check_module(wasm: &[u8], manifest: &Manifest) -> Result<WasmInfo> {
    let info = space_screener_check::inspect(wasm).map_err(|report| {
        anyhow!(
            "{WASM_FILE}:\n  {}{}",
            problems(&report),
            shape_hint(&report)
        )
    })?;
    space_screener_check::check_imports(manifest, &info)
        .map_err(|report| anyhow!("{}:\n  {}", manifest::FILE, problems(&report)))?;
    Ok(info)
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

fn build_rust(dir: &Path) -> Result<Vec<u8>> {
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
    std::fs::read(&artifact).with_context(|| format!("cannot read {}", artifact.display()))
}

pub fn build(dir: &Path) -> Result<Built> {
    let (manifest_text, manifest) = check_manifest(dir)?;
    let wasm = match manifest.lang {
        PluginLang::Rust => build_rust(dir)?,
        PluginLang::Ts => ts::build(dir, &manifest.min_terminal)?,
    };
    std::fs::write(dir.join(WASM_FILE), &wasm).context("cannot write screener.wasm")?;
    let module = check_module(&wasm, &manifest)?;
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
    let module = check_module(&wasm, &manifest)?;
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
        built.manifest.name.get(Lang::En),
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
        .filter(|e| ENTRY_POINTS.contains(e))
        .collect();
    println!("entry points: {}", exports.join(", "));
}

/// The catalog's extra rules (categories, description, source): a warning here, an error on
/// `st publish`.
pub fn print_catalog_readiness(built: &Built) {
    let checked = space_screener_check::registry::check_wasm(&built.module).and_then(|()| {
        space_screener_check::registry::check(&built.manifest_text, &built.manifest)
    });
    match checked {
        Ok(info) => println!("catalog: ready ({})", info.categories.join(", ")),
        Err(report) => eprintln!(
            "warning: not ready for the catalog (`st publish` refuses it):\n  {}",
            problems(&report)
        ),
    }
}

pub fn screener_id(dir: &Path, explicit: Option<String>) -> Result<String> {
    if let Some(id) = explicit {
        return Ok(id);
    }
    manifest::read_id(dir).context("pass --id or run inside a screener project")
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_screener_check::{Code, Issue};

    fn report(path: &str) -> Report {
        Report {
            errors: vec![Issue {
                code: Code::InvalidWasm,
                path: path.to_string(),
                message: "x".to_string(),
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn shape_errors_get_a_hint() {
        assert!(shape_hint(&report("function 12")).contains("256 KiB"));
        assert!(shape_hint(&report("functions")).contains("split the function"));
        assert_eq!(shape_hint(&report("import wasi::fd_write")), "");
    }
}
