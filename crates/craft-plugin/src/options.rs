//! Bounded, offline validation for the JSON Schema fields used by format options.
use crate::{HostError, Result};
use serde_json::Value;

fn invalid(path: &str, reason: &str) -> HostError {
    HostError::InvalidInput { field: path.into(), reason: reason.into() }
}

pub(crate) fn validate(schema: &Value, value: &Value) -> Result<()> {
    check(schema, schema, value, "options", 0, &mut 8192)
}

fn check(root: &Value, schema: &Value, value: &Value, path: &str, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 32 || *budget == 0 {
        return Err(HostError::LimitExceeded { limit: "option schema validation depth or size".into() });
    }
    *budget -= 1;
    if schema == &Value::Bool(true) {
        return Ok(());
    }
    if schema == &Value::Bool(false) {
        return Err(invalid(path, "value is not allowed"));
    }
    if !schema.is_object() {
        return Err(invalid(path, "schema reference must resolve to an object or boolean schema"));
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = reference
            .strip_prefix('#')
            .and_then(|pointer| root.pointer(pointer))
            .ok_or_else(|| invalid(path, "only existing local schema references are supported"))?;
        check(root, target, value, path, depth + 1, budget)?;
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        return Err(invalid(path, "value is not in the declared enum"));
    }
    if let Some(expected) = schema.get("const")
        && expected != value
    {
        return Err(invalid(path, "value differs from the declared constant"));
    }
    if let Some(kind) = schema.get("type") {
        let matches = |kind: &str| match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        let valid = kind.as_str().is_some_and(matches) || kind.as_array().is_some_and(|kinds| kinds.iter().filter_map(Value::as_str).any(matches));
        if !valid {
            return Err(invalid(path, "value does not match the declared type"));
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            let mut successes = 0;
            for branch in branches {
                match check(root, branch, value, path, depth + 1, budget) {
                    Ok(()) => successes += 1,
                    Err(error @ HostError::LimitExceeded { .. }) => return Err(error),
                    Err(_) => {}
                }
            }
            if (keyword == "allOf" && successes != branches.len()) || (keyword == "anyOf" && successes == 0) || (keyword == "oneOf" && successes != 1)
            {
                return Err(invalid(path, "value does not match the declared schema alternatives"));
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(invalid(&format!("{path}.{key}"), "required option is missing"));
                }
            }
        }
        for (key, value) in object {
            if let Some(property) = schema.get("properties").and_then(|properties| properties.get(key)) {
                check(root, property, value, &format!("{path}.{key}"), depth + 1, budget)?;
            } else if let Some(additional) = schema.get("additionalProperties") {
                check(root, additional, value, &format!("{path}.{key}"), depth + 1, budget)?;
            }
        }
    }
    if let Some(values) = value.as_array() {
        bounds(schema, values.len() as f64, "minItems", "maxItems", path)?;
        if let Some(items) = schema.get("items") {
            for (index, value) in values.iter().enumerate() {
                check(root, items, value, &format!("{path}[{index}]"), depth + 1, budget)?;
            }
        }
    }
    if let Some(text) = value.as_str() {
        bounds(schema, text.chars().count() as f64, "minLength", "maxLength", path)?;
    }
    if value.is_number() {
        use std::cmp::Ordering;
        for (keyword, forbidden, exclusive) in [
            ("minimum", Ordering::Less, false),
            ("maximum", Ordering::Greater, false),
            ("exclusiveMinimum", Ordering::Less, true),
            ("exclusiveMaximum", Ordering::Greater, true),
        ] {
            if let Some(order) = schema.get(keyword).and_then(|bound| number_order(value, bound))
                && (order == forbidden || (exclusive && order == Ordering::Equal))
            {
                return Err(invalid(path, "number exceeds the declared bounds"));
            }
        }
    }
    Ok(())
}

fn number_order(value: &Value, bound: &Value) -> Option<std::cmp::Ordering> {
    // Preserve all 64 integer bits for integer limits, including unsigned seeds.
    let integer = |value: &Value| value.as_i64().map(i128::from).or_else(|| value.as_u64().map(i128::from));
    match (integer(value), integer(bound)) {
        (Some(value), Some(bound)) => Some(value.cmp(&bound)),
        _ => value.as_f64()?.partial_cmp(&bound.as_f64()?),
    }
}

fn bounds(schema: &Value, number: f64, min: &str, max: &str, path: &str) -> Result<()> {
    if schema.get(min).and_then(Value::as_f64).is_some_and(|min| number < min)
        || schema.get(max).and_then(Value::as_f64).is_some_and(|max| number > max)
    {
        return Err(invalid(path, "value exceeds the declared bounds"));
    }
    Ok(())
}

/// Refuse unsupported validation keywords instead of silently accepting unchecked options.
pub(crate) fn validate_schema(schema: &Value) -> Result<()> {
    fn walk(schema: &Value, depth: usize) -> Result<()> {
        let bad = |reason: &str| HostError::InvalidManifest { field: "runtime.entries.input_schema".into(), reason: reason.into() };
        if depth > 32 {
            return Err(bad("option schema exceeds depth 32"));
        }
        if schema.is_boolean() {
            return Ok(());
        }
        let object = schema.as_object().ok_or_else(|| bad("schema must be an object or boolean"))?;
        for (key, value) in object {
            match key.as_str() {
                "$schema" | "$id" | "$comment" | "title" | "description" | "default" | "examples" | "deprecated" | "readOnly" | "writeOnly"
                | "const" => {}
                "$ref" => {
                    if !value.as_str().is_some_and(|reference| reference.starts_with('#')) {
                        return Err(bad("schema references must be local"));
                    }
                }
                "type" => {
                    let supported = |kind: &str| matches!(kind, "object" | "array" | "string" | "number" | "integer" | "boolean" | "null");
                    if !value.as_str().is_some_and(supported)
                        && !value.as_array().is_some_and(|types| !types.is_empty() && types.iter().all(|kind| kind.as_str().is_some_and(supported)))
                    {
                        return Err(bad("unsupported option type"));
                    }
                }
                "properties" | "$defs" => {
                    for child in value.as_object().ok_or_else(|| bad("properties and $defs must be objects"))?.values() {
                        walk(child, depth + 1)?;
                    }
                }
                "items" | "additionalProperties" => walk(value, depth + 1)?,
                "allOf" | "anyOf" | "oneOf" => {
                    let children = value
                        .as_array()
                        .filter(|children| !children.is_empty())
                        .ok_or_else(|| bad("schema alternatives must be a nonempty array"))?;
                    for child in children {
                        walk(child, depth + 1)?;
                    }
                }
                "required" => {
                    if !value.as_array().is_some_and(|keys| keys.iter().all(Value::is_string)) {
                        return Err(bad("required must list option names"));
                    }
                }
                "enum" => {
                    if !value.as_array().is_some_and(|values| !values.is_empty()) {
                        return Err(bad("enum must be a nonempty array"));
                    }
                }
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => {
                    if !value.is_number() {
                        return Err(bad("numeric bounds must be numbers"));
                    }
                }
                "minItems" | "maxItems" | "minLength" | "maxLength" => {
                    if value.as_u64().is_none() {
                        return Err(bad("length bounds must be nonnegative integers"));
                    }
                }
                _ => return Err(bad(&format!("unsupported option schema keyword: {key}"))),
            }
        }
        Ok(())
    }
    walk(schema, 0)
}
