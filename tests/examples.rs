//! The example collections behave the way their READMEs say they do.
//!
//! These test documentation, not the runtime — but the READMEs quote exit codes, refusal
//! messages, and specific numbers, and that is precisely the kind of prose that drifts out of
//! step with the code it describes. Every figure asserted here appears verbatim in a README.

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

fn call(collection: &str, node: &str, input: &str) -> Output {
    vouch(collection, &["call", node, "--input", input])
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited normally")
}

fn result(output: &Output) -> Value {
    assert_eq!(code(output), 0, "expected success, got {}", report(output));
    serde_json::from_slice(&output.stdout).expect("stdout is a JSON object")
}

fn report(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text.lines().last().unwrap_or_default();
    serde_json::from_str(line).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// Assert a refusal, and that the reason still says what the README claims it says.
fn assert_refusal(output: &Output, contains: &str) {
    let report = report(output);
    assert_eq!(code(output), 11, "expected a refusal; report was {report}");
    assert_eq!(report["outcome"], "refusal");
    assert!(
        output.stdout.is_empty(),
        "a refusal must not write a value to stdout"
    );
    assert!(
        report["reason"]
            .as_str()
            .unwrap_or_default()
            .contains(contains),
        "reason should mention {contains:?}; report was {report}"
    );
}

/// The JavaScript nodes need a `node` binary. Rather than fail on a machine without one,
/// say so and move on — the Python paths still cover the runtime behaviour.
fn node_missing() -> bool {
    let available = Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("skipping: `node` is not installed");
    }
    !available
}

// ------------------------------------------------------------- every example loads

/// Nothing under examples/ may fall foul of the contract-strength gate, a schema that will
/// not compile, or a contract that will not parse. A broken example is a broken tutorial.
#[test]
fn every_example_node_loads() {
    for collection in [
        "examples/hello-world",
        "examples/support-triage",
        "examples/ds3-tools",
    ] {
        let listing = vouch(collection, &["list"]);
        assert_eq!(code(&listing), 0, "{collection} should list");
        let text = String::from_utf8_lossy(&listing.stdout);
        assert!(
            !text.contains("unloadable"),
            "{collection} has an unloadable node:\n{text}"
        );

        for line in text.lines() {
            let node = line.split_whitespace().next().expect("a node name");
            let described = vouch(collection, &["describe", node, "--json"]);
            assert_eq!(
                code(&described),
                0,
                "{collection}/{node} should describe; {}",
                report(&described)
            );
        }
    }
}

// ------------------------------------------------------------------- hello-world

const HELLO: &str = "examples/hello-world";

#[test]
fn strawberry_has_three_rs() {
    let value = result(&call(
        HELLO,
        "count-letters",
        r#"{"word":"strawberry","letter":"r"}"#,
    ));
    assert_eq!(value["count"], 3);
    assert_eq!(value["word_length"], 10);
}

#[test]
fn a_multi_character_letter_is_refused() {
    let output = call(
        HELLO,
        "count-letters",
        r#"{"word":"strawberry","letter":"rr"}"#,
    );
    assert_refusal(&output, "exactly one character");
}

// ---------------------------------------------------------------- support-triage

const TRIAGE: &str = "examples/support-triage";

#[test]
fn triage_reports_a_breached_enterprise_ticket() {
    let value = result(&call(TRIAGE, "triage", r#"{"ticket_id":"T-1001"}"#));
    assert_eq!(value["tier"], "enterprise");
    assert_eq!(value["breached"], true);
    assert_eq!(value["minutes_remaining"], -250);
}

#[test]
fn triage_reports_a_healthy_ticket() {
    let value = result(&call(TRIAGE, "triage", r#"{"ticket_id":"T-1002"}"#));
    assert_eq!(value["breached"], false);
    assert_eq!(value["minutes_remaining"], 145);
}

#[test]
fn a_bare_ticket_number_is_refused_with_the_right_shape() {
    assert_refusal(&call(TRIAGE, "triage", r#"{"ticket_id":"1001"}"#), "T-1001");
}

#[test]
fn escalation_cost_credits_a_breached_enterprise_ticket() {
    if node_missing() {
        return;
    }
    let value = result(&call(
        TRIAGE,
        "escalation-cost",
        r#"{"tier":"enterprise","minutes_over":250}"#,
    ));
    assert_eq!(value["credit_percent"], 25);
    assert_eq!(value["credit_usd"], 500.0);
}

/// The branch that ends in no answer at all: the ticket really has breached, and there
/// really is no credit to compute.
#[test]
fn the_free_tier_branch_ends_in_no_answer() {
    if node_missing() {
        return;
    }
    let output = call(
        TRIAGE,
        "escalation-cost",
        r#"{"tier":"free","minutes_over":1440}"#,
    );
    assert_refusal(&output, "no SLA commitment");
}

/// A wrong turn is corrected rather than merely rejected.
#[test]
fn routing_to_the_wrong_node_names_the_right_one() {
    if node_missing() {
        return;
    }
    let output = call(
        TRIAGE,
        "escalation-cost",
        r#"{"tier":"pro","minutes_over":-145}"#,
    );
    assert_refusal(&output, "call wait-estimate instead");
}

#[test]
fn wait_estimate_computes_a_queue_wait() {
    if node_missing() {
        return;
    }
    let value = result(&call(
        TRIAGE,
        "wait-estimate",
        r#"{"queue_depth":12,"agents_available":4,"avg_handle_minutes":20}"#,
    ));
    assert_eq!(value["rounds"], 3);
    assert_eq!(value["estimated_wait_minutes"], 60);
}

/// The precondition that stops a division by zero from becoming a defect.
#[test]
fn no_agents_on_shift_is_a_refusal_not_a_crash() {
    if node_missing() {
        return;
    }
    let output = call(
        TRIAGE,
        "wait-estimate",
        r#"{"queue_depth":12,"agents_available":0,"avg_handle_minutes":20}"#,
    );
    assert_refusal(&output, "no agents are on shift");
}

/// The rough edge the collection README documents: a well-formed but absent ticket id is
/// reported as a defect, because in M1 only the runtime can refuse and it sees only the
/// input. If this ever becomes a refusal, that README paragraph needs deleting.
#[test]
fn an_absent_ticket_is_still_reported_as_a_defect() {
    let output = call(TRIAGE, "triage", r#"{"ticket_id":"T-9999"}"#);
    let report = report(&output);
    assert_eq!(code(&output), 20, "report was {report}");
    assert_eq!(report["outcome"], "defect");
}

// --------------------------------------------------------------------- ds3-tools

const DS3: &str = "examples/ds3-tools";

#[test]
fn the_documented_lothric_build_still_computes() {
    let value = result(&call(
        DS3,
        "stat-optimizer",
        r#"{"weapon":"Lothric Knight Sword","soul_level":120}"#,
    ));
    assert_eq!(value["attack_rating"], 190.9);
    assert_eq!(value["total_points_spent"], 119);
    assert_eq!(value["stats"]["dexterity"], 60);
}

#[test]
fn a_misspelled_weapon_is_refused_with_the_valid_names() {
    let output = call(
        DS3,
        "stat-optimizer",
        r#"{"weapon":"lothric sword","soul_level":120}"#,
    );
    assert_refusal(&output, "Lothric Knight Sword");
}

#[test]
fn an_impossible_soul_level_is_refused() {
    let output = call(
        DS3,
        "stat-optimizer",
        r#"{"weapon":"Uchigatana","soul_level":9999}"#,
    );
    assert_refusal(&output, "802");
}
