use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params(Map<String, Value>);

impl Params {
    pub fn new(map: Map<String, Value>) -> Self {
        Self(map)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        match self.0.get(key)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        match self.0.get(key)? {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.0.get(key)?.as_bool()
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.as_str()
    }

    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.f64(key).unwrap_or(default)
    }

    pub fn i64_or(&self, key: &str, default: i64) -> i64 {
        self.i64(key).unwrap_or(default)
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.bool(key).unwrap_or(default)
    }

    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
}
