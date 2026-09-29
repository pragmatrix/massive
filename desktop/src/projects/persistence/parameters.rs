//! The spawn-parameter conversation: JSON parameter values (the live model's
//! instance parameters, `InstanceParameters`) as KDL child nodes, and back.

use log::warn;
use serde_json::Value;

use kdl::{KdlNode, KdlValue};

use super::document::fresh_node_format;

/// A parameter value as a KDL child node: the node's name is the parameter's key,
/// its unnamed entries carry the value (arrays become several entries).
pub(super) fn params_node(key: &str, value: &Value, indent: String) -> KdlNode {
    let mut node = KdlNode::new(key);
    match value {
        Value::Null => {}
        Value::Bool(value) => node.push(*value),
        Value::Number(number) => node.push(number_value(number)),
        Value::String(value) => node.push(value.as_str()),
        Value::Array(values) => {
            for value in values {
                match value {
                    Value::String(value) => node.push(value.as_str()),
                    Value::Bool(value) => node.push(*value),
                    Value::Number(number) => node.push(number_value(number)),
                    other => warn!("Skipping unsupported parameter value for '{key}': {other}"),
                }
            }
        }
        other => warn!("Skipping unsupported parameter value for '{key}': {other}"),
    }
    node.set_format(fresh_node_format(&indent));
    node
}

/// Reads a parameter node's value from its unnamed entries: none means `true`
/// (a flag), several means an array.
pub(super) fn params_value(node: &KdlNode) -> Value {
    let values: Vec<Value> = node
        .entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(|entry| json_value(entry.value()))
        .collect();
    match values.as_slice() {
        [] => Value::Bool(true),
        [single] => single.clone(),
        _ => Value::Array(values),
    }
}

fn json_value(value: &KdlValue) -> Value {
    match value {
        KdlValue::String(value) => Value::String(value.into()),
        KdlValue::Integer(value) => match i64::try_from(*value) {
            Ok(value) => Value::Number(value.into()),
            // Out-of-i64-range integers lose precision as JSON numbers.
            Err(_) => {
                serde_json::Number::from_f64(*value as f64).map_or(Value::Null, Value::Number)
            }
        },
        KdlValue::Float(value) => {
            serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
        }
        KdlValue::Bool(value) => Value::Bool(*value),
        KdlValue::Null => Value::Null,
    }
}

fn number_value(number: &serde_json::Number) -> KdlValue {
    if let Some(integer) = number.as_i64() {
        KdlValue::from(i128::from(integer))
    } else if let Some(float) = number.as_f64() {
        KdlValue::from(float)
    } else {
        KdlValue::Null
    }
}
