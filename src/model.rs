use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub command: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub object_id: i64,
    pub revision: i64,
    pub full: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Output {
    pub kind: String,
    pub items: Vec<Value>,
    pub more: bool,
    pub notices: Vec<String>,
    #[serde(skip)]
    pub receipts: Vec<Receipt>,
    #[serde(skip)]
    pub notification_ids: Vec<i64>,
}

impl Output {
    pub fn one(kind: &str, item: Value) -> Self {
        Self { kind: kind.into(), items: vec![item], ..Self::default() }
    }
    pub fn list(kind: &str, items: Vec<Value>) -> Self {
        Self { kind: kind.into(), items, ..Self::default() }
    }
}

pub fn string(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing {key}"))
}
pub fn id(args: &Value, key: &str) -> anyhow::Result<i64> {
    args.get(key).and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("missing integer {key}"))
}
pub fn limit(args: &Value) -> usize {
    args.get("limit").and_then(Value::as_u64).unwrap_or(50).min(100_000) as usize
}
