//! The ledger and `vouch attest` (§6).
//!
//! §1.2 states the guarantee in two halves — the result passed its contracts, *and* its
//! origin is recorded. M1 covered the first half. These tests cover the second, and the
//! reconciliation that makes a written answer checkable rather than merely plausible.
//!
//! Each test uses its own `VOUCH_SESSION` so they can run in parallel without sharing a
//! ledger, and clears it first so a rerun starts clean.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ledger_path(collection: &str, session: &str) -> PathBuf {
    root()
        .join(collection)
        .join(".vouch/ledger")
        .join(format!("session-{session}.jsonl"))
}

fn vouch(collection: &str, session: &str, args: &[&str]) -> Output {
    let mut full = vec!["-C", collection];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&full)
        .current_dir(root())
        .env("VOUCH_SESSION", session)
        .output()
        .expect("vouch runs")
}

/// Start from an empty ledger so a rerun is deterministic.
fn fresh(collection: &str, session: &str) {
    let _ = std::fs::remove_file(ledger_path(collection, session));
}

fn entries(collection: &str, session: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(ledger_path(collection, session))
        .unwrap_or_else(|e| panic!("no ledger for session {session}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is a JSON object"))
        .collect()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

const FIXTURES: &str = "tests/fixtures";
const TRIAGE: &str = "examples/support-triage";

// ------------------------------------------------------------------- the ledger

#[test]
fn a_successful_call_is_recorded_with_its_scalars() {
    let session = "test-success";
    fresh(FIXTURES, session);

    let output = vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );
    assert_eq!(code(&output), 0);

    let entries = entries(FIXTURES, session);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];

    assert_eq!(entry["node"], "ok");
    assert_eq!(entry["outcome"], "ok");
    assert_eq!(entry["code"], 0);
    assert_eq!(entry["input"]["n"], 21);
    assert_eq!(entry["result"]["n"], 42);
    // The flattened numeric leaves are what attestation checks against (§6.1).
    assert_eq!(entry["scalars"]["result.n"], 42);
    assert!(
        entry["ts"].as_str().unwrap().ends_with('Z'),
        "an RFC3339 UTC timestamp"
    );
}

/// The ledger is an account of the session, not a highlight reel. A refusal is exactly the
/// kind of thing an audit wants to see.
#[test]
fn refusals_and_defects_are_recorded_too() {
    let session = "test-outcomes";
    fresh(FIXTURES, session);

    vouch(
        FIXTURES,
        session,
        &["call", "liar", "--input", r#"{"n": 1}"#],
    );
    vouch(
        FIXTURES,
        session,
        &["call", "crasher", "--input", r#"{"n": 1}"#],
    );

    let entries = entries(FIXTURES, session);
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0]["outcome"], "defect");
    assert_eq!(entries[0]["code"], 13);
    // A postcondition failure still has a result worth recording — it is the evidence.
    assert_eq!(entries[0]["result"]["n"], -5);

    assert_eq!(entries[1]["outcome"], "defect");
    assert_eq!(entries[1]["code"], 20);
    assert!(
        entries[1].get("result").is_none(),
        "a crash produced no value"
    );
}

/// A refusal never runs the node, so nothing is recorded for it — the precondition is
/// checked before execution and the call never reaches the ledger.
#[test]
fn a_precondition_refusal_records_nothing() {
    let session = "test-precondition";
    fresh(FIXTURES, session);

    let output = vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": -1}"#],
    );
    assert_eq!(code(&output), 11);
    assert!(
        !ledger_path(FIXTURES, session).exists(),
        "nothing ran, so nothing is recorded"
    );
}

