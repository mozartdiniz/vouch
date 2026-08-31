//! The `-C` / `--directory` flag.
//!
//! `-C` matches `git -C`: it changes the working directory before anything else happens, so
//! collection discovery *and* relative paths on the command line both resolve from there.
//! That second half is the part worth pinning — it is the difference between "start here"
//! and "look for a collection here", and only the first is what the flag claims to do.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

/// The repository root — deliberately *not* a collection, so every test here proves the flag
/// did the work rather than the working directory happening to be right.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vouch(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("vouch runs")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn report(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().last().unwrap_or_default();
    serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stderr is not a JSON report ({e}): {text}"))
}

#[test]
fn without_the_flag_the_repository_root_is_not_a_collection() {
    // The premise every other test here rests on.
    let output = vouch(&["list"]);
    assert_eq!(code(&output), 1);
    assert_eq!(report(&output)["outcome"], "error");
}

#[test]
fn finds_a_collection_from_outside_it() {
    let output = vouch(&["-C", "tests/fixtures", "list"]);
    assert_eq!(code(&output), 0, "{}", report(&output));
    assert!(stdout(&output).contains("ok"));
}

#[test]
fn works_after_the_subcommand_too() {
    let output = vouch(&["list", "-C", "tests/fixtures"]);
    assert_eq!(code(&output), 0, "{}", report(&output));
    assert!(stdout(&output).contains("ok"));
}

#[test]
fn long_form_is_equivalent() {
    let output = vouch(&["--directory", "tests/fixtures", "list"]);
    assert_eq!(code(&output), 0, "{}", report(&output));
}

/// Discovery still walks upward from wherever `-C` lands, so pointing at a node directory
/// finds the collection containing it.
fn node_dir_case(dir: &str) {
    let output = vouch(&["-C", dir, "call", "ok", "--input", r#"{"n": 21}"#]);
    assert_eq!(code(&output), 0, "{}", report(&output));
    let value: Value = serde_json::from_str(&stdout(&output)).expect("a JSON object");
    assert_eq!(value["n"], 42);
}

#[test]
fn discovery_still_walks_upward_from_the_target() {
    node_dir_case("tests/fixtures/nodes");
    node_dir_case("tests/fixtures/nodes/ok");
}

/// The chdir commitment: a relative `--input @file` resolves against the `-C` directory, not
/// against the shell's working directory. `doubling-input.json` exists only inside the
/// fixtures collection, so this passes only under "start here" semantics.
#[test]
fn relative_input_paths_resolve_against_the_target_directory() {
    assert!(
        !repo_root().join("doubling-input.json").exists(),
        "the fixture must not also exist at the repo root, or this test proves nothing"
    );

    let output = vouch(&[
        "-C",
        "tests/fixtures",
        "call",
        "ok",
        "--input",
        "@doubling-input.json",
    ]);
    assert_eq!(code(&output), 0, "{}", report(&output));
    let value: Value = serde_json::from_str(&stdout(&output)).expect("a JSON object");
    assert_eq!(value["n"], 42);
}

#[test]
fn a_missing_directory_is_reported_as_such() {
    let output = vouch(&["-C", "no/such/place", "list"]);
    assert_eq!(code(&output), 1);
    let report = report(&output);
    assert_eq!(report["outcome"], "error");
    assert!(
        report["reason"].as_str().unwrap().contains("cannot enter"),
        "{report}"
    );
}

/// A real directory that is not a collection gets the discovery error, not the entry error.
/// The two failures are different problems and must not be conflated.
#[test]
fn a_directory_without_a_collection_reports_the_discovery_failure() {
    let output = vouch(&["-C", "src", "list"]);
    assert_eq!(code(&output), 1);
    let report = report(&output);
    assert!(
        report["reason"]
            .as_str()
            .unwrap()
            .contains("no vouch project found"),
        "{report}"
    );
}
