use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub use space_screener_check::manifest::{FILE, ID_RULE};

pub fn read(dir: &Path) -> Result<String> {
    let path = dir.join(FILE);
    std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))
}

#[derive(Deserialize)]
struct IdOnly {
    id: Option<String>,
}

/// The id alone, so `st logs`/`st rows` work even while the rest of the manifest is broken.
pub fn read_id(dir: &Path) -> Result<String> {
    let text = read(dir)?;
    let parsed: IdOnly =
        serde_yaml::from_str(&text).context("invalid_manifest: manifest.yaml does not parse")?;
    match parsed.id {
        Some(id) if !id.is_empty() => Ok(id),
        _ => bail!("manifest.yaml has no id"),
    }
}
