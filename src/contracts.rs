//! CEL contracts (§3.2).
//!
//! CEL is non-Turing-complete, terminates by construction, and is a published spec, so a
//! contributor can read a contract without learning a project-specific DSL.
//!
//! Two rules govern everything here:
//!
//! 1. **Fail closed.** An expression that errors — undefined field, type mismatch, division
//!    by zero — or that returns a non-boolean has *not held*. A contract that cannot be
//!    evaluated is a contract that did not hold. If an evaluation error ever falls through
//!    as success the entire guarantee is gone, so `Verdict` has no variant that could be
//!    mistaken for a pass.
//! 2. **Types come from the JSON Schema**, never from the incoming JSON. `integer` becomes
//!    a CEL int and `number` becomes a CEL double because the schema says so, so an
//!    `attack_rating` of exactly `512` does not silently become an int and stop comparing
//!    against a float bound.

use crate::schema;
use cel::objects::{Key, Map};
use cel::{Context, Program, Value as Cel};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::Arc;

/// The outcome of evaluating one contract expression. There is no third state: `Held` is the
/// only way past a contract.
#[derive(Debug)]
pub enum Verdict {
    Held,
    Failed,
    /// The expression could not be evaluated to a boolean. Distinct from `Failed` because it
    /// carries a different exit code (15, a refusal) than a genuine violation.
    Unevaluable(String),
}

pub struct Contract {
    pub source: String,
    /// Author-written explanation, used verbatim as the refusal reason.
    ///
    /// A failed precondition is a routing correction (§4.2): the reader can act on
    /// "unknown weapon 'lothric sword'; call weapon-lookup for valid names" and cannot act
    /// on the expression that produced it. Absent a message the expression is reported,
    /// which is honest but rarely useful.
    pub message: Option<String>,
    program: Program,
}

impl Contract {
    pub fn compile(source: &str, message: Option<String>) -> std::result::Result<Contract, String> {
        Program::compile(source)
            .map(|program| Contract {
                source: source.to_string(),
                message,
                program,
            })
            .map_err(|e| format!("{e}"))
    }

    /// What to tell a caller when this contract does not hold.
    pub fn explain(&self, fallback: &str) -> String {
        match &self.message {
            Some(message) => message.clone(),
            None => {
                let source = self.source.split_whitespace().collect::<Vec<_>>().join(" ");
                format!("{fallback}: {source}")
            }
        }
    }

    pub fn references(&self, name: &str) -> bool {
        self.program.references().has_variable(name)
    }

    pub fn evaluate(&self, ctx: &Context) -> Verdict {
        match self.program.execute(ctx) {
            Ok(Cel::Bool(true)) => Verdict::Held,
            Ok(Cel::Bool(false)) => Verdict::Failed,
            // A contract that evaluates to a non-boolean is not a contract. Treating a
            // truthy value as a pass is exactly the fall-through this module exists to
            // prevent.
            Ok(other) => Verdict::Unevaluable(format!(
                "expression returned {}, not a boolean",
                type_name(&other)
            )),
            Err(e) => Verdict::Unevaluable(format!("{e}")),
        }
    }
}

fn type_name(value: &Cel) -> &'static str {
    match value {
        Cel::List(_) => "a list",
        Cel::Map(_) => "a map",
        Cel::Function(..) => "a function",
        Cel::Int(_) => "an int",
        Cel::UInt(_) => "a uint",
        Cel::Float(_) => "a double",
        Cel::String(_) => "a string",
        Cel::Bytes(_) => "bytes",
        Cel::Bool(_) => "a bool",
        Cel::Null => "null",
        _ => "an unsupported type",
    }
}

/// Convert JSON into CEL values, taking types from `schema` rather than from the shape of
/// the data. `root` is the schema document the local `$ref`s resolve against.
pub fn to_cel(value: &Json, schema: Option<&Json>, root: &Json) -> Cel {
    let schema = schema.map(|s| schema::deref(s, root));
    convert(value, schema, root, 0)
}

