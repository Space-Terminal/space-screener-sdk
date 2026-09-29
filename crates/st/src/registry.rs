use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use space_screener_check::{Recording, registry};
use space_screener_harness::{Verdict, trial};

use crate::project::{self, Built};
use crate::{manifest, test};

pub const DEFAULT_REGISTRY: &str = "https://store.space-terminal.com";
pub const TOKEN_ENV: &str = "ST_TOKEN";
pub const REGISTRY_ENV: &str = "ST_REGISTRY";
/// Folder with `credentials.yaml` (default: the OS config folder + `space-screener`).
pub const CONFIG_ENV: &str = "ST_CONFIG_DIR";
const TOKEN_PREFIX: &str = "stp_";
const CREDENTIALS_FILE: &str = "credentials.yaml";
const MAX_RECORDING_BYTES: usize = 4 * 1024 * 1024;
/// Upload plus the server's trial run, which waits for a free slot.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(CONFIG_ENV) {
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir()
        .map(|d| d.join("space-screener"))
        .with_context(|| format!("cannot locate the config folder; set {CONFIG_ENV}"))
}

fn credentials_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(CREDENTIALS_FILE))
}

fn read_credentials(path: &Path) -> Result<Credentials> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_yaml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
        Err(e) => Err(anyhow!(e).context(format!("cannot read {}", path.display()))),
    }
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("cannot restrict {}", path.display()))?;
    }
    file.write_all(text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

fn check_token(token: &str) -> Result<()> {
    let body = token.strip_prefix(TOKEN_PREFIX).unwrap_or_default();
    if body.len() < 16
        || !body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        bail!(
            "a publish token looks like `{TOKEN_PREFIX}…`; create one on the author page of the store (/author)"
        );
    }
    Ok(())
}

/// `https://` anywhere; plain `http://` only on this machine, so a token never crosses the
/// network in the clear.
fn normalize_registry(url: &str) -> Result<String> {
    let url = url.trim().trim_end_matches('/');
    let host = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
            bail!("`{url}`: plain http is allowed only for 127.0.0.1/localhost; use https://");
        }
        rest
    } else {
        bail!("`{url}` is not an http(s) URL");
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        bail!("`{url}` has no host");
    }
    Ok(url.to_string())
}

pub fn login(token: Option<String>, registry: Option<String>) -> Result<()> {
    let token = match token {
        Some(token) => token,
        None => {
            print!("publish token (from the author page, starts with {TOKEN_PREFIX}): ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .context("cannot read the token")?;
            line
        }
    };
    let token = token.trim().to_string();
    check_token(&token)?;
    let registry = registry.map(|r| normalize_registry(&r)).transpose()?;
    let path = credentials_path()?;
    let credentials = Credentials {
        registry: registry.clone(),
        token: Some(token),
    };
    let text = serde_yaml::to_string(&credentials).context("cannot encode the credentials")?;
    write_private(&path, &text)?;
    println!(
        "saved the token for {} in {}",
        registry.as_deref().unwrap_or(DEFAULT_REGISTRY),
        path.display()
    );
    Ok(())
}

struct Target {
    registry: String,
    token: String,
}

fn resolve(cli_registry: Option<String>) -> Result<Target> {
    let path = credentials_path()?;
    let saved = read_credentials(&path)?;
    let token = match std::env::var(TOKEN_ENV) {
        Ok(token) if !token.trim().is_empty() => token.trim().to_string(),
        _ => saved.token.clone().with_context(|| {
            format!(
                "no publish token: run `st login` or set {TOKEN_ENV} (tokens are made on the author page of the store)"
            )
        })?,
    };
    check_token(&token)?;
    let registry = cli_registry
        .or_else(|| {
            std::env::var(REGISTRY_ENV)
                .ok()
                .filter(|r| !r.trim().is_empty())
        })
        .or(saved.registry)
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_string());
    Ok(Target {
        registry: normalize_registry(&registry)?,
        token,
    })
}

fn load_recording(path: &Path, built: &Built) -> Result<(Recording, Value)> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("cannot read {}", path.display()))?
        .len();
    if size > MAX_RECORDING_BYTES as u64 {
        bail!(
            "{} is {} KiB; the registry takes recordings up to {} KiB — record fewer seconds",
            path.display(),
            size / 1024,
            MAX_RECORDING_BYTES / 1024
        );
    }
    let recording = test::read_recording(path)?;
    if recording.id != built.manifest.id {
        bail!(
            "{} was recorded for `{}`, not `{}`",
            path.display(),
            recording.id,
            built.manifest.id
        );
    }
    let value = serde_json::to_value(&recording).context("cannot encode the recording")?;
    Ok((recording, value))
}