/// §6.1's audit record: which bytes produced this number.
#[test]
fn declared_reads_are_hashed() {
    let session = "test-reads";
    fresh(TRIAGE, session);

    vouch(
        TRIAGE,
        session,
        &["call", "triage", "--input", r#"{"ticket_id": "T-1001"}"#],
    );

    let entries = entries(TRIAGE, session);
    let read = &entries[0]["reads"][0];
    assert_eq!(read["path"], "data/tickets.csv");
    assert_eq!(
        read["sha256"].as_str().unwrap().len(),
        64,
        "a sha256 is 64 hex characters"
    );
    assert!(read["bytes"].as_u64().unwrap() > 0);
}

#[test]
fn entries_accumulate_across_calls() {
    let session = "test-append";
    fresh(FIXTURES, session);

    for n in ["1", "2", "3"] {
        vouch(
            FIXTURES,
            session,
            &["call", "ok", "--input", &format!(r#"{{"n": {n}}}"#)],
        );
    }
    assert_eq!(
        entries(FIXTURES, session).len(),
        3,
        "the ledger is append-only"
    );
}

// ------------------------------------------------------------------ vouch attest

fn attest(collection: &str, session: &str, extra: &[&str]) -> Output {
    let ledger = ledger_path(collection, session);
    let mut args = vec!["attest", "--ledger", ledger.to_str().unwrap()];
    args.extend_from_slice(extra);
    vouch(collection, session, &args)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn attest_accepts_an_answer_built_from_verified_values() {
    let session = "test-attest-clean";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    let output = attest(
        FIXTURES,
        session,
        &["--text", "The doubled value is 42 units."],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stderr(&output).contains("clean"));
}

/// The whole point of §6.2: a hand-edited digit in otherwise correct prose.
#[test]
fn attest_catches_a_changed_digit() {
    let session = "test-attest-dirty";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    let output = attest(
        FIXTURES,
        session,
        &["--text", "The doubled value is 43 units."],
    );
    assert_eq!(code(&output), 1, "an unmatched numeral exits 1");
    let report = stderr(&output);
    assert!(report.contains("UNATTESTED"), "{report}");
    assert!(report.contains("43"), "{report}");
}

#[test]
fn attest_does_not_treat_the_users_own_numbers_as_fabrication() {
    let session = "test-attest-question";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    // 21 is the input, not a result, so it is not in `scalars` — but the user said it.
    let dirty = attest(FIXTURES, session, &["--text", "Doubling 21 gives 42."]);
    assert_eq!(
        code(&dirty),
        1,
        "without the question, 21 is unaccounted for"
    );

    let clean = attest(
        FIXTURES,
        session,
        &[
            "--text",
            "Doubling 21 gives 42.",
            "--question",
            "what is 21 doubled?",
        ],
    );
    assert_eq!(code(&clean), 0, "{}", stderr(&clean));
}

/// Inputs are recorded but not provenanced — an agent chose them. Admitting them is opt-in,
/// because doing it by default would launder a fabricated argument into an attested figure.
#[test]
fn input_values_count_only_when_asked_for() {
    let session = "test-attest-inputs";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    assert_eq!(
        code(&attest(FIXTURES, session, &["--text", "the input was 21"])),
        1
    );
    assert_eq!(
        code(&attest(
            FIXTURES,
            session,
            &["--text", "the input was 21", "--include-inputs"]
        )),
        0
    );
}

#[test]
fn attest_reads_text_from_stdin_by_default() {
    let session = "test-attest-stdin";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    let ledger = ledger_path(FIXTURES, session);
    let mut child = Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args([
            "-C",
            FIXTURES,
            "attest",
            "--ledger",
            ledger.to_str().unwrap(),
        ])
        .current_dir(root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("vouch runs");

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"the answer is 42")
        .unwrap();
    let output = child.wait_with_output().expect("vouch finishes");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn attest_emits_a_machine_readable_report() {
    let session = "test-attest-json";
    fresh(FIXTURES, session);
    vouch(
        FIXTURES,
        session,
        &["call", "ok", "--input", r#"{"n": 21}"#],
    );

    let output = attest(FIXTURES, session, &["--text", "42 and 99", "--json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");

    assert_eq!(report["clean"], false);
    assert_eq!(report["matched"], 1);
    assert_eq!(report["unmatched"][0]["numeral"], "99");
    assert!(report["unmatched"][0]["line"].is_number());
}

/// Unmatched numerals are a finding (exit 1). A check that could not run at all is a
/// different thing and must not be mistaken for a clean bill of health.
#[test]
fn attest_separates_its_own_failures_from_its_findings() {
    let output = vouch(
        FIXTURES,
        "test-attest-missing",
        &[
            "attest",
            "--ledger",
            "/nonexistent/ledger.jsonl",
            "--text",
            "the answer is 42",
        ],
    );
    assert_eq!(
        code(&output),
        2,
        "a ledger that cannot be read exits 2, not 1"
    );
}
