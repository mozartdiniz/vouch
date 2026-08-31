//! The registry preamble and the markdown routing pack (§5).
//!
//! This is what a collection says about itself. §5.3 calls it the entire integration story —
//! a user pastes it into a CLAUDE.md, or an agent runs the command at session start — so the
//! test that matters most is what it *omits*: contracts are enforcing rather than advisory,
//! and publishing them would bloat every turn's context to no purpose.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn vouch(collection: &str, args: &[&str]) -> Output {
    let mut full = vec!["-C", collection];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&full)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("vouch runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

const TRIAGE: &str = "examples/support-triage";
const FIXTURES: &str = "tests/fixtures";

// -------------------------------------------------------------- the markdown pack

#[test]
fn the_pack_leads_with_the_collections_own_context() {
    let pack = stdout(&vouch(TRIAGE, &["describe", "--all", "--md"]));

    assert!(pack.starts_with("# support-triage\n"), "{pack}");
    assert!(pack.contains("SLA status, breach credits and queue waits"));
    // The rule that belongs to no single node, and so exists only here.
    assert!(pack.contains("Call triage first"), "{pack}");
}

#[test]
fn the_pack_carries_every_nodes_routing_context() {
    let pack = stdout(&vouch(TRIAGE, &["describe", "--all", "--md"]));

    for node in ["## triage", "## escalation-cost", "## wait-estimate"] {
        assert!(pack.contains(node), "missing {node}");
    }
    assert!(pack.contains("**Use when**"));
    assert!(pack.contains("**Not for**"));
    // Negative examples do disproportionate work against misrouting (§5.1).
    assert!(pack.contains("call wait-estimate instead"));
    // Parameters carry their type, obligation, and the author's guidance.
    assert!(pack.contains("`ticket_id` (string, required)"));
    assert!(pack.contains("prefix it with 'T-'"));
    // A worked example a caller can copy.
    assert!(
        pack.contains(r#"vouch call triage --input '{"ticket_id":"T-1001"}'"#),
        "{pack}"
    );
}

/// Contracts are enforcing, not advisory. A caller does not need to read a precondition to
/// make a good call — if they get it wrong the refusal says so, in words they can act on.
#[test]
fn the_pack_omits_the_contracts() {
    let pack = stdout(&vouch(TRIAGE, &["describe", "--all", "--md"]));

    assert!(
        !pack.contains("startsWith"),
        "no CEL expressions belong in the pack"
    );
    assert!(
        !pack.contains("result.minutes_remaining =="),
        "no postconditions either"
    );
    assert!(!pack.contains("ensures"));
}

/// The pack has to say how to invoke a node and what the outcomes mean, or it is a catalogue
/// rather than an integration story.
#[test]
fn the_pack_explains_how_to_call_and_how_to_read_the_result() {
    let pack = stdout(&vouch(TRIAGE, &["describe", "--all", "--md"]));

    assert!(pack.contains("vouch call <node> --input"));
    assert!(pack.contains("JSON object on stdout"));
    assert!(pack.contains("exit 0"));
    assert!(pack.contains("A refusal is a good outcome when it is the true one."));
}

/// A collection without a preamble is still perfectly usable.
#[test]
fn a_collection_without_a_preamble_still_produces_a_pack() {
    let output = vouch(FIXTURES, &["describe", "--all", "--md"]);
    assert_eq!(code(&output), 0);

    let pack = stdout(&output);
    assert!(
        pack.starts_with("# fixtures\n"),
        "falls back to the directory name: {pack}"
    );
    assert!(pack.contains("## ok"));
}

/// A node that fails the contract-strength gate cannot be called, so advertising it would
/// only invite a failure. It is skipped, and named on stderr.
#[test]
fn unloadable_nodes_are_left_out_of_the_pack_and_reported() {
    let output = vouch(FIXTURES, &["describe", "--all", "--md"]);
    let pack = stdout(&output);
    let warnings = String::from_utf8_lossy(&output.stderr);

    assert!(
        !pack.contains("## vacuous"),
        "an uncallable node must not be advertised"
    );
    assert!(
        warnings.contains("vacuous"),
        "but it must be reported: {warnings}"
    );
}

#[test]
fn all_defaults_to_markdown() {
    let with_flag = stdout(&vouch(TRIAGE, &["describe", "--all", "--md"]));
    let without = stdout(&vouch(TRIAGE, &["describe", "--all"]));
    assert_eq!(with_flag, without);
}

// -------------------------------------------------------------------- the JSON form

/// `--all --json` is what an agent loop consumes: one call instead of one per node.
#[test]
fn all_json_carries_the_preamble_and_every_node() {
    let output = vouch(TRIAGE, &["describe", "--all", "--json"]);
    let described: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");

    assert_eq!(described["collection"], "support-triage");
    assert!(
        described["description"]
            .as_str()
            .unwrap()
            .contains("SLA status")
    );
    assert_eq!(described["nodes"].as_array().unwrap().len(), 3);

    let notes = described["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap().contains("Call triage first"))
    );
}

// ------------------------------------------------------------------- argument shape

#[test]
fn describe_needs_either_a_node_or_all() {
    let output = vouch(TRIAGE, &["describe"]);
    assert_eq!(code(&output), 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("--all"));
}

#[test]
fn describe_rejects_a_node_name_together_with_all() {
    let output = vouch(TRIAGE, &["describe", "triage", "--all"]);
    assert_eq!(code(&output), 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("not both"));
}

#[test]
fn a_single_node_can_be_rendered_as_markdown_too() {
    let pack = stdout(&vouch(TRIAGE, &["describe", "triage", "--md"]));
    assert!(pack.contains("## triage"));
    assert!(!pack.contains("## wait-estimate"));
}

/// Built in a temp directory rather than in `tests/fixtures`, because the other tests read
/// that collection concurrently and a preamble appearing under them would race.
#[test]
fn a_malformed_preamble_is_an_error_rather_than_silently_dropped() {
    let collection = std::env::temp_dir().join("vouch-test-bad-preamble");
    std::fs::create_dir_all(collection.join("nodes")).expect("can create the collection");
    std::fs::create_dir_all(collection.join(".vouch")).expect("can create .vouch");
    std::fs::write(
        collection.join(".vouch/registry.toml"),
        "name = [this is not toml\n",
    )
    .expect("can write the preamble");

    let output = vouch(collection.to_str().unwrap(), &["describe", "--all", "--md"]);
    let _ = std::fs::remove_dir_all(&collection);

    assert_eq!(
        code(&output),
        1,
        "context the author meant to publish must not be dropped"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("registry.toml"));
}