fn convert(value: &Json, schema: Option<&Json>, root: &Json, depth: usize) -> Cel {
    if depth > 64 {
        return Cel::Null;
    }
    match value {
        Json::Null => Cel::Null,
        Json::Bool(b) => Cel::Bool(*b),
        Json::String(s) => Cel::String(Arc::new(s.clone())),
        Json::Number(n) => number(n, schema),
        Json::Array(items) => {
            let item_schema = schema
                .and_then(|s| s.get("items"))
                .map(|s| schema::deref(s, root));
            let converted = items
                .iter()
                .map(|i| convert(i, item_schema, root, depth + 1))
                .collect();
            Cel::List(Arc::new(converted))
        }
        Json::Object(fields) => {
            let mut map: HashMap<Key, Cel> = HashMap::with_capacity(fields.len());
            for (key, sub) in fields {
                let sub_schema = field_schema(schema, key, root);
                map.insert(
                    Key::String(Arc::new(key.clone())),
                    convert(sub, sub_schema.as_ref(), root, depth + 1),
                );
            }
            Cel::Map(Map { map: Arc::new(map) })
        }
    }
}

/// The schema governing one object field: `properties`, then `additionalProperties` when it
/// is a schema rather than a boolean.
fn field_schema(schema: Option<&Json>, key: &str, root: &Json) -> Option<Json> {
    let schema = schema?;
    if let Some(prop) = schema.get("properties").and_then(|p| p.get(key)) {
        return Some(schema::deref(prop, root).clone());
    }
    match schema.get("additionalProperties") {
        Some(Json::Object(_)) => {
            let sub = schema.get("additionalProperties")?;
            Some(schema::deref(sub, root).clone())
        }
        _ => None,
    }
}

