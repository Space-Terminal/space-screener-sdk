use serde::Deserialize;

pub type Result<T> = std::result::Result<T, Error>;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("host error {code}: {message}")]
    Host { code: String, message: String },
    #[error("http status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("host call failed: {0}")]
    Call(String),
    #[error("host functions are available only inside Space Terminal")]
    NotInTerminal,
}

impl Error {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Host { code, .. } => Some(code),
            _ => None,
        }
    }

    pub(crate) fn parse(msg: impl Into<String>) -> Self {
        Self::Parse(msg.into())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct HostError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

impl From<HostError> for Error {
    fn from(e: HostError) -> Self {
        Self::Host {
            code: e.code,
            message: e.message,
        }
    }
}
