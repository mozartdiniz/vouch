//! `vouch eval` — agent routing evals (§7.2).
//!
//! A natural-language question goes in; the assertions are that the right node was called
//! with the right parameters, and that the prose written from the results attests clean.
//!
//! There is a model in the loop, so **this is a rate, not a pass**. A case is run `n` times
//! and reported as `9/10`. Do not conflate it with `vouch test`, which is boolean: one
//! catches the arithmetic breaking, this catches a reworded `use_when` quietly dropping
//! routing accuracy from 95% to 60%, which is otherwise invisible until someone complains.
//!
//! The loop here is `examples/ask.py` in Rust, and deliberately so: plan a call, run it, feed
//! refusals back as corrections, then narrate and attest. The agent is any command that takes
//! a prompt and prints a reply, so the runtime never depends on a particular harness.

use crate::attest;
use crate::error::{Result, VouchError};
use crate::ledger;
use crate::manifest::Node;
use crate::markdown;
use crate::registry::Registry;
use crate::verify;
use serde::Deserialize;
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

/// How many decisions the agent may make before a run is abandoned. Enough for a couple of
/// chained calls plus a correction, low enough that a confused model cannot spin.
const MAX_DECISIONS: usize = 6;

/// The rules the agent is held to. Deliberately the same shape `ask.py` uses, so that the
/// loop being measured is the loop the examples demonstrate.
const PLANNING_RULES: &str = "\
You are driving a set of verified functions, called nodes, to answer a question. You are NOT
answering it yourself.

Reply with ONLY a JSON object, in one of three shapes:

  {\"call\": {\"node\": \"<name>\", \"input\": {...}}}
      Run a node. `input` must satisfy that node's parameters exactly. Build the arguments
      from the question and from the verified results of earlier calls — never from your own
      guesses. If you need a figure you do not have, call the node that produces it first.

  {\"done\": true}
      The verified results so far are enough to answer the question.

  {\"stop\": \"<one sentence>\"}
      No node can answer this, or a node has told you the answer does not exist. Say what you
      cannot answer and why.

Stopping is a good answer when it is the true one. Never pick a node that is merely close,
and never fill a parameter with a number you invented — a wrong answer is worse than none.
";

// ------------------------------------------------------------------------ the suite

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    #[serde(default)]
    eval: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// The question, in plain English, exactly as a user would put it.
    pub ask: String,
    /// A node the run must call.
    #[serde(default)]
    pub expect_node: Option<String>,
    /// Parameters that call must carry. A subset: a case pins what matters and leaves the
    /// rest free, so an added optional parameter does not break every eval.
    #[serde(default)]
    pub expect_params: Option<toml::Value>,
    /// The final prose must reconcile against the run's own ledger.
    #[serde(default)]
    pub attest: bool,
    /// The run must end in the agent declining. Not in §7.2, but the case that matters most
    /// to this project — the question whose honest answer is "there isn't one" — is otherwise
    /// unwritable.
    #[serde(default)]
    pub expect_stop: bool,
}

pub fn load(path: &Path) -> Result<Vec<Case>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| VouchError::error(format!("cannot read {}: {e}", path.display())))?;
    let suite: Suite = toml::from_str(&text)
        .map_err(|e| VouchError::error(format!("cannot parse {}: {e}", path.display())))?;

    for case in &suite.eval {
        if case.expect_params.is_some() && case.expect_node.is_none() {
            return Err(VouchError::error(format!(
                "{}: '{}' expects params without expecting a node; there is nothing to check \
                 them against",
                path.display(),
                case.ask
            )));
        }
        if case.expect_stop && case.attest {
            return Err(VouchError::error(format!(
                "{}: '{}' expects the agent to stop and also to attest; a run that stops \
                 writes no answer to check",
                path.display(),
                case.ask
            )));
        }
    }
    Ok(suite.eval)
}

/// Where a collection keeps its evals. Collection-level, like the registry preamble, because
/// routing is a property of the set rather than of any one node.
pub fn default_path(registry: &Registry) -> PathBuf {
    registry.root.join(".vouch").join("evals.toml")
}

// ------------------------------------------------------------------------ the agent

/// A command that takes a prompt and prints a reply — `claude -p {prompt}`, or anything else.
///
/// `{prompt}` is substituted where it appears; without it the prompt is appended as the last
/// argument. Staying agent-agnostic is the point: the runtime shells out to whatever harness
/// the user already has.
pub struct Agent {
    argv: Vec<String>,
}

