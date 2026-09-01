//! `cases.toml` — node fixture tests (§7.1).
//!
//! Fixed input, expected exit code, expected values at named paths. No model, no network,
//! nothing to average: a case passes or it does not. This is the layer that catches the
//! scaling math silently breaking, and it must not be confused with `vouch eval`, which has a
//! model in the loop and therefore reports a rate.
//!
//! A case runs the identical pipeline a real call runs (`verify::attempt`) minus the ledger
//! and stdout. Fixtures that took a different path would be testing something nobody ships.

use crate::error::{Result, VouchError};
use crate::ledger;
use crate::manifest::{Node, toml_to_json};
use crate::verify;
use serde::Deserialize;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::Path;

/// Expected values are addressed by the same dotted paths the ledger uses for scalars, and so
/// are rooted at `result`. Insisting on the prefix keeps one path grammar across `cases.toml`,
/// a ledger entry and an attestation report, rather than three that nearly agree.
const ROOT: &str = "result";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFile {
    #[serde(default)]
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    pub input: toml::Value,
    /// The exit code this input should produce. Omitted means 0.
    #[serde(default)]
    pub expect_code: Option<i32>,
    /// Dotted path to expected value, e.g. `"result.stats.dexterity" = 40`.
    #[serde(default)]
    pub expect: BTreeMap<String, toml::Value>,
}

