//! `vouch provenance` (§6.3): an agent's input traced to the question and the ledger, with
//! the same exit-code convention as `attest` — 0 traced, 1 a finding, 2 the check could not
//! run.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn vouch(args: &[&str]) -> Output {
    let mut full = vec!["-C", "tests/fixtures", "provenance"];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&full)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("vouch runs")
}

fn ledger(name: &str, lines: &[&str]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("vouch-provenance-{name}-{}.jsonl", std::process::id()));
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

const SCHEMA: &str = r#"{"type":"object","properties":{
  "company":{"type":"string","x-source":["question","result:resolve-company.id"]},
  "from":{"type":"string","x-source":"result:resolve-period.start"}}}"#;

#[test]
fn a_traced_input_exits_0_and_says_where_each_value_came_from() {
    let file = ledger("traced", &[r#"{"node":"resolve-period","outcome":"ok","code":0,"result":{"start":"2026-08-01"}}"#]);
    let out = vouch(&[
        "--schema", SCHEMA,
        "--input", r#"{"company":"Acme","from":"2026-08-01"}"#,
        "--ledger", file.to_str().unwrap(),
        "--question", "Report for Acme over the last two months",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["clean"], true);
    let traced: Vec<&str> = report["checked"].as_array().unwrap().iter().map(|c| c["traced_to"][0].as_str().unwrap()).collect();
    assert_eq!(traced, ["question", "0:resolve-period result.start"]);
}

#[test]
fn a_value_worked_out_by_the_agent_exits_1() {
    let file = ledger("untraced", &[r#"{"node":"resolve-period","outcome":"ok","code":0,"result":{"start":"2026-08-01"}}"#]);
    let out = vouch(&[
        "--schema", SCHEMA,
        "--input", r#"{"company":"Acme Holding S.p.A.","from":"2026-07-01"}"#,
        "--ledger", file.to_str().unwrap(),
        "--question", "Report for Acme over the last two months",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("UNTRACED company"), "{text}");
    assert!(text.contains("UNTRACED from"), "{text}");
}

#[test]
fn a_bad_declaration_exits_2() {
    let out = vouch(&["--schema", r#"{"type":"object","properties":{"a":{"x-source":"memory"}}}"#, "--input", r#"{"a":"x"}"#]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown source"));
}
