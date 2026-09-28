use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::abi::{ExchangeMarket, Market, MarketRef};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Row {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<Market>,
    #[serde(default)]
    pub cells: BTreeMap<String, Cell>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rank: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<u32>,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

impl Row {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }

    pub fn market_ref(mut self, m: &MarketRef) -> Self {
        self.symbol = Some(m.symbol.clone());
        self.exchange = Some(m.exchange.clone());
        self.market = Some(m.market);
        self
    }

    pub fn symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    pub fn exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = Some(exchange.into());
        self
    }

    pub fn market(mut self, market: Market) -> Self {
        self.market = Some(market);
        self
    }

    pub fn cell(mut self, column: impl Into<String>, value: impl Into<Cell>) -> Self {
        self.cells.insert(column.into(), value.into());
        self
    }

    pub fn rank(mut self, rank: i32) -> Self {
        self.rank = rank;
        self
    }

    pub fn ttl_s(mut self, ttl_s: u32) -> Self {
        self.ttl_s = Some(ttl_s);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Pos,
    Neg,
    Muted,
    Warn,
    Accent,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellValue {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Markets(Vec<ExchangeMarket>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cell {
    pub v: CellValue,
    pub tone: Option<Tone>,
    pub text: Option<String>,
}

impl Cell {
    pub fn new(v: impl Into<CellValue>) -> Self {
        Self {
            v: v.into(),
            tone: None,
            text: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn signed(v: f64) -> Self {
        let cell = Self::new(v);
        match v {
            v if v > 0.0 => cell.tone(Tone::Pos),
            v if v < 0.0 => cell.tone(Tone::Neg),
            _ => cell,
        }
    }

    pub fn muted(v: impl Into<CellValue>) -> Self {
        Self::new(v).tone(Tone::Muted)
    }

    pub fn markets(markets: impl IntoIterator<Item = ExchangeMarket>) -> Self {
        Self::new(CellValue::Markets(markets.into_iter().collect()))
    }

    fn is_plain(&self) -> bool {
        self.tone.is_none() && self.text.is_none() && !matches!(self.v, CellValue::Markets(_))
    }
}

#[derive(Serialize)]
struct StyledRef<'a> {
    v: &'a CellValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    tone: Option<Tone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
}

impl Serialize for Cell {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.is_plain() {
            return self.v.serialize(s);
        }
        StyledRef {
            v: &self.v,
            tone: self.tone,
            text: self.text.as_deref(),
        }
        .serialize(s)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CellRepr {
    Styled {
        #[serde(default)]
        v: CellValue,
        #[serde(default)]
        tone: Option<Tone>,
        #[serde(default)]
        text: Option<String>,
    },
    Plain(CellValue),
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match CellRepr::deserialize(d)? {
            CellRepr::Styled { v, tone, text } => Self { v, tone, text },
            CellRepr::Plain(v) => Self::new(v),
        })
    }
}

macro_rules! cell_from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for CellValue {
            fn from(v: $t) -> Self {
                Self::Number(v as f64)
            }
        }
        impl From<$t> for Cell {
            fn from(v: $t) -> Self {
                Self::new(v)
            }
        }
    )*};
}

cell_from_number!(f64, f32, i64, i32, u64, u32, usize);

impl From<bool> for CellValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<&str> for CellValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_string())
    }
}

impl From<String> for CellValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<Vec<ExchangeMarket>> for CellValue {
    fn from(v: Vec<ExchangeMarket>) -> Self {
        Self::Markets(v)
    }
}

impl From<bool> for Cell {
    fn from(v: bool) -> Self {
        Self::new(v)
    }
}

impl From<&str> for Cell {
    fn from(v: &str) -> Self {
        Self::new(v)
    }
}

impl From<String> for Cell {
    fn from(v: String) -> Self {
        Self::new(v)
    }
}

impl From<CellValue> for Cell {
    fn from(v: CellValue) -> Self {
        Self::new(v)
    }
}

impl<T: Into<Cell>> From<Option<T>> for Cell {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_and_styled_cells_round_trip() {
        let row = Row::new("binance:BTCUSDT")
            .market_ref(&MarketRef::new("binance", Market::Futures, "BTCUSDT"))
            .cell("oi", 1.5e9)
            .cell("chg", Cell::signed(-2.0))
            .cell("name", "BTC")
            .cell(
                "where",
                Cell::markets([ExchangeMarket::new("bybit", Market::Spot)]),
            )
            .cell("none", Option::<f64>::None);
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(
            v,
            json!({
                "key": "binance:BTCUSDT",
                "symbol": "BTCUSDT",
                "exchange": "binance",
                "market": "futures",
                "cells": {
                    "chg": {"v": -2.0, "tone": "neg"},
                    "name": "BTC",
                    "none": null,
                    "oi": 1.5e9,
                    "where": {"v": [{"exchange": "bybit", "market": "spot"}]}
                }
            })
        );
        let back: Row = serde_json::from_value(v).unwrap();
        assert_eq!(back, row);
    }
}
