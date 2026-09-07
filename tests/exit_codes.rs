//! The exit-code taxonomy (§4.1).
//!
//! The distinction between a refusal and a defect is the product, so every code gets a test.
//! Two properties are asserted throughout and matter more than the codes themselves:
//!
//! - a non-zero exit **never** writes a value to stdout, and
//! - every non-zero exit emits one structured JSON object on stderr.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn vouch(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(args)
        .current_dir(fixtures())
        .output()
        .expect("vouch runs")
}

fn call(node: &str, input: &str) -> Output {
    vouch(&["call", node, "--input", input])
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// The single JSON object every non-zero exit puts on stderr.
fn report(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().last().unwrap_or_default();
    serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stderr is not a JSON report ({e}): {text}"))
}

/// Assert the full shape of a failed call: code, outcome, silent stdout, structured stderr.
fn assert_failure(output: &Output, expected_code: i32, expected_outcome: &str) -> Value {
    let report = report(output);
    assert_eq!(
        code(output),
        expected_code,
        "exit code; report was {report}"
    );
    assert_eq!(
        report["outcome"], expected_outcome,
        "outcome; report was {report}"
    );
    assert_eq!(report["code"], expected_code, "code in report");
    assert!(
        stdout(output).trim().is_empty(),
        "a failed call must not write a value to stdout, got: {}",
        stdout(output)
    );
    report
}

// ---------------------------------------------------------------- success

#[test]
fn success_returns_the_value_on_stdout() {
    let output = call("ok", r#"{"n": 21}"#);
    assert_eq!(code(&output), 0);
    let value: Value = serde_json::from_str(&stdout(&output)).expect("stdout is a JSON object");
    assert_eq!(value["n"], 42);
}

// ------------------------------------------------------------ caller error

#[test]
fn input_violating_its_schema_is_a_caller_error() {
    let report = assert_failure(&call("ok", r#"{"n": "twelve"}"#), 10, "caller_error");
    assert!(report["reason"].as_str().unwrap().contains("input schema"));
}

#[test]
fn unknown_input_field_is_a_caller_error() {
    assert_failure(
        &call("ok", r#"{"n": 1, "extra": true}"#),
        10,
        "caller_error",
    );
}

#[test]
fn malformed_json_input_is_a_caller_error() {
    assert_failure(&call("ok", "{not json"), 10, "caller_error");
}

#[test]
fn non_object_input_is_a_caller_error() {
    assert_failure(&call("ok", "[1, 2, 3]"), 10, "caller_error");
}

// ---------------------------------------------------------------- refusals

#[test]
fn failed_precondition_refuses() {
    let report = assert_failure(&call("ok", r#"{"n": -1}"#), 11, "refusal");
    assert_eq!(report["details"]["expression"], "input.n >= 0");
}

#[test]
fn timeout_refuses() {
    let report = assert_failure(&call("slow", r#"{"n": 1}"#), 14, "refusal");
    assert!(report["reason"].as_str().unwrap().contains("timeout"));
}

/// Fail closed (§3.2). A postcondition that cannot be evaluated must refuse, never pass.
/// If this test ever goes green on exit 0, the entire guarantee is gone.
#[test]
fn unevaluable_contract_refuses_rather_than_passing() {
    let report = assert_failure(&call("unevaluable", r#"{"n": 1}"#), 15, "refusal");
    assert_eq!(report["details"]["expression"], "result.missing > 0");
}

// ----------------------------------------------------------------- defects

#[test]
fn output_violating_its_schema_is_a_defect() {
    assert_failure(&call("wrong-type", r#"{"n": 1}"#), 12, "defect");
}

/// NaN and Infinity are not JSON and must never reach a contract, where every comparison
/// against them would silently be false (§8).
///
/// They are caught at the JSON boundary rather than the schema boundary: `NaN`, `Infinity`,
/// and an overflowing literal like `1e400` are all rejected by the parser, so the outcome is
/// a protocol violation. Either way it is a defect and no value is returned.
#[test]
fn non_finite_output_is_a_defect() {
    let report = assert_failure(&call("infinite", "{}"), 21, "defect");
    assert!(
        report["reason"].as_str().unwrap().contains("out of range"),
        "{report}"
    );
}

#[test]
fn failed_postcondition_is_a_defect_and_leaks_no_value() {
    let report = assert_failure(&call("liar", r#"{"n": 1}"#), 13, "defect");
    assert_eq!(report["details"]["expression"], "result.n >= 0");
    // The rejected value is available for debugging on stderr, but never on stdout.
    assert_eq!(report["details"]["rejected_result"]["n"], -5);
}

#[test]
fn node_crash_is_a_defect() {
    let report = assert_failure(&call("crasher", r#"{"n": 1}"#), 20, "defect");
    assert!(
        report["details"]["stderr"]
            .as_str()
            .unwrap()
            .contains("boom")
    );
}

/// The distinction this whole file is about, at the one boundary that could not express it.
///
/// A node that exits 3 has understood the question and is saying the answer does not exist.
/// That is a refusal — try something else — and not a defect, which tells the caller the node
/// is broken and to stop trusting it. Before this, both came out as 20, so an author with an
/// ordinary "not in my table" to report had to choose between libelling their own node and
/// inventing a success-shaped way to say nothing.
#[test]
fn a_node_can_refuse_and_it_is_not_a_defect() {
    let report = assert_failure(&call("refuser", r#"{"n": 1}"#), 16, "refusal");
    // The node's own words, not a paraphrase: §4.2 asks for reasons a reader can act on, and
    // the node is the only thing that knows why.
    assert_eq!(report["reason"], "no entry for 7 in my table; try another");
    assert_eq!(report["node"], "refuser");
}

/// Refusing without a reason is allowed — it is still not a defect — but it is a dead end for
/// whoever has to act on it, so the runtime names it rather than reporting an empty string.
#[test]
fn a_refusal_with_no_reason_says_so() {
    let report = assert_failure(&call("silent-refuser", r#"{"n": 1}"#), 16, "refusal");
    let reason = report["reason"].as_str().unwrap();
    assert!(reason.contains("gave no reason"), "{reason}");
    assert!(reason.contains("silent-refuser"), "{reason}");
}

/// The direction this must never fail in. An uncaught exception is exit 1 in every language a
/// node is likely to be written in; reading that as a considered refusal would launder a
/// crash into an answer.
#[test]
fn an_ordinary_crash_is_still_a_defect() {
    assert_failure(&call("crasher", r#"{"n": 1}"#), 20, "defect");
}

/// stdout is the payload channel (§8.2). A stray log line is the mistake every contributor
/// makes on day one, so it has to produce a message that names the cause.
#[test]
fn log_line_on_stdout_is_a_protocol_violation() {
    let report = assert_failure(&call("chatty", r#"{"n": 1}"#), 21, "defect");
    assert!(
        report["details"]["hint"]
            .as_str()
            .unwrap()
            .contains("stderr")
    );
}

#[test]
fn non_object_stdout_is_a_protocol_violation() {
    let report = assert_failure(&call("array-out", r#"{"n": 1}"#), 21, "defect");
    assert!(
        report["reason"]
            .as_str()
            .unwrap()
            .contains("expected a JSON object")
    );
}

// ------------------------------------------------- contract strength (§3.3)

#[test]
fn node_with_vacuous_postcondition_will_not_load() {
    let report = assert_failure(&call("vacuous", r#"{"n": 1}"#), 1, "error");
    assert!(
        report["reason"]
            .as_str()
            .unwrap()
            .contains("references `result`")
    );
}

#[test]
fn node_with_no_postconditions_will_not_load() {
    let report = assert_failure(&call("no-ensures", r#"{"n": 1}"#), 1, "error");
    assert!(report["reason"].as_str().unwrap().contains("no `ensures`"));
}

#[test]
fn describing_an_unknown_node_lists_the_known_ones() {
    let report = assert_failure(&vouch(&["describe", "nonexistent"]), 1, "error");
    assert!(report["reason"].as_str().unwrap().contains("known nodes"));
}

// --------------------------------------------------------------- discovery

#[test]
fn list_reports_every_loadable_node_and_does_not_hide_broken_ones() {
    let output = vouch(&["list"]);
    assert_eq!(code(&output), 0);
    let text = stdout(&output);
    assert!(text.contains("ok"), "{text}");
    // A node that fails the strength gate is still listed, marked unloadable, so a broken
    // node cannot make the rest of the collection disappear.
    assert!(text.contains("vacuous"), "{text}");
    assert!(text.contains("unloadable"), "{text}");
}