impl Agent {
    pub fn new(command: &str) -> Result<Agent> {
        let argv = split(command);
        if argv.is_empty() {
            return Err(VouchError::error("--agent is empty; nothing to run"));
        }
        Ok(Agent { argv })
    }

    async fn ask(&self, prompt: &str) -> Result<String> {
        let substituted = self.argv.iter().any(|a| a.contains("{prompt}"));
        let mut args: Vec<String> = self
            .argv
            .iter()
            .map(|a| a.replace("{prompt}", prompt))
            .collect();
        if !substituted {
            args.push(prompt.to_string());
        }

        let output = tokio::process::Command::new(&args[0])
            .args(&args[1..])
            // The prompt goes in as an argument, so the agent has no stdin to read. Leaving
            // it inherited makes a harness that also accepts piped input wait for one:
            // `claude -p` blocks three seconds per call before warning and carrying on, which
            // across a suite is minutes of nothing.
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| {
                VouchError::error(format!(
                    "cannot run the agent command `{}`: {e}",
                    self.argv[0]
                ))
            })?;

        if !output.status.success() {
            // Say what happened even when the agent is quiet about it. A harness that reports
            // its own trouble on *stdout* — `claude -p` prints "You've hit your session limit"
            // there — otherwise produces "the agent command failed: " and nothing else, which
            // is the least useful sentence a checking tool can end on.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let said = match stderr.trim() {
                "" => stdout.trim(),
                text => text,
            };
            let status = match output.status.code() {
                Some(code) => format!("exit {code}"),
                None => "killed by signal".to_string(),
            };
            return Err(VouchError::error(match said {
                "" => format!("the agent command failed ({status}) and said nothing"),
                said => format!("the agent command failed ({status}): {said}"),
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Split a command line on whitespace, respecting single and double quotes. Enough for an
/// `--agent` string and one fewer dependency than the general case would cost.
fn split(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;

    for c in command.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// Models wrap JSON in a fence or a sentence often enough that insisting on clean output
/// would measure formatting rather than routing. Take the outermost object.
fn parse_reply(text: &str) -> Result<Json> {
    let body = match (text.find("```"), text.rfind("```")) {
        (Some(open), Some(close)) if close > open => {
            let inner = &text[open + 3..close];
            inner.strip_prefix("json").unwrap_or(inner)
        }
        _ => text,
    };
    let (start, end) = (body.find('{'), body.rfind('}'));
    match (start, end) {
        (Some(start), Some(end)) if end > start => serde_json::from_str(&body[start..=end])
            .map_err(|e| VouchError::error(format!("the agent's reply is not valid JSON: {e}"))),
        _ => Err(VouchError::error(format!(
            "no JSON object in the agent's reply: {}",
            text.chars().take(200).collect::<String>()
        ))),
    }
}

// -------------------------------------------------------------------------- one run

/// How one run of one case ended.
#[derive(Debug, PartialEq)]
pub enum Ending {
    /// The agent narrated an answer from verified results.
    Answered,
    /// The agent declined — the honest out when no answer exists.
    Stopped,
    /// A node reported itself broken, so the agent stopped rather than working around it.
    Broken,
    /// The agent used its decisions without reaching an answer.
    Exhausted,
    /// The agent did not say anything the loop could act on.
    Confused(String),
}

impl Ending {
    /// A short name for a report line. `Confused` carries its reason, which belongs in the
    /// failure list rather than in the one-line route summary.
    pub fn label(&self) -> &'static str {
        match self {
            Ending::Answered => "answered",
            Ending::Stopped => "stopped",
            Ending::Broken => "broken node",
            Ending::Exhausted => "out of decisions",
            Ending::Confused(_) => "unusable reply",
        }
    }
}

pub struct Run {
    pub ending: Ending,
    /// Every call the runtime accepted, as (node, input).
    pub calls: Vec<(String, Json)>,
    /// The prose the agent wrote, when it wrote any.
    pub answer: Option<String>,
    /// Checks that did not hold, in the order a reader wants them.
    pub failures: Vec<String>,
}

impl Run {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Run one case once.
///
/// The ledger stays in memory. An eval is a rehearsal: writing these calls to
/// `.vouch/ledger/` would let a figure that only ever appeared in an eval account for a
/// numeral in a real answer later, which is exactly the loosening `VOUCH_SESSION` exists to
/// prevent.
pub async fn run_once(nodes: &[Node], context: &str, agent: &Agent, case: &Case) -> Result<Run> {
    let mut entries: Vec<Json> = Vec::new();
    let mut calls: Vec<(String, Json)> = Vec::new();
    let mut steps: Vec<String> = Vec::new();
    let mut correction: Option<String> = None;
    let mut ending = Ending::Exhausted;
    let mut answer = None;

    for _ in 0..MAX_DECISIONS {
        let prompt = plan_prompt(context, &case.ask, &steps, correction.as_deref());
        correction = None;

        let decision = match parse_reply(&agent.ask(&prompt).await?) {
            Ok(decision) => decision,
            // A reply the loop cannot act on is a failure of the run, not of the command:
            // it is precisely the kind of thing an eval exists to count.
            Err(e) => {
                ending = Ending::Confused(e.reason);
                break;
            }
        };

        if let Some(reason) = decision.get("stop").and_then(Json::as_str) {
            ending = Ending::Stopped;
            answer = Some(reason.to_string());
            break;
        }

        if decision.get("done").and_then(Json::as_bool) == Some(true) {
            if steps.is_empty() {
                ending = Ending::Stopped;
                break;
            }
            let prose = agent.ask(&narrate_prompt(&case.ask, &steps)).await?;
            answer = Some(prose);
            ending = Ending::Answered;
            break;
        }

        let Some(call) = decision.get("call") else {
            ending = Ending::Confused(format!("reply had no call, done or stop: {decision}"));
            break;
        };
        let Some(name) = call.get("node").and_then(Json::as_str) else {
            ending = Ending::Confused(format!("call named no node: {call}"));
            break;
        };
        let input = call.get("input").cloned().unwrap_or_else(|| json!({}));

        let Some(node) = nodes.iter().find(|n| n.manifest.name == name) else {
            correction = Some(format!(
                "there is no node called '{name}' in this collection; the nodes are: {}",
                nodes
                    .iter()
                    .map(|n| n.manifest.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        };

        let attempt = verify::attempt(node, &input).await;
        if attempt.executed {
            entries.push(ledger::entry(
                node,
                &input,
                attempt.outcome(),
                attempt.code(),
                attempt.produced.as_ref(),
            ));
        }

        match attempt.verdict {
            Ok(()) => {
                let result = attempt.produced.expect("a verdict that held has a value");
                steps.push(format!(
                    "{name}({input})\n  → {result}",
                    input = compact(&input),
                    result = compact(&result)
                ));
                calls.push((name.to_string(), input));
            }
            // A defect means the node is broken. Retrying is pointless and answering anyway
            // would be dishonest, so the run stops here.
            Err(e) if e.outcome == crate::error::Outcome::Defect => {
                ending = Ending::Broken;
                answer = Some(e.reason);
                break;
            }
            // A refusal, or an input the schema rejected, is a correction. Hand it back.
            Err(e) => correction = Some(e.reason),
        }
    }

    let failures = judge(case, &ending, &calls, answer.as_deref(), &entries);
    Ok(Run {
        ending,
        calls,
        answer,
        failures,
    })
}

/// What the case asserted, checked against what the run did.
fn judge(
    case: &Case,
    ending: &Ending,
    calls: &[(String, Json)],
    answer: Option<&str>,
    entries: &[Json],
) -> Vec<String> {
    let mut failures = Vec::new();

    if let Ending::Confused(why) = ending {
        failures.push(format!("the agent's reply could not be acted on: {why}"));
    }
    if matches!(ending, Ending::Exhausted) {
        failures.push(format!("no answer after {MAX_DECISIONS} decisions"));
    }
    if matches!(ending, Ending::Broken) {
        failures.push("a node reported itself broken".to_string());
    }

    let matched = case.expect_node.as_ref().map(|expected| {
        calls
            .iter()
            .filter(|(node, _)| node == expected)
            .collect::<Vec<_>>()
    });

    if let (Some(expected), Some(matched)) = (&case.expect_node, &matched)
        && matched.is_empty()
    {
        failures.push(match calls.is_empty() {
            true => format!("expected a call to {expected}; no node was called"),
            false => format!(
                "expected a call to {expected}; called {}",
                calls
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    // Parameters are checked as a subset, and against *any* matching call: a run that reaches
    // the right call after a correction has routed correctly, which is what is being measured.
    if let (Some(expected), Some(matched)) = (&case.expect_params, &matched) {
        let expected = crate::manifest::toml_to_json(expected);
        if !matched.is_empty() && !matched.iter().any(|(_, input)| subset(&expected, input)) {
            failures.push(format!(
                "expected params {} in a call to {}; got {}",
                compact(&expected),
                case.expect_node.as_deref().unwrap_or("?"),
                matched
                    .iter()
                    .map(|(_, i)| compact(i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if case.expect_stop && !matches!(ending, Ending::Stopped) {
        failures.push(format!(
            "expected the agent to decline; it {}",
            match ending {
                Ending::Answered => "answered",
                Ending::Broken => "hit a broken node",
                Ending::Exhausted => "ran out of decisions",
                Ending::Confused(_) => "produced an unusable reply",
                Ending::Stopped => unreachable!(),
            }
        ));
    }

    if case.attest {
        match (ending, answer) {
            (Ending::Answered, Some(prose)) => {
                let scalars: Vec<f64> = attest::ledger_scalars(entries, false)
                    .values()
                    .copied()
                    .collect();
                let report = attest::attest(prose, &scalars, Some(&case.ask));
                if !report.is_clean() {
                    failures.push(format!(
                        "{} of {} numerals did not come from a node: {}",
                        report.unmatched.len(),
                        report.checked,
                        report
                            .unmatched
                            .iter()
                            .map(|u| u.numeral.raw.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            _ => failures.push("no answer was written, so nothing could be attested".to_string()),
        }
    }

    failures
}

/// Every key in `expected` present in `actual` with an equal value. Numbers compare
/// numerically, so a soul level of `120` matches whether TOML or the agent spelled it as an
/// integer.
fn subset(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::Object(want), Json::Object(got)) => want
            .iter()
            .all(|(k, v)| got.get(k).is_some_and(|found| subset(v, found))),
        _ => match (expected.as_f64(), actual.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => expected == actual,
        },
    }
}

fn compact(value: &Json) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

// ------------------------------------------------------------------------- prompts

/// The context an agent is given is the collection's own routing pack — the same markdown a
/// user pastes into a CLAUDE.md. Building a second, eval-only description would measure a
/// context nobody ships.
pub fn context(registry: &Registry, nodes: &[Node]) -> Result<String> {
    let preamble = registry.preamble()?;
    let name = registry
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "collection".to_string());
    Ok(markdown::pack(&name, preamble.as_ref(), nodes))
}

fn plan_prompt(context: &str, ask: &str, steps: &[String], correction: Option<&str>) -> String {
    let mut prompt = format!(
        "{PLANNING_RULES}\nThe collection you are working with:\n\n{context}\n\nThe user asked: {ask}\n"
    );

    if !steps.is_empty() {
        prompt.push_str("\nCalls made so far, and their verified results:\n");
        for step in steps {
            prompt.push_str(&format!("  {step}\n"));
        }
    }
    if let Some(reason) = correction {
        prompt.push_str(&format!(
            "\nYour last call was rejected by the runtime, which said:\n  {reason}\n\n\
             That message is a correction you can act on. Fix the arguments, call a different \
             node, or stop if there is genuinely no answer to be had.\n"
        ));
    }
    prompt
}

fn narrate_prompt(ask: &str, steps: &[String]) -> String {
    format!(
        "Answer the user's question in one to three plain sentences, using ONLY the values in \
         the verified results below.\n\nDo not calculate anything. Do not introduce any number \
         that does not appear below. If the results do not contain what was asked for, say so \
         plainly.\n\nThe user asked: {ask}\n\n{}\n",
        steps.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_command_splits_on_quotes() {
        assert_eq!(split("claude -p"), vec!["claude", "-p"]);
        assert_eq!(
            split(r#"sh -c 'echo hi' {prompt}"#),
            vec!["sh", "-c", "echo hi", "{prompt}"]
        );
        assert!(split("   ").is_empty());
    }

    #[test]
    fn a_fenced_reply_still_parses() {
        let fenced = "sure!\n```json\n{\"done\": true}\n```\n";
        assert_eq!(parse_reply(fenced).unwrap(), json!({"done": true}));
        assert_eq!(
            parse_reply("{\"stop\": \"no\"}").unwrap(),
            json!({"stop": "no"})
        );
        assert!(parse_reply("I refuse to emit JSON").is_err());
    }

    #[test]
    fn expected_params_are_a_subset_not_an_equality() {
        let actual = json!({"weapon": "Uchigatana", "soul_level": 120});
        assert!(subset(&json!({"soul_level": 120}), &actual));
        // The spelling of a number must not decide a routing eval.
        assert!(subset(&json!({"soul_level": 120.0}), &actual));
        assert!(!subset(&json!({"soul_level": 125}), &actual));
        assert!(!subset(&json!({"missing": 1}), &actual));
    }
}
