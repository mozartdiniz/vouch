//! Where an agent's inputs came from (§6.3).
//!
//! `attest` checks the far end of a call chain: every figure an agent *writes* must have come
//! back from a node. That leaves the near end open. An agent can pass a correct node the
//! wrong thing — "the last two months" worked out by hand and off by one, a company name
//! "tidied" into a different company — and the node will compute faithfully on it, its
//! contracts will pass, and attestation will vouch for every digit of a wrong answer.
//!
//! Provenance closes that end. A schema property may declare where its value is allowed to
//! come from with `x-source`, and before the call the proposed input is checked against the
//! user's question and the ledger:
//!
//! - `question`: quoted from the user's question. A string must appear in it (ignoring case
//!   and runs of whitespace, at word boundaries); a number must be one of its numerals.
//! - `result`: equal to a value some earlier call returned.
//! - `result:<node>`: returned by that node.
//! - `result:<node>.<path>`: returned by that node at that path, e.g.
//!   `result:resolve-period.start`.
//! - `any`: no constraint, the same as leaving `x-source` out.
//!
//! A property may list several; any one of them suffices. An array is traced element by
//! element. **No model is involved**, exactly as for `attest`: this is string and number
//! comparison over what the ledger recorded.
//!
//! Only `result` values count, never `input`: an input is what some agent chose, so
//! accepting it here would let one guess launder the next.

use crate::attest;
use crate::ledger;
use serde_json::Value as Json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Question,
    Result { node: Option<String>, path: Option<String> },
    Any,
}

impl Source {
    pub fn parse(text: &str) -> Result<Source, String> {
        let text = text.trim();
        match text {
            "question" => return Ok(Source::Question),
            "any" => return Ok(Source::Any),
            "result" => return Ok(Source::Result { node: None, path: None }),
            _ => {}
        }
        let Some(rest) = text.strip_prefix("result:") else {
            return Err(format!(
                "unknown source \"{text}\": use question, result, result:<node>, \
                 result:<node>.<path> or any"
            ));
        };
        let (node, path) = match rest.split_once('.') {
            Some((node, path)) => (node, Some(path.to_string())),
            None => (rest, None),
        };
        if node.is_empty() || path.as_deref() == Some("") {
            return Err(format!("source \"{text}\" is missing a node name or a path"));
        }
        Ok(Source::Result { node: Some(node.to_string()), path })
    }
}

/// The verdict on one declared property.
#[derive(Debug)]
pub struct FieldCheck {
    pub field: String,
    pub value: Json,
    pub sources: Vec<String>,
    /// What each traced value was found as: `question`, or `<entry>:<node> result.<path>`.
    pub traced_to: Vec<String>,
    /// Why it could not be traced; `None` when it was.
    pub problem: Option<String>,
}

#[derive(Debug)]
pub struct Report {
    pub checked: Vec<FieldCheck>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.checked.iter().all(|c| c.problem.is_none())
    }
}

