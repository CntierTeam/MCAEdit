use crate::error::{Error, Result};
use fastnbt::Value;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub fn nbt_to_json(value: Value) -> Result<JsonValue> {
    Ok(match value {
        Value::Byte(v) => JsonValue::from(v),
        Value::Short(v) => JsonValue::from(v),
        Value::Int(v) => JsonValue::from(v),
        Value::Long(v) => JsonValue::from(v),
        Value::Float(v) => JsonValue::from(v as f64),
        Value::Double(v) => JsonValue::from(v),
        Value::ByteArray(v) => {
            JsonValue::Array(v.iter().copied().map(JsonValue::from).collect())
        }
        Value::IntArray(v) => {
            JsonValue::Array(v.iter().copied().map(JsonValue::from).collect())
        }
        Value::LongArray(v) => {
            JsonValue::Array(v.iter().copied().map(JsonValue::from).collect())
        }
        Value::String(v) => JsonValue::String(v),
        Value::List(v) => {
            JsonValue::Array(v.into_iter().map(nbt_to_json).collect::<Result<Vec<_>>>()?)
        }
        Value::Compound(v) => {
            let mut map = serde_json::Map::new();
            for (k, val) in v {
                map.insert(k, nbt_to_json(val)?);
            }
            JsonValue::Object(map)
        }
    })
}

pub fn json_to_nbt(value: &JsonValue) -> Result<Value> {
    Ok(match value {
        JsonValue::Null => Value::String(String::new()),
        JsonValue::Bool(b) => Value::Byte(if *b { 1 } else { 0 }),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Value::Int(i as i32)
                } else {
                    Value::Long(i)
                }
            } else if let Some(f) = n.as_f64() {
                Value::Double(f)
            } else {
                return Err(Error::msg(format!("bad number {n}")));
            }
        }
        JsonValue::String(s) => Value::String(s.clone()),
        JsonValue::Array(arr) => {
            if !arr.is_empty()
                && arr
                    .iter()
                    .all(|v| v.as_i64().is_some() || v.as_u64().is_some())
            {
                let longs: Vec<i64> = arr
                    .iter()
                    .map(|v| v.as_i64().unwrap_or_else(|| v.as_u64().unwrap() as i64))
                    .collect();
                Value::LongArray(fastnbt::LongArray::new(longs))
            } else {
                Value::List(arr.iter().map(json_to_nbt).collect::<Result<Vec<_>>>()?)
            }
        }
        JsonValue::Object(map) => {
            let mut compound = BTreeMap::new();
            for (k, v) in map {
                compound.insert(k.clone(), json_to_nbt(v)?);
            }
            Value::Compound(compound.into_iter().collect())
        }
    })
}
