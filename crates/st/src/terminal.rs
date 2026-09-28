use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub const DEFAULT_PORT: u16 = 5055;
pub const DIR_ENV: &str = "SPACE_TERMINAL_DIR";
pub const PORT_ENV: &str = "SPACE_TERMINAL_PORT";
const APP_DIR: &str = "Space Terminal";
const TOKEN_FILE: &str = "screeners/local_api_token";
const CONFIG_FILE: &str = "config/general.yaml";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub fn base_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    dirs::data_dir()
        .map(|d| d.join(APP_DIR))
        .with_context(|| format!("cannot locate the user data directory; set {DIR_ENV}"))
}

fn config_port(base: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(base.join(CONFIG_FILE)).ok()?;
    let config: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let port = config
        .get("general")
        .and_then(|g| g.get("local_api"))
        .or_else(|| config.get("local_api"))?
        .get("port")?
        .as_u64()?;
    u16::try_from(port).ok()
}

fn resolve_port(base: &Path, cli: Option<u16>) -> Result<(u16, String)> {
    if let Some(port) = cli {
        return Ok((port, "--port".into()));
    }
    if let Ok(raw) = std::env::var(PORT_ENV) {
        let port = raw
            .trim()
            .parse()
            .with_context(|| format!("{PORT_ENV}=`{raw}` is not a port"))?;
        return Ok((port, PORT_ENV.into()));
    }
    if let Some(port) = config_port(base) {
        return Ok((port, base.join(CONFIG_FILE).display().to_string()));
    }
    Ok((DEFAULT_PORT, "default".into()))
}

#[derive(Debug, Deserialize)]
pub struct Name {
    #[serde(default)]
    pub ru: Option<String>,
    #[serde(default)]
    pub en: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScreenerInfo {
    pub id: String,
    #[serde(default)]
    pub version: String,
    pub name: Option<Name>,
    #[serde(default)]
    pub dev: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub rows: u64,
    #[serde(default)]
    pub panes: u64,
}

#[derive(Debug, Deserialize)]
pub struct List {
    #[serde(default)]
    pub screeners: Vec<ScreenerInfo>,
}

#[derive(Debug, Deserialize)]
pub struct Installed {
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub pane: String,
}

#[derive(Debug, Deserialize)]
pub struct LogLine {
    #[serde(default)]
    pub ts_ms: i64,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub msg: String,
}

#[derive(Debug, Deserialize)]
pub struct Logs {
    #[serde(default)]
    pub lines: Vec<LogLine>,
    #[serde(default)]
    pub next: u64,
}

#[derive(Debug, Deserialize)]
pub struct Rows {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub rows: Vec<Value>,
}

pub struct Terminal {
    port: u16,
    port_source: String,
    token_path: PathBuf,
    token: String,
    client: Client,
}

impl Terminal {
    pub fn connect(cli_port: Option<u16>) -> Result<Self> {
        let base = base_dir()?;
        let (port, port_source) = resolve_port(&base, cli_port)?;
        let token_path = base.join(TOKEN_FILE);
        let token = std::fs::read_to_string(&token_path)
            .map(|t| t.trim().to_string())
            .with_context(|| {
                format!(
                    "cannot read the terminal token at {}. Start Space Terminal once (it creates the token) \
                     or set {DIR_ENV} to the terminal data folder (for `cargo run` builds: the repository root)",
                    token_path.display()
                )
            })?;
        let client = Client::builder()
            .no_proxy()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("cannot build the HTTP client")?;
        Ok(Self {
            port,
            port_source,
            token_path,
            token,
            client,
        })
    }

    pub fn describe(&self) -> String {
        format!("127.0.0.1:{} (port from {})", self.port, self.port_source)
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/api/v1/screeners{path}", self.port)
    }

    fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = req.bearer_auth(&self.token).send().map_err(|e| {
            if e.is_connect() {
                anyhow!(
                    "Space Terminal is not reachable at {}. Is it running? Override with --port or {PORT_ENV}",
                    self.describe()
                )
            } else {
                anyhow!(e).context("request to Space Terminal failed")
            }
        })?;
        let status = resp.status();
        let body = resp.text().context("cannot read the terminal response")?;
        if status == StatusCode::UNAUTHORIZED {
            bail!(
                "unauthorized: the terminal rejected the token from {}. Does {DIR_ENV} point at the data folder \
                 of the terminal listening on {}?",
                self.token_path.display(),
                self.describe()
            );
        }
        let value: Value = serde_json::from_str(&body).map_err(|_| {
            if status == StatusCode::NOT_FOUND {
                anyhow!(
                    "this Space Terminal build has no screener API (HTTP 404); update the terminal"
                )
            } else {
                anyhow!("unexpected terminal response (HTTP {status}): {body}")
            }
        })?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let code = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("error");
            let message = value.get("message").and_then(Value::as_str).unwrap_or("");
            bail!("{code}: {message} (HTTP {status})");
        }
        serde_json::from_value(value).context("unexpected terminal response shape")
    }

    pub fn list(&self) -> Result<List> {
        self.send(self.client.get(self.url("")))
    }

    pub fn install(&self, manifest: &str, wasm: &[u8], open_pane: bool) -> Result<Installed> {
        let body = json!({
            "manifest": manifest,
            "wasm_b64": base64::engine::general_purpose::STANDARD.encode(wasm),
            "open_pane": open_pane,
        });
        self.send(self.client.post(self.url("/install")).json(&body))
    }

    pub fn logs(&self, id: &str, since: u64) -> Result<Logs> {
        self.send(
            self.client
                .get(self.url(&format!("/{id}/logs")))
                .query(&[("since", since)]),
        )
    }

    pub fn rows(&self, id: &str) -> Result<Rows> {
        self.send(self.client.get(self.url(&format!("/{id}/rows"))))
    }

    /// The rows response exactly as the terminal sent it.
    pub fn rows_json(&self, id: &str) -> Result<Value> {
        self.send(self.client.get(self.url(&format!("/{id}/rows"))))
    }
}

pub fn format_time(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1000).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

pub fn print_logs(lines: &[LogLine]) {
    for line in lines {
        println!("{} {:<5} {}", format_time(line.ts_ms), line.level, line.msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_comes_from_nested_general_config() {
        let dir = std::env::temp_dir().join(format!("st-port-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            "general:\n  local_api:\n    enabled: true\n    port: 5056\n    target: Ask\npanes: {}\n",
        )
        .unwrap();
        assert_eq!(config_port(&dir), Some(5056));
        assert_eq!(resolve_port(&dir, Some(6000)).unwrap().0, 6000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn utc_clock() {
        assert_eq!(format_time(3_723_000), "01:02:03Z");
    }
}
