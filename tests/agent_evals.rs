//! `vouch eval` — agent routing evals (§7.2).
//!
//! Driven by the fake agents in `tests/agents/`, so the machinery is covered without a model:
//! the loop, the correction path, the in-memory ledger, and the attestation of the final
//! prose. What is *not* covered here is routing accuracy, which needs a real model and is a
//! rate rather than a pass — that is the point of the command, and it costs tokens, so it
//! cannot live in `cargo test`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `-C` is a real change of working directory, so an agent command has to be absolute.
fn agent(name: &str) -> String {
    format!(
        "sh {} {{prompt}}",
        repo_root().join("tests/agents").join(name).display()
    )
}

/// Written per-test rather than committed, so the repository never carries an eval suite that
/// is meant to fail.
fn suite(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("vouch-evals-{name}.toml"));
    std::fs::write(&path, body).expect("can write the suite");
    path
}

fn eval(suite: &Path, agent: &str, extra: &[&str]) -> Output {
    let mut args = vec![
        "-C".to_string(),
        "tests/fixtures".to_string(),
        "eval".to_string(),
        "--file".to_string(),
        suite.display().to_string(),
        "--agent".to_string(),
        agent.to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));

    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&args)
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

const DOUBLING: &str = r#"[[eval]]
ask = "what is 21 doubled?"
expect_node = "ok"
expect_params = { n = 21 }
attest = true
"#;

// ------------------------------------------------------------------ the happy path

#[test]
fn a_run_that_routes_and_attests_passes() {
    let path = suite("routes", DOUBLING);
    let output = eval(&path, &agent("routes.sh"), &["-n", "3"]);
    let text = stderr(&output);

    assert_eq!(code(&output), 0, "{text}");
    assert!(text.contains("3/3 runs passed (100%)"), "{text}");
    // The route is reported, not just the rate: a rate that drops is unactionable without it.
    assert!(text.contains("[answered: ok]"), "{text}");
}