/// The integer/float decision — the most likely source of spurious contract failures, so it
/// is made in exactly one place.
fn number(n: &serde_json::Number, schema: Option<&Json>) -> Cel {
    let declared = schema.map(schema::types).unwrap_or_default();

    if declared.contains(&"integer") {
        // The value already passed schema validation, so it *is* an integer; JSON just may
        // have spelled it `40.0`.
        if let Some(i) = n.as_i64() {
            return Cel::Int(i);
        }
        if let Some(f) = n.as_f64() {
            if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                return Cel::Int(f as i64);
            }
        }
    }
    if declared.contains(&"number") {
        if let Some(f) = n.as_f64() {
            return Cel::Float(f);
        }
    }

    // Undeclared: fall back to the shape of the literal. Reached only for fields the schema
    // does not describe, which `describe` reports as uncovered.
    if let Some(i) = n.as_i64() {
        Cel::Int(i)
    } else if let Some(f) = n.as_f64() {
        Cel::Float(f)
    } else {
        Cel::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn eval(expr: &str, vars: &[(&str, Cel)]) -> Verdict {
        let contract = Contract::compile(expr, None).expect("compiles");
        let mut ctx = Context::default();
        for (name, value) in vars {
            ctx.add_variable_from_value(*name, value.clone());
        }
        contract.evaluate(&ctx)
    }

    fn held(v: Verdict) -> bool {
        matches!(v, Verdict::Held)
    }

    fn unevaluable(v: Verdict) -> bool {
        matches!(v, Verdict::Unevaluable(_))
    }

    // --- the fail-closed rule (§3.2). If any of these ever pass, the guarantee is gone. ---

    #[test]
    fn undefined_field_is_unevaluable_not_a_pass() {
        let result = to_cel(&json!({"a": 1}), None, &Json::Null);
        assert!(unevaluable(eval(
            "result.missing > 0",
            &[("result", result)]
        )));
    }

    #[test]
    fn undeclared_variable_is_unevaluable_not_a_pass() {
        assert!(unevaluable(eval("nonexistent > 0", &[])));
    }

    #[test]
    fn type_mismatch_is_unevaluable_not_a_pass() {
        let result = to_cel(&json!({"name": "x"}), None, &Json::Null);
        assert!(unevaluable(eval("result.name > 0", &[("result", result)])));
    }

    #[test]
    fn division_by_zero_is_unevaluable_not_a_pass() {
        let result = to_cel(&json!({"a": 1, "b": 0}), None, &Json::Null);
        assert!(unevaluable(eval(
            "result.a / result.b > 0",
            &[("result", result)]
        )));
    }

    #[test]
    fn non_boolean_result_is_unevaluable_not_a_pass() {
        let result = to_cel(&json!({"a": 1}), None, &Json::Null);
        // Truthiness must not be accepted as a verdict.
        assert!(unevaluable(eval("result.a", &[("result", result.clone())])));
        assert!(unevaluable(eval("result", &[("result", result)])));
    }

    #[test]
    fn genuine_violation_is_failed_not_unevaluable() {
        let result = to_cel(&json!({"a": 1}), None, &Json::Null);
        assert!(matches!(
            eval("result.a > 5", &[("result", result)]),
            Verdict::Failed
        ));
    }

    // --- schema-driven typing (§3.2) ---

    #[test]
    fn integer_schema_yields_int_even_when_json_says_float() {
        let s = json!({"type": "object", "properties": {"n": {"type": "integer"}}});
        assert_eq!(
            to_cel(&json!({"n": 40.0}), Some(&s), &s),
            Cel::Map(Map {
                map: Arc::new(HashMap::from([(
                    Key::String(Arc::new("n".into())),
                    Cel::Int(40)
                )]))
            })
        );
    }

    #[test]
    fn number_schema_yields_float_even_when_json_says_int() {
        let s = json!({"type": "object", "properties": {"n": {"type": "number"}}});
        let converted = to_cel(&json!({"n": 512}), Some(&s), &s);
        // Without this, `result.attack_rating > 0.0` would compare an int to a double.
        assert!(matches!(
            eval("v.n > 0.0", &[("v", converted)]),
            Verdict::Held
        ));
    }

    /// The CEL builtins the example collections rely on, so a node author can copy them with
    /// confidence and a dependency bump that drops one fails here rather than in someone's
    /// manifest at exit 15.
    #[test]
    fn documented_builtins_are_available() {
        let s = json!({
            "type": "object",
            "properties": {"id": {"type": "string"}, "tier": {"type": "string"}}
        });
        let v = to_cel(&json!({"id": "T-1001", "tier": "pro"}), Some(&s), &s);

        for expr in [
            r#"v.id.startsWith("T-")"#,
            r#"v.id.endsWith("1001")"#,
            r#"v.id.contains("-")"#,
            r#"size(v.id) == 6"#,
            r#"v.tier in ["free", "pro", "enterprise"]"#,
            r#"v.tier != "free""#,
            r#"["free", "pro"].exists(t, t == v.tier)"#,
        ] {
            assert!(
                matches!(eval(expr, &[("v", v.clone())]), Verdict::Held),
                "{expr} should hold, got {:?}",
                eval(expr, &[("v", v.clone())])
            );
        }
    }

    #[test]
    fn schema_typing_survives_a_local_ref() {
        let s = json!({
            "type": "object",
            "properties": {"stats": {"$ref": "#/$defs/stats"}},
            "$defs": {
                "stats": {"type": "object", "properties": {"strength": {"type": "integer"}}}
            }
        });
        let converted = to_cel(&json!({"stats": {"strength": 16.0}}), Some(&s), &s);
        assert!(held(eval("v.stats.strength == 16", &[("v", converted)])));
    }

    /// Mixed int/double comparison is legal CEL. If this ever regresses, ordinary contracts
    /// like `result.total <= input.budget` start refusing with exit 15 for no visible reason.
    #[test]
    fn int_and_double_compare_across_types() {
        let s = json!({
            "type": "object",
            "properties": {"i": {"type": "integer"}, "f": {"type": "number"}}
        });
        let v = to_cel(&json!({"i": 10, "f": 2.5}), Some(&s), &s);
        assert!(held(eval("v.i > v.f", &[("v", v.clone())])));
        assert!(held(eval("v.f < v.i", &[("v", v.clone())])));
        assert!(held(eval("v.i <= 10.0", &[("v", v)])));
    }
}
