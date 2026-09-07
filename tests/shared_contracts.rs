//! Contracts the collection makes, not one node (§3.4).
//!
//! A contract written in one node is a contract about that node. What a collection wants to
//! say is "a stat is 1 to 99, wherever a stat appears", and the only way to say it was to
//! write it into every manifest and remember to write it into the next one. That is a habit,
//! not a guard, and it is how the same defect turns up in a node written months after it was
//! fixed somewhere else.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures-shared")
}

fn call(node: &str, input: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(["call", node, "--input", input])
        .current_dir(fixtures())
        .output()
        .expect("vouch runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited normally")
}

fn report(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().last().unwrap_or_default();
    serde_json::from_str(line).unwrap_or_else(|e| panic!("stderr is not a report ({e}): {text}"))
}

fn value(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is a JSON object")
}

#[test]
fn a_collection_rule_refuses_a_call_no_node_would_have() {
    let out = call("takes-n", r#"{"n": 500}"#);
    assert_eq!(code(&out), 11);
    let report = report(&out);
    assert_eq!(report["outcome"], "refusal");
    // The collection's own words, not the expression: it is the broader statement and the one
    // a reader can act on.
    assert_eq!(report["reason"], "n runs to 100 everywhere in this collection");
    assert!(out.stdout.is_empty(), "a refusal writes no value");
}

#[test]
fn a_call_the_rule_permits_is_untouched() {
    let out = call("takes-n", r#"{"n": 5}"#);
    assert_eq!(code(&out), 0);
    assert_eq!(value(&out)["n"], 5);
}

/// The trap this design exists to avoid. An unevaluable contract fails closed (§3.2), which is
/// right for a rule an author wrote about their own node and wrong for one written about the
/// whole collection: an optional parameter left out would refuse every call that omitted it.
#[test]
fn an_optional_parameter_left_out_does_not_fire_the_rule() {
    let out = call("takes-n", "{}");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(value(&out)["n"], 1);
}

/// A postcondition is about what comes back, so it is scoped to the result and not to the
/// input. Scoping it to the input would skip exactly the calls that omitted the parameter,
/// and the node would return the value the collection said it never returns with nothing to
/// catch it.
#[test]
fn a_shared_postcondition_checks_the_result_whatever_the_input_was() {
    for input in ["{}", r#"{"n": 1}"#] {
        let out = call("negative", input);
        assert_eq!(code(&out), 13, "for input {input}");
        let report = report(&out);
        assert_eq!(report["outcome"], "defect");
        assert_eq!(report["reason"], "no node here returns a negative n");
        assert!(out.stdout.is_empty(), "a defect leaks no value");
    }
}

/// A collection rule is about the nodes it names a parameter of, and silent everywhere else.
#[test]
fn a_node_that_does_not_take_the_parameter_is_untouched() {
    let out = call("no-n", r#"{"word": "hi"}"#);
    assert_eq!(code(&out), 0);
    assert_eq!(value(&out)["n"], 7);
}
