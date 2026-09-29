use std::fmt;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::Localized;

pub const MAX_TEXT_PARAM: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Number,
    Integer,
    Bool,
    Text,
    Select,
}

impl ParamType {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "number" => Self::Number,
            "integer" => Self::Integer,
            "bool" => Self::Bool,
            "text" => Self::Text,
            "select" => Self::Select,
            _ => return None,
        })
    }
}

/// A user-tunable parameter from `manifest.yaml` (the ⚙ dialog in the terminal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: ParamType,
    #[serde(default)]
    pub title: Localized,
    pub default: Value,
    #[serde(default)]
    pub min: Option<Decimal>,
    #[serde(default)]
    pub max: Option<Decimal>,
    #[serde(default)]
    pub options: Vec<String>,
}

/// Why a value does not fit a parameter. Structured rather than text so the terminal can
/// show the reason in the interface language; `Display` is the English wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamProblem {
    ExpectedBool,
    ExpectedText,
    TextTooLong {
        max: usize,
    },
    NotAnOption,
    ExpectedNumber,
    ExpectedInteger,
    OutOfRange {
        min: Option<Decimal>,
        max: Option<Decimal>,
    },
    Unknown,
}

impl fmt::Display for ParamProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedBool => write!(f, "expected true or false"),
            Self::ExpectedText => write!(f, "expected text"),
            Self::TextTooLong { max } => write!(f, "text longer than {max} characters"),
            Self::NotAnOption => write!(f, "not one of the options"),
            Self::ExpectedNumber => write!(f, "expected a number"),
            Self::ExpectedInteger => write!(f, "expected a whole number"),
            Self::OutOfRange { min, max } => match (min, max) {
                (Some(min), Some(max)) => write!(f, "must be within {min}..{max}"),
                (Some(min), None) => write!(f, "must be at least {min}"),
                (None, Some(max)) => write!(f, "must be at most {max}"),
                (None, None) => write!(f, "out of range"),
            },
            Self::Unknown => write!(f, "unknown param"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{key}: {problem}")]
pub struct ParamError {
    pub key: String,
    pub problem: ParamProblem,
}

impl Param {
    /// Coerces a value to the parameter's type: an integer comes back as a whole JSON number
    /// (`5.0` → `5`) so the plugin never gets a fraction where it expects an integer.
    pub fn validate(&self, value: &Value) -> Result<Value, ParamError> {
        self.check(value).map_err(|problem| ParamError {
            key: self.key.clone(),
            problem,
        })
    }

    fn check(&self, value: &Value) -> Result<Value, ParamProblem> {
        match self.kind {
            ParamType::Bool => value
                .as_bool()
                .map(Value::Bool)
                .ok_or(ParamProblem::ExpectedBool),
            ParamType::Text => match value.as_str() {
                Some(text) if text.chars().count() <= MAX_TEXT_PARAM => {
                    Ok(Value::String(text.to_string()))
                }
                Some(_) => Err(ParamProblem::TextTooLong {
                    max: MAX_TEXT_PARAM,
                }),
                None => Err(ParamProblem::ExpectedText),
            },
            ParamType::Select => match value.as_str() {
                Some(option) if self.options.iter().any(|o| o == option) => {
                    Ok(Value::String(option.to_string()))
                }
                _ => Err(ParamProblem::NotAnOption),
            },
            ParamType::Number | ParamType::Integer => {
                let number = match value {
                    Value::Number(number) => decimal_from_number(number),
                    _ => None,
                }
                .ok_or(ParamProblem::ExpectedNumber)?;
                if self.kind == ParamType::Integer && !number.fract().is_zero() {
                    return Err(ParamProblem::ExpectedInteger);
                }
                let out_of_range = ParamProblem::OutOfRange {
                    min: self.min,
                    max: self.max,
                };
                if self.min.is_some_and(|min| number < min)
                    || self.max.is_some_and(|max| number > max)
                {
                    return Err(out_of_range);
                }
                match self.kind {
                    ParamType::Integer => number.to_i64().map(Value::from).ok_or(out_of_range),
                    _ => Ok(value.clone()),
                }
            }
        }
    }
}

/// Exact decimal of a JSON number: integers as is, floats through their shortest text form,
/// so `0.1` stays `0.1` instead of the nearest binary fraction.
pub fn decimal_from_number(number: &serde_json::Number) -> Option<Decimal> {
    if let Some(int) = number.as_i64() {
        return Some(Decimal::from(int));
    }
    let text = number.to_string();
    Decimal::from_str_exact(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .ok()
}