/// What one case did. `failures` empty means it passed.
pub struct Outcome {
    pub name: String,
    pub failures: Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Read `cases.toml` from a node directory. `Ok(None)` when the node has no fixtures — that
/// is a gap, not a breakage, and the caller decides what to make of it.
pub fn load(dir: &Path) -> Result<Option<Vec<Case>>> {
    let path = dir.join("cases.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| VouchError::error(format!("cannot read {}: {e}", path.display())))?;
    let file: CaseFile = toml::from_str(&text)
        .map_err(|e| VouchError::error(format!("cannot parse {}: {e}", path.display())))?;

    for case in &file.case {
        validate(case).map_err(|e| {
            VouchError::error(format!("{}: case '{}': {e}", path.display(), case.name))
        })?;
    }
    Ok(Some(file.case))
}

/// Reject a case that cannot mean what it says, at load time rather than as a mysterious
/// failure later.
fn validate(case: &Case) -> std::result::Result<(), String> {
    if case.expect_code.unwrap_or(0) != 0 && !case.expect.is_empty() {
        return Err(format!(
            "expects exit {} and also expects values; a non-zero exit produces no result to \
             read them from",
            case.expect_code.unwrap_or(0)
        ));
    }
    for path in case.expect.keys() {
        if path != ROOT && !path.starts_with("result.") {
            return Err(format!(
                "expected path '{path}' does not start with 'result.'; paths are rooted at the \
                 returned value, as they are in the ledger"
            ));
        }
    }
    Ok(())
}

/// Run one case against a node.
pub async fn run(node: &Node, case: &Case) -> Outcome {
    let mut failures = Vec::new();
    let input = toml_to_json(&case.input);
    let expected_code = case.expect_code.unwrap_or(0);

    let attempt = verify::attempt(node, &input).await;
    let actual = attempt.code();

    if actual != expected_code {
        failures.push(match &attempt.verdict {
            Ok(()) => format!("expected exit {expected_code}, got 0 and a value"),
            Err(e) => format!(
                "expected exit {expected_code}, got {actual} ({}): {}",
                e.outcome.as_str(),
                e.reason
            ),
        });
    }

    // Values are only checked when the call actually produced one that the runtime accepted.
    // Reading them off a rejected result would report a fixture as passing against a value
    // the runtime refused to hand a caller.
    if let Some(value) = attempt.value() {
        let root = serde_json::json!({ ROOT: value });
        for (path, expected) in &case.expect {
            let expected = toml_to_json(expected);
            match ledger::value_at(&root, path) {
                None => failures.push(format!("{path}: nothing at that path")),
                Some(found) if !equal(found, &expected) => {
                    failures.push(format!("{path}: expected {expected}, got {found}"));
                }
                Some(_) => {}
            }
        }
    } else if !case.expect.is_empty() && actual == expected_code {
        // Only reachable for `expect_code = 0` cases, which `validate` is the other half of.
        failures.push("no result was produced, so no expected value could be checked".into());
    }

    Outcome {
        name: case.name.clone(),
        failures,
    }
}

/// JSON equality, except that numbers compare numerically: TOML's `40` and a node's `40.0`
/// are the same number, and a fixture should not fail on the spelling.
///
/// A float expectation is checked **to the precision it was written to**, the same rule
/// §6.2 gives `attest` for prose. `543.1948141` in a fixture asserts nine significant digits
/// and passes against `543.1948140689826`; write more digits to demand more.
///
/// The alternative — exact float equality — sounds stricter and is mostly a way to fail. A
/// node is asked to emit full precision (§8.3), while expectations are copied from wherever
/// the ground truth lives: a spreadsheet cell, a screenshot, another implementation's
/// printout. Demanding that such a figure reproduce IEEE noise tests the transcription, not
/// the node. An integer expectation still compares exactly, because it has no decimals to
/// round to.
fn equal(found: &Json, expected: &Json) -> bool {
    match (found.as_f64(), expected.as_f64()) {
        (Some(found), Some(expected)) => rounds_to(found, expected),
        _ => found == expected,
    }
}

/// Whether `found`, rounded to as many decimal places as `expected` was written to, is
/// `expected`.
fn rounds_to(found: f64, expected: f64) -> bool {
    if found == expected {
        return true;
    }
    let factor = 10f64.powi(decimals(expected) as i32);
    let rounded = (found * factor).round() / factor;
    (rounded - expected).abs() <= 1e-9 * expected.abs().max(1.0)
}

/// How many decimal places a number was written to. `40` is 0, `0.495` is 3.
///
/// Read off the shortest representation that round-trips, which is what `{}` prints, so the
/// answer is the author's spelling rather than the binary expansion of it.
fn decimals(value: f64) -> usize {
    let text = format!("{value}");
    match text.split_once('.') {
        Some((_, fraction)) => fraction.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn case(toml_src: &str) -> Case {
        toml::from_str(toml_src).expect("a case parses")
    }

    #[test]
    fn an_integer_matches_the_same_number_written_as_a_float() {
        assert!(equal(&json!(40.0), &json!(40)));
        assert!(equal(&json!(40), &json!(40.0)));
        assert!(!equal(&json!(40), &json!(41)));
        // Types other than numbers still compare exactly.
        assert!(!equal(&json!("40"), &json!(40)));
        assert!(equal(&json!(true), &json!(true)));
    }

    /// An expectation copied from a spreadsheet cell is written to the precision that cell
    /// showed. Demanding it also reproduce the node's IEEE noise tests the transcription.
    #[test]
    fn a_float_expectation_is_checked_to_the_precision_it_was_written_to() {
        assert!(equal(&json!(543.1948140689826), &json!(543.1948141)));
        assert!(equal(&json!(0.6300000000000001), &json!(0.63)));
        assert!(equal(&json!(0.8999999999999999), &json!(0.9)));

        // Writing more digits demands more of the node: this value agrees to five places and
        // not to seven, so the shorter expectation passes and the longer one does not.
        assert!(equal(&json!(543.1948149), &json!(543.19481)));
        assert!(!equal(&json!(543.1948149), &json!(543.1948141)));
        // And a genuinely different number is still a failure at any precision.
        assert!(!equal(&json!(543.2), &json!(543.1948141)));
        assert!(!equal(&json!(0.64), &json!(0.63)));
        // An integer expectation has no decimals to round to, so it stays exact.
        assert!(!equal(&json!(347.7), &json!(347)));
    }

    #[test]
    fn decimals_reads_the_authors_spelling() {
        assert_eq!(decimals(40.0), 0);
        assert_eq!(decimals(0.495), 3);
        assert_eq!(decimals(543.1948141), 7);
    }

    #[test]
    fn a_case_expecting_a_refusal_may_not_also_expect_values() {
        let bad = case(
            r#"name = "x"
               input = {}
               expect_code = 11
               expect = { "result.n" = 1 }"#,
        );
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn expected_paths_must_be_rooted_at_result() {
        let bad = case(
            r#"name = "x"
               input = {}
               expect = { "stats.dexterity" = 40 }"#,
        );
        assert!(validate(&bad).unwrap_err().contains("rooted"));

        let good = case(
            r#"name = "x"
               input = {}
               expect = { "result.stats.dexterity" = 40 }"#,
        );
        assert!(validate(&good).is_ok());
    }
}
