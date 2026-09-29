//! Fields the catalog reads from `manifest.yaml` on publish. The terminal ignores them, so they
//! are checked here and not by [`crate::Manifest::parse`].

use serde::Deserialize;

use crate::manifest::Manifest;
use crate::report::{Code, Report};

pub const CATEGORIES: &[&str] = &[
    "volume",
    "open-interest",
    "funding",
    "spread",
    "movers",
    "listings",
    "orderbook",
    "other",
];
pub const MAX_CATEGORIES: usize = 3;
pub const MAX_DESCRIPTION: usize = 2000;
pub const MAX_SOURCE_URL: usize = 512;

/// Catalog metadata of a manifest that passed [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryInfo {
    pub categories: Vec<String>,
    pub source: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRegistry {
    #[serde(default)]
    categories: Option<serde_yaml::Value>,
    #[serde(default)]
    source: Option<serde_yaml::Value>,
}

/// Registry rules on top of a manifest that already passed [`Manifest::parse`]: 1..=3 known
/// categories, a description in at least one language (each ≤ 2000 characters) and an optional
/// `https://` link to the source code.
pub fn check(yaml: &str, manifest: &Manifest) -> Result<RegistryInfo, Report> {
    let mut r = Report::default();
    let raw: RawRegistry = match serde_yaml::from_str(yaml) {
        Ok(raw) => raw,
        Err(e) => {
            r.error(
                Code::InvalidManifest,
                "",
                format!("manifest.yaml does not parse: {e}"),
            );
            return Err(r);
        }
    };

    let mut categories = Vec::new();
    match raw.categories {
        None => r.error(
            Code::InvalidManifest,
            "categories",
            format!(
                "categories are required for the catalog: 1..={MAX_CATEGORIES} of {}",
                CATEGORIES.join(", ")
            ),
        ),
        Some(serde_yaml::Value::Sequence(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(category) if CATEGORIES.contains(&category) => {
                        if !categories.iter().any(|c| c == category) {
                            categories.push(category.to_string());
                        }
                    }
                    _ => r.error(
                        Code::InvalidManifest,
                        format!("categories[{i}]"),
                        format!("must be one of {}", CATEGORIES.join(", ")),
                    ),
                }
            }
            if items.is_empty() || items.len() > MAX_CATEGORIES {
                r.error(
                    Code::InvalidManifest,
                    "categories",
                    format!("list 1..={MAX_CATEGORIES} categories"),
                );
            }
        }
        Some(_) => r.error(
            Code::InvalidManifest,
            "categories",
            "categories must be a list, e.g. [open-interest, funding]",
        ),
    }

    match &manifest.description {
        Some(description) if !description.is_blank() => {
            for (lang, text) in [("ru", &description.ru), ("en", &description.en)] {
                if text.chars().count() > MAX_DESCRIPTION {
                    r.error(
                        Code::InvalidManifest,
                        format!("description.{lang}"),
                        format!("longer than {MAX_DESCRIPTION} characters"),
                    );
                }
            }
        }
        _ => r.error(
            Code::InvalidManifest,
            "description",
            "the catalog card needs a description in `ru` or `en`",
        ),
    }

    let source = match raw.source {
        None | Some(serde_yaml::Value::Null) => None,
        Some(serde_yaml::Value::String(url)) if is_https_url(&url) => Some(url),
        Some(_) => {
            r.error(
                Code::InvalidManifest,
                "source",
                format!(
                    "source must be an https:// link to the code, ≤ {MAX_SOURCE_URL} characters"
                ),
            );
            None
        }
    };

    if r.is_ok() {
        Ok(RegistryInfo { categories, source })
    } else {
        Err(r)
    }
}

fn is_https_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= MAX_SOURCE_URL
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
        && host.contains('.')
        && !host.contains('@')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(extra: &str) -> String {
        format!(
            "abi: 1\nid: ivan.oi\nversion: 1.0.0\nname: {{en: OI}}\nlang: rust\nmin_terminal: 0.104.0\n\
             columns: [{{key: oi, type: usd}}]\n{extra}"
        )
    }

    fn run(extra: &str) -> Result<RegistryInfo, Report> {
        let text = yaml(extra);
        let manifest = Manifest::parse(&text).unwrap();
        check(&text, &manifest)
    }

    #[test]
    fn accepts_catalog_fields() {
        let info = run(
            "description: {en: Open interest}\ncategories: [open-interest, funding, open-interest]\nsource: https://github.com/ivan/oi\n",
        )
        .unwrap();
        assert_eq!(info.categories, ["open-interest", "funding"]);
        assert_eq!(info.source.as_deref(), Some("https://github.com/ivan/oi"));
    }

    #[test]
    fn manifest_without_catalog_fields_still_parses_for_the_terminal() {
        assert!(Manifest::parse(&yaml("categories: 5\nsource: {x: 1}\n")).is_ok());
    }

    #[test]
    fn reports_every_catalog_problem() {
        let long = "x".repeat(MAX_DESCRIPTION + 1);
        let report = run(&format!(
            "description: {{ru: \"{long}\"}}\ncategories: [volume, cats, funding, spread]\nsource: http://example.com\n"
        ))
        .unwrap_err()
        .to_string();
        for needle in [
            "categories[1]",
            "list 1..=3",
            "description.ru",
            "source must be",
        ] {
            assert!(report.contains(needle), "missing `{needle}` in:\n{report}");
        }
        let missing = run("").unwrap_err().to_string();
        assert!(missing.contains("categories are required"), "{missing}");
        assert!(missing.contains("needs a description"), "{missing}");
    }

    #[test]
    fn source_must_be_a_plain_https_link() {
        for bad in [
            "https://",
            "https://user@evil.com/x",
            "https://exa mple.com",
            "javascript:alert(1)",
            "https://localhost/x",
        ] {
            assert!(!is_https_url(bad), "{bad}");
        }
        assert!(is_https_url("https://gitlab.com/a/b?tab=readme#top"));
    }
}
