use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable error code, the same string the terminal's local API and the registry return as `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidManifest,
    UnsupportedFeed,
    TerminalTooOld,
    InvalidWasm,
    ForbiddenImport,
    ForbiddenExport,
    TooLarge,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidManifest => "invalid_manifest",
            Self::UnsupportedFeed => "unsupported_feed",
            Self::TerminalTooOld => "terminal_too_old",
            Self::InvalidWasm => "invalid_wasm",
            Self::ForbiddenImport => "forbidden_import",
            Self::ForbiddenExport => "forbidden_export",
            Self::TooLarge => "too_large",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One broken rule. `path` points into the manifest (`columns[1].sort`) or the module
/// (`import extism:host/env::log_info`); it is empty when the whole file is at fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: Code,
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}: {}", self.code, self.message)
        } else {
            write!(f, "{}: {}: {}", self.code, self.path, self.message)
        }
    }
}

/// Every problem found in one pass. Callers that stop at the first error (the terminal)
/// take [`Report::first`]; tools show all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn first(&self) -> Option<&Issue> {
        self.errors.first()
    }

    pub(crate) fn error(
        &mut self,
        code: Code,
        path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.errors.push(Issue {
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, issue) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Report {}
