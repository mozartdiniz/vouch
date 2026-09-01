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
    // The same fact with the sign applied, so escalation-cost's argument is copied from a
    // result rather than computed by whoever is holding it.
    assert_eq!(value["minutes_over"], 250);
}

#[test]
fn triage_reports_a_healthy_ticket() {
    let value = result(&call(TRIAGE, "triage", r#"{"ticket_id":"T-1002"}"#));
    assert_eq!(value["breached"], false);
    assert_eq!(value["minutes_remaining"], 145);
    assert_eq!(value["minutes_over"], 0);
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

// ------------------------------------------------------------------- the ask loop

/// `examples/ask.py` drives the runtime from plain English. The model is the one part that
/// cannot be asserted on, so `VOUCH_LLM` points at a canned stand-in and what gets tested is
/// the control flow around it: catalog loading, decision parsing, the call, the narration,
/// and the exit code each outcome produces.
fn ask(question: &str) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new("python3")
        .arg(root.join("examples/ask.py"))
        .args(["-C", "hello-world", question])
        .current_dir(&root)
        .env("VOUCH_BIN", env!("CARGO_BIN_EXE_vouch"))
        .env(
            "VOUCH_LLM",
            format!(
                "python3 {}",
                root.join("tests/fixtures/fake-llm.py").display()
            ),
        )
        .output()
        .expect("python3 runs ask.py")
}

#[test]
fn the_ask_loop_answers_from_a_verified_result() {
    let output = ask("how many r's are in strawberry?");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trace = String::from_utf8_lossy(&output.stderr);

    assert_eq!(code(&output), 0, "trace was:\n{trace}");
    assert!(stdout.contains("3 times"), "stdout was: {stdout}");
    // The verified value has to reach the trace, since that is what makes the prose
    // checkable rather than merely plausible.
    assert!(trace.contains("\"count\": 3"), "trace was:\n{trace}");
}

#[test]
fn the_ask_loop_declines_an_out_of_scope_question() {
    let output = ask("what is the capital of France?");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(code(&output), 1, "an unanswerable question should exit 1");
    assert!(stdout.contains("I don't know"), "stdout was: {stdout}");
    // Declining must be free: nothing should have been called.
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("→ calling"),
        "no node should run for an out-of-scope question"
    );
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

/// §10 step 4 asks for the routing pack pasted into a `CLAUDE.md`, and `examples/ds3-tools`
/// carries the result so the acceptance run is reproducible. A pasted copy is a copy that
/// drifts, so this asserts it still matches what `describe --all --md` generates today.
#[test]
fn the_committed_claude_md_still_matches_the_generated_pack() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ds3-tools/CLAUDE.md");
    let committed = std::fs::read_to_string(&path).expect("ds3-tools has a CLAUDE.md");

    // Everything after the generated-by comment.
    let body = committed
        .split_once("-->\n")
        .map(|(_, rest)| rest.trim_start())
        .expect("the file leads with the generated-by comment");

    let pack = vouch(DS3, &["describe", "--all", "--md"]);
    assert_eq!(code(&pack), 0, "{}", report(&pack));
    let generated = String::from_utf8_lossy(&pack.stdout);

    assert_eq!(
        body,
        generated.trim_start(),
        "examples/ds3-tools/CLAUDE.md has drifted; regenerate it with \
         `vouch -C examples/ds3-tools describe --all --md`"
    );
}

fn lookup(query: &str) -> Value {
    result(&call(
        DS3,
        "weapon-lookup",
        &format!(r#"{{"query":{}}}"#, Value::String(query.to_string())),
    ))
}

#[test]
fn a_partial_weapon_name_resolves_to_one_exact_name() {
    let value = lookup("uchi");
    assert_eq!(value["resolved"], "Uchigatana");
    assert_eq!(value["match_count"], 1);
    assert_eq!(value["ambiguous"], false);
}

/// The failure this node exists to prevent: two weapons match "lothric sword", and the answer
/// is both of them with nothing resolved, never the first one silently.
#[test]
fn an_ambiguous_name_resolves_to_nothing_and_names_the_candidates() {
    let value = lookup("lothric sword");
    assert_eq!(value["resolved"], "");
    assert_eq!(value["ambiguous"], true);
    assert_eq!(value["match_count"], 2);
    assert_eq!(
        value["candidates"],
        serde_json::json!(["Lothric Knight Sword", "Lothric's Holy Sword"])
    );
}

/// An exact name is unambiguous even though it is also a substring of the ambiguous query
/// above. This is the one clear way the node has to remove ambiguity, so it is worth pinning.
#[test]
fn an_exact_name_resolves_despite_overlapping_candidates() {
    assert_eq!(
        lookup("Lothric Knight Sword")["resolved"],
        "Lothric Knight Sword"
    );
    // Apostrophes and case are normalised away before comparison.
    assert_eq!(
        lookup("lothrics holy sword")["resolved"],
        "Lothric's Holy Sword"
    );
}

/// No match is a real answer about the data, not an error: the weapon is outside the dataset.
#[test]
fn a_weapon_outside_the_dataset_matches_nothing() {
    let value = lookup("moonblade");
    assert_eq!(value["resolved"], "");
    assert_eq!(value["match_count"], 0);
    assert_eq!(value["ambiguous"], false);
    assert_eq!(value["catalog_size"], 10);
}

#[test]
fn a_query_too_short_to_resolve_anything_is_refused() {
    let output = call(DS3, "weapon-lookup", r#"{"query":"ax"}"#);
    assert_refusal(&output, "3 characters");
}

/// The two nodes read the same file, so the collection cannot answer from one copy of the
/// dataset while resolving names against another.
#[test]
fn both_ds3_nodes_read_the_one_dataset() {
    for node in ["stat-optimizer", "weapon-lookup"] {
        let described = vouch(DS3, &["describe", node, "--json"]);
        let value: Value = serde_json::from_slice(&described.stdout).expect("json");
        assert_eq!(value["reads"][0]["path"], "../../data/weapons.csv");
    }
}

/// "Call weapon-lookup first" belongs to no single node's `use_when`, so it lives in the
/// preamble — which only reaches an agent if the file is actually in the repository.
#[test]
fn the_ds3_preamble_routes_ambiguous_names_to_the_lookup() {
    let pack = vouch(DS3, &["describe", "--all", "--md"]);
    assert_eq!(code(&pack), 0, "{}", report(&pack));
    let text = String::from_utf8_lossy(&pack.stdout);
    assert!(text.contains("weapon-lookup first"), "pack was:\n{text}");
    assert!(
        text.contains("not a list to choose from"),
        "the pack must carry the do-not-choose rule:\n{text}"
    );
}
