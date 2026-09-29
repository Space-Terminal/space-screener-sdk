use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::manifest;

/// Oldest terminal with the screener runtime (ABI v1).
pub const MIN_TERMINAL: &str = "0.104.70";
const SDK_GIT: &str = "https://github.com/Space-Terminal/space-screener-sdk";
const BUNDLED_SDK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../space-screener");

const CARGO_TOML: &str = include_str!("../templates/Cargo.toml.tmpl");
const MANIFEST: &str = include_str!("../templates/manifest.yaml.tmpl");
const LIB_RS: &str = include_str!("../templates/lib.rs.tmpl");
const GITIGNORE: &str = include_str!("../templates/gitignore.tmpl");

fn crate_name(name: &str) -> String {
    let snake: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    match snake.chars().next() {
        Some(c) if c.is_ascii_lowercase() => snake,
        _ => format!("screener_{snake}"),
    }
}

fn type_name(name: &str) -> String {
    let camel: String = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase())
                .unwrap_or_default()
        })
        .collect();
    match camel.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => camel,
        _ => format!("Screener{camel}"),
    }
}

fn default_id(name: &str) -> String {
    let kebab: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("local.{}", kebab.trim_matches('-'))
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
}

// `cargo install --git` builds st from a checkout inside the cargo cache; that path changes with every
// update and disappears on a cache clean, so such builds point new projects at the git repository.
fn in_cargo_cache(path: &Path, cargo_home: &Path) -> bool {
    let home = cargo_home
        .canonicalize()
        .unwrap_or_else(|_| cargo_home.to_path_buf());
    ["git/checkouts", "registry"]
        .iter()
        .any(|cache| path.starts_with(home.join(cache)))
}

fn sdk_dependency(sdk_path: Option<PathBuf>) -> Result<String> {
    select_sdk_dependency(sdk_path, Path::new(BUNDLED_SDK), cargo_home().as_deref())
}

/// `--sdk-path` wins; then the SDK st was built from, unless that is the cargo cache; then git.
fn select_sdk_dependency(
    explicit: Option<PathBuf>,
    bundled: &Path,
    cargo_home: Option<&Path>,
) -> Result<String> {
    let path = match explicit {
        Some(path) => Some(
            path.canonicalize()
                .with_context(|| format!("--sdk-path {} does not exist", path.display()))?,
        ),
        None => bundled
            .canonicalize()
            .ok()
            .filter(|path| !cargo_home.is_some_and(|home| in_cargo_cache(path, home))),
    };
    Ok(match path {
        Some(path) => format!(
            "space-screener = {{ path = '{}' }}",
            path.display().to_string().replace('\\', "/")
        ),
        None => format!("space-screener = {{ git = \"{SDK_GIT}\" }}"),
    })
}

pub fn init(dir: &Path, id: Option<String>, sdk_path: Option<PathBuf>) -> Result<()> {
    for existing in ["Cargo.toml", manifest::FILE, "src/lib.rs"] {
        if dir.join(existing).exists() {
            bail!(
                "{} already has {existing}; st init only creates new projects",
                dir.display()
            );
        }
    }
    std::fs::create_dir_all(dir.join("src"))
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let name = dir
        .canonicalize()?
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("screener")
        .to_string();
    let id = id.unwrap_or_else(|| default_id(&name));
    if !space_screener_check::is_valid_id(&id) {
        bail!(
            "id `{id}` must match {} (pass --id author.name)",
            manifest::ID_RULE
        );
    }
    let render = |template: &str| {
        template
            .replace("{{crate}}", &crate_name(&name))
            .replace("{{type_name}}", &type_name(&name))
            .replace("{{name}}", &name)
            .replace("{{id}}", &id)
            .replace("{{min_terminal}}", MIN_TERMINAL)
    };
    let cargo = render(CARGO_TOML).replace("{{sdk_dep}}", &sdk_dependency(sdk_path)?);
    let files = [
        ("Cargo.toml", cargo),
        (manifest::FILE, render(MANIFEST)),
        ("src/lib.rs", render(LIB_RS)),
        (".gitignore", GITIGNORE.to_string()),
    ];
    for (file, content) in files {
        std::fs::write(dir.join(file), content).with_context(|| format!("cannot write {file}"))?;
    }
    println!("created screener `{id}` in {}", dir.display());
    println!("next: edit src/lib.rs and manifest.yaml, then `st build` and `st dev`");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_folder() {
        assert_eq!(crate_name("oi-8 exchanges"), "oi_8_exchanges");
        assert_eq!(crate_name("8ball"), "screener_8ball");
        assert_eq!(type_name("oi-8-exchanges"), "Oi8Exchanges");
        assert_eq!(type_name("8ball"), "Screener8ball");
        assert_eq!(default_id("My Screener"), "local.my-screener");
        assert!(space_screener_check::is_valid_id(&default_id("OI_8")));
    }

    #[test]
    fn sdk_dependency_prefers_explicit_then_local_checkout_then_git() {
        let root = std::env::temp_dir().join(format!("st-init-sdk-{}", std::process::id()));
        let home = root.join("cargo");
        let cached = home.join("git/checkouts/space-screener-sdk-1a2b/3c4d/crates/space-screener");
        let registry = home.join("registry/src/index/space-screener-0.1.0");
        let local = root.join("work/space-screener-sdk/crates/space-screener");
        for dir in [&cached, &registry, &local] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let git = format!("space-screener = {{ git = \"{SDK_GIT}\" }}");
        let is_path = |dep: &str| dep.starts_with("space-screener = { path = '");

        assert_eq!(
            select_sdk_dependency(None, &cached, Some(&home)).unwrap(),
            git
        );
        assert_eq!(
            select_sdk_dependency(None, &registry, Some(&home)).unwrap(),
            git
        );
        assert_eq!(
            select_sdk_dependency(None, &root.join("missing"), Some(&home)).unwrap(),
            git
        );
        assert!(is_path(
            &select_sdk_dependency(None, &local, Some(&home)).unwrap()
        ));
        assert!(is_path(
            &select_sdk_dependency(Some(cached.clone()), &local, Some(&home)).unwrap()
        ));
        assert!(select_sdk_dependency(Some(root.join("missing")), &local, Some(&home)).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn template_manifest_is_valid() {
        let text = MANIFEST
            .replace("{{id}}", "local.test")
            .replace("{{name}}", "test")
            .replace("{{min_terminal}}", MIN_TERMINAL);
        let manifest = space_screener_check::Manifest::parse(&text);
        assert!(manifest.is_ok(), "{manifest:?}");
    }
}