/// A refusal is a routing correction (§4.2). An agent that reads one and fixes its argument
/// has routed correctly, and the eval must say so rather than penalising the first attempt.
#[test]
fn a_run_that_recovers_from_a_refusal_still_passes() {
    let path = suite("corrects", DOUBLING);
    let output = eval(&path, &agent("corrects.sh"), &[]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
}

// ---------------------------------------------------------- what an eval must catch

/// Every call succeeded and every contract held; the model then wrote a number no node
/// produced. This is the failure the whole project exists for, so it is the one an eval must
/// not miss.
#[test]
fn a_fabricated_figure_fails_the_run_even_though_every_call_succeeded() {
    let path = suite("fabricates", DOUBLING);
    let output = eval(&path, &agent("fabricates.sh"), &[]);
    let text = stderr(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(
        text.contains("did not come from a node: 43"),
        "the unattested numeral must be named:\n{text}"
    );
    // It routed correctly. Only the prose was wrong, and the report has to separate the two.
    assert!(text.contains("[answered: ok]"), "{text}");
}

#[test]
fn a_reply_the_loop_cannot_act_on_is_a_failed_run_not_a_failed_command() {
    let path = suite("confused", DOUBLING);
    let output = eval(&path, &agent("confused.sh"), &[]);
    let text = stderr(&output);

    assert_eq!(
        code(&output),
        1,
        "a model that will not answer is a rate, not an error:\n{text}"
    );
    assert!(text.contains("could not be acted on"), "{text}");
}

#[test]
fn routing_to_the_wrong_node_names_what_was_called_instead() {
    let path = suite("stops", DOUBLING);
    let output = eval(&path, &agent("stops.sh"), &[]);
    let text = stderr(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(text.contains("expected a call to ok"), "{text}");
}

/// A node the agent invented does not exist, so nothing runs and the refusal is the loop's
/// own — it must reach the agent as a correction rather than crashing the run.
#[test]
fn a_call_to_a_node_that_does_not_exist_is_corrected() {
    let path = suite("wrong-node", DOUBLING);
    let output = eval(&path, &agent("wrong-node.sh"), &[]);
    let text = stderr(&output);

    assert_eq!(code(&output), 1, "{text}");
    assert!(text.contains("stopped"), "{text}");
}

// ------------------------------------------------------------------- stopping cases

/// The case this project cares most about: the honest "there is no answer". §7.2 has no field
/// for it, so `expect_stop` is an addition — without it the collection's most important
/// behaviour is the one thing an eval cannot assert.
#[test]
fn a_case_can_require_the_agent_to_decline() {
    let path = suite(
        "expect-stop",
        r#"[[eval]]
ask = "what is the airspeed velocity of an unladen swallow?"
expect_stop = true
"#,
    );
    assert_eq!(code(&eval(&path, &agent("stops.sh"), &[])), 0);

    // And an agent that answers it anyway fails the case.
    let output = eval(&path, &agent("routes.sh"), &[]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("expected the agent to decline"),
        "{}",
        stderr(&output)
    );
}

// -------------------------------------------------------------------- rates, not passes

/// Because there is a model in the loop, a suite is graded on a rate. Without a floor below
/// 1.0, any flake fails the run and nobody can use this in CI.
#[test]
fn a_pass_rate_below_the_floor_fails_and_above_it_passes() {
    let path = suite("rate", DOUBLING);
    let strict = eval(&path, &agent("fabricates.sh"), &["--min-rate", "0.5"]);
    assert_eq!(code(&strict), 1, "0/1 is below a floor of 0.5");

    let lenient = eval(&path, &agent("fabricates.sh"), &["--min-rate", "0.0"]);
    assert_eq!(
        code(&lenient),
        0,
        "a floor of 0 accepts any rate:\n{}",
        stderr(&lenient)
    );
}

// ------------------------------------------------------- errors, distinct from findings

/// A harness that dies partway through has still done real work, and the cases that finished
/// are worth more than the tidiness of discarding them. The run still fails — a partial rate
/// is not a rate — but it says how far it got, and it says *why* even when the agent reports
/// its trouble on stdout with nothing on stderr, which is what `claude -p` does when it hits a
/// usage limit.
#[test]
fn an_agent_that_quits_partway_keeps_what_finished_and_says_why() {
    let path = suite(
        "quits",
        &format!(
            "{DOUBLING}\n[[eval]]\nask = \"what is the airspeed velocity of an unladen \
             swallow?\"\nexpect_stop = true\n"
        ),
    );
    let output = eval(&path, &agent("quits.sh"), &[]);
    let text = stderr(&output);

    assert_eq!(
        code(&output),
        2,
        "a broken harness is an error, not a rate:\n{text}"
    );
    // The case that completed is reported rather than thrown away.
    assert!(text.contains("1/1 runs passed"), "{text}");
    assert!(text.contains("run cut short in case 2 of 2"), "{text}");
    assert!(
        text.contains("after 1 of 2 complete case"),
        "it must say how far it got:\n{text}"
    );
    // And the reason reaches the user even though the agent put it on stdout.
    assert!(
        text.contains("session limit"),
        "the agent's own reason must be surfaced:\n{text}"
    );
    assert!(text.contains("exit 1"), "{text}");
}

#[test]
fn a_missing_suite_is_an_error_rather_than_an_empty_pass() {
    let output = eval(
        &std::env::temp_dir().join("vouch-evals-not-here.toml"),
        &agent("routes.sh"),
        &[],
    );
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("no eval suite at"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_suite_with_no_cases_is_an_error() {
    let path = suite("empty", "");
    let output = eval(&path, &agent("routes.sh"), &[]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("nothing to run"),
        "{}",
        stderr(&output)
    );
}

/// An agent command that will not run is a broken harness, not a failed case: there is no
/// rate to report, so it must not be reported as 0/1.
#[test]
fn an_agent_command_that_cannot_run_is_an_error_not_a_zero_rate() {
    let path = suite("no-agent", DOUBLING);
    let output = eval(&path, "definitely-not-a-real-command-9f2a", &[]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot run the agent command"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn expecting_params_without_a_node_is_rejected_when_the_suite_loads() {
    let path = suite(
        "params-no-node",
        r#"[[eval]]
ask = "what is 21 doubled?"
expect_params = { n = 21 }
"#,
    );
    let output = eval(&path, &agent("routes.sh"), &[]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("nothing to check them against"),
        "{}",
        stderr(&output)
    );
}

/// A run that stops writes no answer, so there is nothing for attestation to check. Saying so
/// when the suite loads beats reporting a puzzling 0/1 later.
#[test]
fn expecting_a_stop_and_an_attestation_is_rejected() {
    let path = suite(
        "stop-and-attest",
        r#"[[eval]]
ask = "what is 21 doubled?"
expect_stop = true
attest = true
"#,
    );
    let output = eval(&path, &agent("stops.sh"), &[]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("writes no answer to check"),
        "{}",
        stderr(&output)
    );
}

// --------------------------------------------------------------------------- --json

#[test]
fn the_json_report_carries_the_rate_and_the_route() {
    let path = suite("json", DOUBLING);
    let output = eval(&path, &agent("fabricates.sh"), &["--json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");

    assert_eq!(report["total"], 1);
    assert_eq!(report["passed"], 0);
    assert_eq!(report["rate"], 0.0);
    assert_eq!(report["cases"][0]["ask"], "what is 21 doubled?");
    assert_eq!(report["cases"][0]["runs_detail"][0]["ending"], "answered");
    assert_eq!(
        report["cases"][0]["runs_detail"][0]["calls"][0]["node"],
        "ok"
    );
    assert_eq!(
        report["cases"][0]["runs_detail"][0]["calls"][0]["input"]["n"],
        21
    );
}

/// An eval is a rehearsal. Writing its calls to the real ledger would let a figure that only
/// ever appeared in an eval account for a numeral in a real answer later.
#[test]
fn eval_writes_nothing_to_the_ledger() {
    let ledger = repo_root().join("tests/fixtures/.vouch/ledger");
    let before = std::fs::read_dir(&ledger)
        .map(|d| d.count())
        .unwrap_or_default();

    let path = suite("no-ledger", DOUBLING);
    assert_eq!(code(&eval(&path, &agent("routes.sh"), &[])), 0);

    let after = std::fs::read_dir(&ledger)
        .map(|d| d.count())
        .unwrap_or_default();
    assert_eq!(before, after, "vouch eval must not write a ledger");
}

// ------------------------------------------------------- the committed example suites

/// Every example collection's `.vouch/evals.toml` parses and grades. A real run needs a model
/// and costs tokens, so what is checked here is the part that can rot silently: a typo in a
/// committed suite, or a case naming a node that no longer exists.
#[test]
fn every_example_eval_suite_loads_and_grades() {
    for collection in [
        "examples/hello-world",
        "examples/support-triage",
        "examples/ds3-tools",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vouch"))
            .args([
                "-C",
                collection,
                "eval",
                "--agent",
                &agent("stops.sh"),
                "--min-rate",
                "0",
            ])
            .current_dir(repo_root())
            .output()
            .expect("vouch runs");

        assert_eq!(
            code(&output),
            0,
            "{collection}'s eval suite does not load:\n{}",
            stderr(&output)
        );
        // A suite that names a node the collection does not have would report this instead of
        // an honest routing failure.
        assert!(
            !stderr(&output).contains("no node called"),
            "{collection} names a node that is not in the collection:\n{}",
            stderr(&output)
        );
    }
}
