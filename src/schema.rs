//! JSON Schema validation (draft 2020-12) and the schema-derived facts other modules need.
//!
//! The schema is the single source of truth for types (§3.2): the CEL environment is built
//! from it rather than inferred from the payload, so a payload that disagrees with its
//! schema is a schema violation rather than a mysterious contract failure.

use crate::error::{Result, VouchError};
use jsonschema::Validator;
use serde_json::Value as Json;

pub fn compile(schema: &Json, label: &str) -> Result<Validator> {
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(schema)
        .map_err(|e| VouchError::error(format!("{label} is not a valid JSON Schema: {e}")))
}

/// All validation errors, as `instance/path: message` strings. Reporting every failure at
/// once means an agent can fix a call in one retry instead of one per field.
pub fn errors(validator: &Validator, instance: &Json) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                e.to_string()
            } else {
                format!("{path}: {e}")
            }
        })
        .collect()
}

/// Resolve a local `$ref` (`#/$defs/...`, `#/definitions/...`, or any `#/`-pointer) against
/// the root schema. Remote refs are not resolved — the validator is built without network or
/// filesystem retrieval, so they would fail at compile time anyway.
pub fn deref<'a>(schema: &'a Json, root: &'a Json) -> &'a Json {
    let mut current = schema;
    // Bounded: a self-referential $ref chain must not spin.
    for _ in 0..32 {
        let Some(reference) = current.get("$ref").and_then(Json::as_str) else {
            return current;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return current;
        };
        match root.pointer(pointer) {
            Some(target) => current = target,
            None => return current,
        }
    }
    current
}

/// The declared type(s) of a schema node, as a slice-friendly list. `type` may be a string
/// or an array of strings.
pub fn types(schema: &Json) -> Vec<&str> {
    match schema.get("type") {
        Some(Json::String(t)) => vec![t.as_str()],
        Some(Json::Array(ts)) => ts.iter().filter_map(Json::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Dotted paths of every numeric leaf the schema declares, prefixed with `prefix`
/// (e.g. `result.stats.strength`). Used to report contract strength (§3.3).
pub fn numeric_paths(schema: &Json, root: &Json, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_numeric(schema, root, prefix, &mut out, 0);
    out
}

fn collect_numeric(schema: &Json, root: &Json, path: &str, out: &mut Vec<String>, depth: usize) {
    if depth > 16 {
        return;
    }
    let schema = deref(schema, root);

    if types(schema)
        .iter()
        .any(|t| *t == "number" || *t == "integer")
    {
        out.push(path.to_string());
        return;
    }
    if let Some(props) = schema.get("properties").and_then(Json::as_object) {
        for (key, sub) in props {
            collect_numeric(sub, root, &format!("{path}.{key}"), out, depth + 1);
        }
    }
    if let Some(items) = schema.get("items") {
        collect_numeric(items, root, &format!("{path}[]"), out, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// §8 asks for an explicit decision about `NaN` and `Infinity`. The decision is that
    /// they are rejected at the JSON boundary, before any contract sees them, and the type
    /// system makes that airtight: `serde_json::Value` cannot hold a non-finite number.
    /// There is deliberately no downstream non-finite check, because nothing could reach it.
    #[test]
    fn non_finite_numbers_cannot_enter_the_value_tree() {
        assert!(serde_json::Number::from_f64(f64::INFINITY).is_none());
        assert!(serde_json::Number::from_f64(f64::NAN).is_none());
        // The parser rejects every spelling a node might emit.
        for literal in [r#"{"x": NaN}"#, r#"{"x": Infinity}"#, r#"{"x": 1e400}"#] {
            assert!(
                serde_json::from_str::<Json>(literal).is_err(),
                "{literal} must not parse"
            );
        }
    }

    #[test]
    fn numeric_paths_walks_local_refs() {
        let schema = json!({
            "type": "object",
            "properties": {
                "stats": {"$ref": "#/$defs/stats"},
                "name": {"type": "string"}
            },
            "$defs": {
                "stats": {
                    "type": "object",
                    "properties": {"strength": {"type": "integer"}, "ar": {"type": "number"}}
                }
            }
        });
        let mut found = numeric_paths(&schema, &schema, "result");
        found.sort();
        assert_eq!(found, vec!["result.stats.ar", "result.stats.strength"]);
    }
}
