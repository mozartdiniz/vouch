//! `vouch test` — node fixtures (§7.1).
//!
//! The layer that is boolean rather than a rate. Two properties matter more than the report
//! format: a fixture runs the *same* pipeline a real call runs, and a run that checked
//! nothing never reports success.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vouch(collection: &str, args: &[&str]) -> Output {
    let mut full = vec!["-C", collection];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&full)
        .current_dir(repo_root())
        .output()
        .expect("vouch runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// A one-node collection in a temp directory, so a test can hold a *failing* fixture without
/// leaving one committed in the repository.
fn scratch(name: &str, cases: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("vouch-test-cases-{name}"));
    let node = root.join("nodes/double");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&node).expect("can create the collection");

    std::fs::write(
        node.join("node.toml"),
        r#"name = "double"
version = "0.1.0"
purpose = "Doubles n"
run = ["python3", "-c", "import sys,json; d=json.load(sys.stdin); print(json.dumps({'n': d['n']*2}))"]
requires = ["input.n >= 0"]
ensures = ["result.n >= 0"]

[input]
type = "object"
additionalProperties = false
required = ["n"]

[input.properties.n]
type = "integer"

[output]
type = "object"
additionalProperties = false
required = ["n"]

[output.properties.n]
type = "integer"
"#,
    )
    .expect("can write the manifest");

    if !cases.is_empty() {
        std::fs::write(node.join("cases.toml"), cases).expect("can write the cases");
    }
    root
}

fn run_scratch(root: &Path, args: &[&str]) -> Output {
    vouch(root.to_str().expect("a utf-8 path"), args)
}

// ------------------------------------------------- the examples check themselves

/// Every example collection passes its own fixtures. This is what `cases.toml` bought: the
/// example nodes are now covered by `cargo test` without a Rust test per figure.
#[test]
fn every_example_collection_passes_its_own_fixtures() {
    for collection in [
        "examples/hello-world",
        "examples/support-triage",
        "examples/ds3-tools",
    ] {
        let output = vouch(collection, &["test"]);
        assert_eq!(
            code(&output),
            0,
            "{collection} fails its own fixtures:\n{}",
            stderr(&output)
        );
    }
}

// ------------------------------------------------------------------ what it reports

#[test]
fn a_wrong_expected_value_fails_the_case_and_names_both_numbers() {
    let root = scratch(
        "wrong-value",
        r#"[[case]]
name = "doubles"
input = { n = 21 }
expect = { "result.n" = 41 }
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 1, "a failing case exits 1; got {text}");
    assert!(text.contains("FAIL"), "{text}");
    assert!(
        text.contains("result.n: expected 41, got 42"),
        "a failure must name both numbers:\n{text}"
    );
}

#[test]
fn an_unexpected_exit_code_names_the_reason_the_runtime_gave() {
    let root = scratch(
        "wrong-code",
        r#"[[case]]
name = "a negative n"
input = { n = -1 }
expect_code = 0
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 1);
    assert!(text.contains("expected exit 0, got 11 (refusal)"), "{text}");
}

#[test]
fn a_passing_run_exits_zero() {
    let root = scratch(
        "passing",
        r#"[[case]]
name = "doubles"
input = { n = 21 }
expect = { "result.n" = 42 }

[[case]]
name = "refuses a negative"
input = { n = -1 }
expect_code = 11
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 0, "{text}");
    assert!(text.contains("2 cases, 2 passed, 0 failed"), "{text}");
}

/// A fixture is a rehearsal, not a call anyone may quote a number from. If `test` wrote to the
/// ledger, a figure that only ever appeared in a fixture could account for a numeral in a real
/// answer — attestation would be checking against calls that never happened.
#[test]
fn test_writes_nothing_to_the_ledger() {
    let root = scratch(
        "no-ledger",
        r#"[[case]]
name = "doubles"
input = { n = 21 }
expect = { "result.n" = 42 }
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let ledger = root.join(".vouch/ledger");
    let exists = ledger.exists();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 0);
    assert!(!exists, "vouch test must not write a ledger");
}

// ------------------------------------------------- a run that checked nothing fails

/// The same false assurance §3.3 refuses to load a vacuous postcondition over: a checking tool
/// that reports success having checked nothing is worse than one that is absent.
#[test]
fn a_collection_with_no_fixtures_is_an_error_not_a_pass() {
    let root = scratch("no-cases", "");
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("nothing to test"), "{text}");
}

#[test]
fn a_named_node_with_no_fixtures_is_an_error() {
    let root = scratch("no-cases-named", "");
    let output = run_scratch(&root, &["test", "double"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("no cases.toml"), "{text}");
}

/// A case that expects a refusal *and* a value cannot mean anything: a non-zero exit produces
/// no result to read the value from. Caught when the file loads, not as a puzzling failure.
#[test]
fn a_case_expecting_a_refusal_may_not_also_expect_values() {
    let root = scratch(
        "contradictory",
        r#"[[case]]
name = "impossible"
input = { n = -1 }
expect_code = 11
expect = { "result.n" = 42 }
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("produces no result"), "{text}");
}

#[test]
fn an_expected_path_must_be_rooted_at_result() {
    let root = scratch(
        "bad-path",
        r#"[[case]]
name = "unrooted"
input = { n = 21 }
expect = { "n" = 42 }
"#,
    );
    let output = run_scratch(&root, &["test"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("rooted at the returned value"), "{text}");
}

/// A name the user typed that is not in the collection is a typo, not a finding about the
/// collection — so it is an error (2), where a node that *is* there but will not load is a
/// failure (1).
#[test]
fn a_node_name_that_does_not_exist_is_an_error_not_a_failed_node() {
    let root = scratch("typo", "");
    let output = run_scratch(&root, &["test", "doubel"]);
    let text = stderr(&output);
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code(&output), 2, "{text}");
    assert!(text.contains("no node named 'doubel'"), "{text}");
}

#[test]
fn a_named_node_that_will_not_load_is_a_failure() {
    let output = vouch("tests/fixtures", &["test", "no-ensures"]);
    let text = stderr(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(text.contains("will not load"), "{text}");
}

// ---------------------------------------------------------------- broken collections

/// One node that will not load must not hide the results of the rest — nor pass silently.
#[test]
fn an_unloadable_node_fails_the_run_without_aborting_it() {
    let output = vouch("tests/fixtures", &["test"]);
    let text = stderr(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(
        text.contains("will not load"),
        "an unloadable node must be reported:\n{text}"
    );
    assert!(
        text.contains("a postcondition violation is a defect"),
        "the loadable nodes' cases must still run:\n{text}"
    );
}

// -------------------------------------------------------------------------- --json

#[test]
fn the_json_report_carries_every_case() {
    let root = scratch(
        "json",
        r#"[[case]]
name = "doubles"
input = { n = 21 }
expect = { "result.n" = 41 }
"#,
    );
    let output = run_scratch(&root, &["test", "--json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(report["cases"], 1);
    assert_eq!(report["passed"], 0);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["nodes"][0]["node"], "double");
    assert_eq!(report["nodes"][0]["cases"][0]["name"], "doubles");
    assert_eq!(report["nodes"][0]["cases"][0]["passed"], false);
    assert!(
        report["nodes"][0]["cases"][0]["failures"][0]
            .as_str()
            .unwrap()
            .contains("expected 41"),
    );
}