fn print_issues(label: &str, items: Option<&Value>) {
    let Some(items) = items.and_then(Value::as_array) else {
        return;
    };
    for item in items {
        match item {
            Value::String(text) => println!("  {label}: {text}"),
            Value::Object(issue) => {
                let field = |k: &str| issue.get(k).and_then(Value::as_str).unwrap_or_default();
                let path = field("path");
                if path.is_empty() {
                    println!("  {label}: {}: {}", field("code"), field("message"));
                } else {
                    println!("  {label}: {}: {path}: {}", field("code"), field("message"));
                }
            }
            other => println!("  {label}: {other}"),
        }
    }
}

/// Builds, checks locally (validator, catalog fields, a trial run) and uploads the version to
/// the registry, where it waits for a moderator.
pub fn publish(
    dir: &Path,
    recording: Option<PathBuf>,
    build: bool,
    cli_registry: Option<String>,
) -> Result<()> {
    let target = resolve(cli_registry)?;
    let built = if build {
        project::build(dir)?
    } else {
        project::validate(dir)?
    };
    let info = registry::check(&built.manifest_text, &built.manifest).map_err(|report| {
        anyhow!(
            "{} is not ready for the catalog:\n  {}",
            manifest::FILE,
            project::problems(&report)
        )
    })?;
    let recording = recording
        .map(|path| load_recording(&path, &built))
        .transpose()?;

    let report = trial(
        &built.manifest,
        &built.wasm,
        recording.as_ref().map(|(r, _)| r),
    );
    println!(
        "trial run ({}): {}, {} rows",
        if report.recorded {
            "recording"
        } else {
            "offline data"
        },
        match report.verdict {
            Verdict::Ok => "ok",
            Verdict::Warn => "warn",
            Verdict::Fail => "fail",
        },
        report.total_rows
    );
    for issue in &report.issues {
        println!("  {issue}");
    }
    if report.verdict == Verdict::Fail {
        bail!("the trial run failed; the registry would reject this version");
    }

    println!(
        "publishing {} {} [{}] to {}…",
        built.manifest.id,
        built.manifest.version,
        info.categories.join(", "),
        target.registry
    );
    let mut body = json!({
        "manifest": built.manifest_text,
        "wasm_b64": base64::engine::general_purpose::STANDARD.encode(&built.wasm),
    });
    if let Some((_, value)) = recording {
        body["recording"] = value;
    }
    let client = Client::builder()
        .timeout(PUBLISH_TIMEOUT)
        .redirect(Policy::none())
        .user_agent(concat!("st/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cannot build the HTTP client")?;
    let resp = client
        .post(format!("{}/api/v1/registry/publish", target.registry))
        .bearer_auth(&target.token)
        .json(&body)
        .send()
        .with_context(|| format!("cannot reach {}", target.registry))?;
    let status = resp.status();
    let text = resp.text().context("cannot read the registry response")?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| anyhow!("unexpected registry response (HTTP {status}): {text}"))?;
    if !status.is_success() {
        let code = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("error");
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        print_issues("error", value.get("details"));
        let hint = match code {
            "unauthorized" | "token_invalid" => {
                " — the token was revoked or mistyped; make a new one and run `st login`"
            }
            "not_author" => " — set your author name on the author page first",
            "version_exists" | "version_not_greater" => " — raise `version` in manifest.yaml",
            _ => "",
        };
        bail!("{code}: {message} (HTTP {status}){hint}");
    }

    let field = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or_default();
    println!(
        "published {} {} — {}; a moderator reviews every version before it shows in the catalog",
        field("id"),
        field("version"),
        field("status")
    );
    if let Some(report) = value.get("report") {
        if let Some(verdict) = report.get("verdict").and_then(Value::as_str) {
            println!("server trial run: {verdict}");
        }
        print_issues("issue", report.get("issues"));
        print_issues("warning", report.get("warnings"));
    }
    println!("status: {}/author", target.registry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_urls() {
        assert_eq!(
            normalize_registry("https://store.space-terminal.com/").unwrap(),
            "https://store.space-terminal.com"
        );
        assert_eq!(
            normalize_registry("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert!(normalize_registry("http://store.space-terminal.com").is_err());
        assert!(normalize_registry("ftp://x").is_err());
        assert!(normalize_registry("https://").is_err());
    }

    #[test]
    fn tokens() {
        assert!(check_token("stp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcd").is_ok());
        assert!(check_token("stp_short").is_err());
        assert!(check_token("ghp_AbCdEfGhIjKlMnOpQrSt").is_err());
        assert!(check_token("stp_has space in it 1234567").is_err());
    }

    #[test]
    fn credentials_roundtrip_with_private_mode() {
        let dir = std::env::temp_dir().join(format!("st-cred-{}", std::process::id()));
        let path = dir.join(CREDENTIALS_FILE);
        let text = serde_yaml::to_string(&Credentials {
            registry: Some("http://127.0.0.1:8080".into()),
            token: Some("stp_x".into()),
        })
        .unwrap();
        write_private(&path, &text).unwrap();
        let back = read_credentials(&path).unwrap();
        assert_eq!(back.token.as_deref(), Some("stp_x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(
            read_credentials(&dir.join("missing.yaml"))
                .unwrap()
                .token
                .is_none()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