/// The sources a property declares, or an error naming the property. Absent means no
/// constraint, and so does a list containing `any`.
fn declared(field: &str, property: &Json) -> Result<Option<Vec<(String, Source)>>, String> {
    let raw: Vec<String> = match property.get("x-source") {
        None => return Ok(None),
        Some(Json::String(s)) => vec![s.clone()],
        Some(Json::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{field}: x-source entries must be strings"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(format!("{field}: x-source must be a string or a list of strings")),
    };
    if raw.is_empty() {
        return Err(format!("{field}: x-source lists no sources"));
    }
    let sources = raw
        .into_iter()
        .map(|s| Source::parse(&s).map(|p| (s, p)).map_err(|e| format!("{field}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    if sources.iter().any(|(_, s)| *s == Source::Any) {
        return Ok(None);
    }
    Ok(Some(sources))
}

/// Lower-cased, with every run of whitespace made one space.
fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Whether `needle` occurs in `haystack` with no letter or digit glued on either side, so
/// "uchi" is found in "the uchi's weight" but not inside "uchigatana".
fn quoted(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if boundary(before) && boundary(after) {
            return true;
        }
        from = start + needle.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn from_question(value: &Json, question: Option<&str>) -> bool {
    let Some(question) = question else { return false };
    match value {
        Json::String(s) => quoted(&normalise(question), &normalise(s)),
        Json::Number(n) => {
            let Some(v) = n.as_f64() else { return false };
            attest::extract(question).iter().any(|numeral| numeral.value == v)
        }
        _ => false,
    }
}

/// Every leaf an entry's result holds, strings and booleans included, by dotted path.
fn leaves(value: &Json, path: &str, out: &mut BTreeMap<String, Json>) {
    match value {
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                leaves(item, &format!("{path}[{i}]"), out);
            }
        }
        Json::Object(fields) => {
            for (key, sub) in fields {
                leaves(sub, &format!("{path}.{key}"), out);
            }
        }
        Json::Null => {}
        _ => {
            out.insert(path.to_string(), value.clone());
        }
    }
}

fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// Where in the ledger this value was returned, under these constraints.
fn from_results(
    value: &Json,
    node: Option<&str>,
    path: Option<&str>,
    entries: &[Json],
) -> Option<String> {
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.get("node").and_then(Json::as_str).unwrap_or("");
        if node.is_some_and(|n| n != name) {
            continue;
        }
        let Some(result) = entry.get("result") else { continue };
        if let Some(path) = path {
            if ledger::value_at(result, path).is_some_and(|found| same(found, value)) {
                return Some(format!("{i}:{name} result.{path}"));
            }
            continue;
        }
        let mut all = BTreeMap::new();
        leaves(result, "result", &mut all);
        if let Some((at, _)) = all.iter().find(|(_, found)| same(found, value)) {
            return Some(format!("{i}:{name} {at}"));
        }
    }
    None
}

fn trace(
    value: &Json,
    sources: &[(String, Source)],
    question: Option<&str>,
    entries: &[Json],
) -> Option<String> {
    for (_, source) in sources {
        match source {
            Source::Question if from_question(value, question) => return Some("question".into()),
            Source::Result { node, path } => {
                if let Some(at) = from_results(value, node.as_deref(), path.as_deref(), entries) {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn show(value: &Json) -> String {
    let text = value.to_string();
    if text.chars().count() > 80 {
        format!("{}…", text.chars().take(80).collect::<String>())
    } else {
        text
    }
}

/// Check every property of `input` whose schema declares `x-source`. Properties the input
/// leaves out are not checked; whether they may be left out is the schema's business.
pub fn check(
    schema: &Json,
    input: &Json,
    question: Option<&str>,
    entries: &[Json],
) -> Result<Report, String> {
    let mut checked = Vec::new();
    let Some(properties) = schema.get("properties").and_then(Json::as_object) else {
        return Ok(Report { checked });
    };
    for (field, property) in properties {
        let Some(sources) = declared(field, property)? else { continue };
        let Some(value) = input.get(field) else { continue };
        let names: Vec<String> = sources.iter().map(|(raw, _)| raw.clone()).collect();
        let wanted = names.join(" or ");

        let items: Vec<&Json> = match value {
            Json::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        let mut traced_to = Vec::new();
        let mut problem = None;
        for item in items {
            if matches!(item, Json::Object(_) | Json::Array(_) | Json::Null) {
                problem = Some(format!(
                    "{field}: {} cannot be traced; x-source applies to strings, numbers, \
                     booleans and arrays of them",
                    show(item)
                ));
                break;
            }
            match trace(item, &sources, question, entries) {
                Some(at) => traced_to.push(at),
                None => {
                    let wants_question = sources.iter().any(|(_, s)| *s == Source::Question);
                    let hint = if question.is_none() && wants_question {
                        " (no question was given to check against)"
                    } else {
                        ""
                    };
                    problem = Some(format!(
                        "{field}: {} must come from {wanted}, and it does not{hint}",
                        show(item)
                    ));
                    break;
                }
            }
        }
        checked.push(FieldCheck {
            field: field.clone(),
            value: value.clone(),
            sources: names,
            traced_to,
            problem,
        });
    }
    Ok(Report { checked })
}

/// Every `x-source` declaration in a schema that cannot be parsed, so a collection can be
/// checked before any call depends on it.
pub fn declaration_problems(schema: &Json) -> Vec<String> {
    let Some(properties) = schema.get("properties").and_then(Json::as_object) else {
        return Vec::new();
    };
    properties.iter().filter_map(|(field, property)| declared(field, property).err()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(node: &str, result: Json) -> Json {
        json!({ "node": node, "outcome": "ok", "code": 0, "result": result })
    }

    fn schema(field: &str, source: Json) -> Json {
        json!({ "type": "object", "properties": { field: { "type": "string", "x-source": source } } })
    }

    fn problems(schema: &Json, input: Json, question: Option<&str>, entries: &[Json]) -> Vec<String> {
        check(schema, &input, question, entries).unwrap().checked.into_iter().filter_map(|c| c.problem).collect()
    }

    #[test]
    fn parses_every_source_form() {
        assert_eq!(Source::parse("question"), Ok(Source::Question));
        assert_eq!(Source::parse("any"), Ok(Source::Any));
        assert_eq!(Source::parse("result"), Ok(Source::Result { node: None, path: None }));
        assert_eq!(
            Source::parse("result:resolve-period.start"),
            Ok(Source::Result { node: Some("resolve-period".into()), path: Some("start".into()) })
        );
        assert_eq!(
            Source::parse("result:lookup.rows[0].id"),
            Ok(Source::Result { node: Some("lookup".into()), path: Some("rows[0].id".into()) })
        );
        assert!(Source::parse("memory").is_err());
        assert!(Source::parse("result:").is_err());
        assert!(Source::parse("result:node.").is_err());
    }

    #[test]
    fn a_quote_from_the_question_is_found_however_it_is_cased_or_spaced() {
        let s = schema("query", json!("question"));
        let q = Some("What  is the UCHI's weight?");
        assert!(problems(&s, json!({ "query": "uchi" }), q, &[]).is_empty());
        assert!(problems(&s, json!({ "query": "What is the uchi" }), q, &[]).is_empty());
    }

    #[test]
    fn a_tidied_or_partial_word_is_not_a_quote() {
        let s = schema("query", json!("question"));
        let q = Some("How heavy is the uchigatana?");
        assert_eq!(problems(&s, json!({ "query": "uchi" }), q, &[]).len(), 1, "inside a longer word");
        assert_eq!(problems(&s, json!({ "query": "Uchigatana sword" }), q, &[]).len(), 1);
        assert_eq!(problems(&s, json!({ "query": "" }), q, &[]).len(), 1, "an empty quote is not a quote");
    }

    #[test]
    fn a_number_must_be_one_of_the_questions_numerals() {
        let s = json!({ "type": "object", "properties": { "rl": { "type": "integer", "x-source": "question" } } });
        let q = Some("Best stats at RL 150 for a 1,200 budget?");
        assert!(problems(&s, json!({ "rl": 150 }), q, &[]).is_empty());
        assert!(problems(&s, json!({ "rl": 1200 }), q, &[]).is_empty());
        assert_eq!(problems(&s, json!({ "rl": 151 }), q, &[]).len(), 1);
    }

    #[test]
    fn a_result_must_have_been_returned_by_the_named_node_at_the_named_path() {
        let entries = [
            entry("resolve-period", json!({ "start": "2026-08-01", "end": "2026-09-30" })),
            entry("other", json!({ "start": "2026-07-01" })),
        ];
        let s = schema("from", json!("result:resolve-period.start"));
        let r = check(&s, &json!({ "from": "2026-08-01" }), None, &entries).unwrap();
        assert!(r.is_clean());
        assert_eq!(r.checked[0].traced_to, ["0:resolve-period result.start"]);

        assert_eq!(problems(&s, json!({ "from": "2026-07-01" }), None, &entries).len(), 1, "another node's value");
        assert_eq!(problems(&s, json!({ "from": "2026-09-30" }), None, &entries).len(), 1, "the node's value at another path");

        let anywhere_in_node = schema("from", json!("result:resolve-period"));
        assert!(problems(&anywhere_in_node, json!({ "from": "2026-09-30" }), None, &entries).is_empty());
        let any_result = schema("from", json!("result"));
        assert!(problems(&any_result, json!({ "from": "2026-07-01" }), None, &entries).is_empty());
    }

    #[test]
    fn inputs_recorded_in_the_ledger_never_count() {
        let entries = [json!({ "node": "n", "input": { "name": "Acme" }, "outcome": "ok", "code": 0, "result": {} })];
        assert_eq!(problems(&schema("name", json!("result")), json!({ "name": "Acme" }), None, &entries).len(), 1);
    }

    #[test]
    fn any_listed_source_suffices() {
        let entries = [entry("resolve-company", json!({ "id": "C-0042" }))];
        let s = schema("company", json!(["question", "result:resolve-company.id"]));
        let q = Some("Who controls Acme?");
        assert!(problems(&s, json!({ "company": "Acme" }), q, &entries).is_empty());
        assert!(problems(&s, json!({ "company": "C-0042" }), q, &entries).is_empty());
        let p = problems(&s, json!({ "company": "Acme Holding S.p.A." }), q, &entries);
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("question or result:resolve-company.id"), "{}", p[0]);
    }

    #[test]
    fn numbers_compare_by_value_and_arrays_by_element() {
        let entries = [entry("lookup", json!({ "ids": [7, 9], "total": 12.0 }))];
        let s = json!({ "type": "object", "properties": {
            "ids": { "type": "array", "x-source": "result:lookup" },
            "n": { "type": "number", "x-source": "result" }
        } });
        assert!(problems(&s, json!({ "ids": [9, 7], "n": 12 }), None, &entries).is_empty());
        assert_eq!(problems(&s, json!({ "ids": [7, 8] }), None, &entries).len(), 1);
    }

    #[test]
    fn undeclared_absent_and_any_fields_are_not_checked() {
        let s = json!({ "type": "object", "properties": {
            "free": { "type": "string" },
            "loose": { "type": "string", "x-source": ["question", "any"] },
            "strict": { "type": "string", "x-source": "question" }
        } });
        let r = check(&s, &json!({ "free": "x", "loose": "y" }), Some("q"), &[]).unwrap();
        assert!(r.is_clean());
        assert!(r.checked.is_empty());
    }

    #[test]
    fn bad_declarations_are_reported() {
        let s = json!({ "type": "object", "properties": {
            "a": { "type": "string", "x-source": "memory" },
            "b": { "type": "string", "x-source": 3 },
            "c": { "type": "string", "x-source": "question" }
        } });
        let found = declaration_problems(&s);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(check(&s, &json!({}), None, &[]).is_err());
    }
}
