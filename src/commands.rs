//! `list`, `describe`, `call`, `attest` and `test` (§4).

use crate::attest;
use crate::cases;
use crate::contracts;
use crate::error::{
    ATTEST_ERROR, ATTEST_UNMATCHED, INPUT_SCHEMA, OK, Result, TEST_ERROR, TEST_FAILED, VouchError,
};
use crate::eval;
use crate::ledger;
use crate::manifest::{Node, toml_to_json};
use crate::markdown;
use crate::registry::{Preamble, Registry};
use crate::verify;
use serde_json::{Value as Json, json};
use std::io::Read;

pub fn list(registry: &Registry) -> Result<i32> {
    let names = registry.node_names()?;
    if names.is_empty() {
        eprintln!("no nodes found in {}", registry.nodes_dir().display());
        return Ok(OK);
    }

    let width = names.iter().map(String::len).max().unwrap_or(0);
    for name in &names {
        // One broken node must not hide the rest of the collection.
        match registry.load(name) {
            Ok(node) => println!("{name:<width$}  {}", node.manifest.purpose),
            Err(e) => println!("{name:<width$}  <unloadable: {}>", e.reason),
        }
    }
    Ok(OK)
}

/// How `describe` should render what it found.
#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    /// Laid out for a person at a terminal.
    Human,
    Json,
    /// The routing pack, for pasting into a CLAUDE.md (§5.3).
    Markdown,
    /// The same routing context, sized for a context window (§5.4).
    Compact,
    /// Names and when to reach for them, for picking a shortlist before reading schemas.
    Index,
}

/// One node's routing entry, with everything a router does not read taken out (§5.4).
///
/// `describe --all --json` is read once and then re-sent on every routing decision an agent
/// makes — six to sixteen per question in the collection this was measured against, where the
/// catalog was 112,189 characters and 28,000 tokens of a prompt that went out every time.
/// Four things were being paid for repeatedly and none of them are read by anything routing:
///
/// - **`$schema` and `title`** are for a validator and a documentation generator.
/// - **`params.*.guidance` duplicates the schema's own `description`.** Two fields, one job,
///   and in that collection every property had both. They are folded into the field a model
///   reading a JSON Schema actually looks at, keeping both texts where they differ, because
///   those strings are where the traps are recorded.
/// - **every example after the first.** One worked call is a shape to copy; the rest pay rent
///   on every decision.
/// - **contracts, output schema, reads, timeouts.** Enforcing rather than advisory, and
///   `markdown.rs` already argues the case for leaving them out of a routing pack.
///
/// Measured on that collection: 112,189 characters to 75,711, a third of it, losing nothing a
/// caller uses to choose a node or fill its arguments.
fn compact_json(node: &Node) -> Json {
    let m = &node.manifest;
    let mut schema = node.input_schema.clone();
    if let Some(map) = schema.as_object_mut() {
        map.remove("$schema");
        map.remove("title");
    }

    // Fold the guidance into the description. Neither is dropped where they say different
    // things: `params` is advice written for a caller and `description` is advice written for
    // a schema reader, and a collection that has bothered with both usually means both.
    if let Some(properties) = schema
        .get_mut("properties")
        .and_then(Json::as_object_mut)
    {
        for (name, param) in &m.params {
            let Some(property) = properties.get_mut(name).and_then(Json::as_object_mut) else {
                continue;
            };
            let guidance = param.guidance.trim();
            if guidance.is_empty() {
                continue;
            }
            let described = property
                .get("description")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();

            let merged = if described.is_empty() || guidance.contains(&described) {
                guidance.to_string()
            } else if described.contains(guidance) {
                described
            } else {
                format!("{described} {guidance}")
            };
            property.insert("description".into(), json!(merged));
        }
    }

    json!({
        "node": m.name,
        "purpose": m.purpose,
        "use_when": m.use_when,
        "not_for": m.not_for,
        "input_schema": schema,
        // Which parameters this node will not decide, and what to offer for them. A router
        // that knows before calling can ask once; one that finds out from a refusal spends a
        // decision to learn it, and a model left to guess invents a different answer per run.
        "judgements": m.params.iter().filter(|(_, p)| p.judgement)
            .map(|(k, v)| (k.clone(), json!(
                v.options.iter().map(toml_to_json).collect::<Vec<_>>()
            )))
            .collect::<serde_json::Map<_, _>>(),
        "examples": m.examples.iter().take(1)
            .map(|e| json!({"ask": e.ask, "call": toml_to_json(&e.call)}))
            .collect::<Vec<_>>(),
    })
}

/// Enough to pick a node, and nothing to call one with (§5.4).
///
/// For a caller that wants a shortlist before it reads any schema. Everything here is prose a
/// person or a model chooses by; the arguments come from `--compact` once the choice is made.
fn index_json(node: &Node) -> Json {
    let m = &node.manifest;
    json!({
        "node": m.name,
        "purpose": m.purpose,
        "use_when": m.use_when,
        "not_for": m.not_for,
    })
}

pub fn describe(registry: &Registry, name: &str, format: Format) -> Result<i32> {
    let node = registry.load(name)?;
    match format {
        // Not pretty-printed: these two exist to be small. Indentation cost 25,000 characters
        // per decision in the collection this was measured against, for whitespace nothing
        // reads.
        Format::Compact => println!("{}", serde_json::to_string(&compact_json(&node)).unwrap()),
        Format::Index => println!("{}", serde_json::to_string(&index_json(&node)).unwrap()),
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&describe_json(&node)).unwrap()
            )
        }
        Format::Markdown => print!(
            "{}",
            markdown::pack(&collection_name(registry), None, &[node])
        ),
        Format::Human => print_description(&node),
    }
    Ok(OK)
}

/// Every node at once: the collection describing itself (§5.3).
pub fn describe_all(registry: &Registry, format: Format) -> Result<i32> {
    let preamble = registry.preamble()?;
    let nodes = registry.load_all()?;

    match format {
        Format::Compact | Format::Index => {
            let render = if format == Format::Compact {
                compact_json
            } else {
                index_json
            };
            let described: Vec<Json> = nodes.iter().map(render).collect();
            let mut out = json!({
                "collection": declared_name(registry, preamble.as_ref()),
                "description": preamble.as_ref().and_then(|p| p.description.clone()),
                "nodes": described,
            });
            // The preamble's notes are collection-wide routing advice, so they belong in the
            // compact pack; an index is a shortlist and they would dwarf it.
            if format == Format::Compact {
                out["notes"] =
                    json!(preamble.as_ref().map(|p| p.notes.clone()).unwrap_or_default());
            }
            println!("{}", serde_json::to_string(&out).unwrap());
        }
        Format::Json => {
            let described: Vec<Json> = nodes.iter().map(describe_json).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "collection": declared_name(registry, preamble.as_ref()),
                    "description": preamble.as_ref().and_then(|p| p.description.clone()),
                    "notes": preamble.as_ref().map(|p| p.notes.clone()).unwrap_or_default(),
                    "nodes": described,
                }))
                .unwrap()
            );
        }
        // A human asking about a whole collection wants the same thing an agent does.
        Format::Markdown | Format::Human => {
            print!(
                "{}",
                markdown::pack(&collection_name(registry), preamble.as_ref(), &nodes)
            )
        }
    }
    Ok(OK)
}

/// The collection's directory name, used when no preamble names it.
fn collection_name(registry: &Registry) -> String {
    registry
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "collection".to_string())
}

/// What the collection calls itself: the preamble's `name`, or the directory.
///
/// `markdown::pack` has always preferred the preamble and the JSON renderer had always used
/// the directory, so the same collection answered to two different names depending on which
/// flag you passed. The preamble wins, because it is the only one anybody chose.
fn declared_name(registry: &Registry, preamble: Option<&Preamble>) -> String {
    preamble
        .and_then(|p| p.name.clone())
        .unwrap_or_else(|| collection_name(registry))
}

fn describe_json(node: &Node) -> Json {
    let m = &node.manifest;
    json!({
        "name": m.name,
        "version": m.version,
        "purpose": m.purpose,
        "use_when": m.use_when,
        "not_for": m.not_for,
        "params": m.params.iter().map(|(k, v)| (k.clone(), json!({
            "guidance": v.guidance,
            "judgement": v.judgement,
            "options": v.options.iter().map(toml_to_json).collect::<Vec<_>>(),
        }))).collect::<serde_json::Map<_, _>>(),
        "input_schema": node.input_schema,
        "output_schema": node.output_schema,
        "requires": node.requires.iter().map(contract_json).collect::<Vec<_>>(),
        "ensures": node.ensures.iter().map(contract_json).collect::<Vec<_>>(),
        "contract_strength": node.strength(),
        "reads": m.reads.iter().map(|r| json!({"path": r.path})).collect::<Vec<_>>(),
        "timeout_ms": m.timeout_ms,
        "examples": m.examples.iter()
            .map(|e| json!({"ask": e.ask, "call": toml_to_json(&e.call)}))
            .collect::<Vec<_>>(),
    })
}

/// Contract sources are stored verbatim but printed on one line — a long `in [...]` list is
/// written across several lines in the manifest and would otherwise break the layout.
fn one_line(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn contract_json(contract: &contracts::Contract) -> Json {
    match &contract.message {
        Some(message) => json!({ "expr": contract.source, "message": message }),
        None => json!({ "expr": contract.source }),
    }
}

fn print_description(node: &Node) {
    let m = &node.manifest;
    println!("{} {}\n", m.name, m.version);
    println!("{}\n", m.purpose);

    if !m.use_when.is_empty() {
        println!("Use when:");
        for line in &m.use_when {
            println!("  - {line}");
        }
        println!();
    }
    if !m.not_for.is_empty() {
        println!("Not for:");
        for line in &m.not_for {
            println!("  - {line}");
        }
        println!();
    }
    if !m.params.is_empty() {
        println!("Parameters:");
        for (name, param) in &m.params {
            let mark = if param.judgement { " [judgement]" } else { "" };
            println!("  {name}{mark}: {}", param.guidance);
            for option in &param.options {
                println!("    offer: {}", toml_to_json(option));
            }
        }
        println!();
    }

    if !node.requires.is_empty() {
        println!("Requires (checked before the node runs; failure refuses with exit 11):");
        for contract in &node.requires {
            println!("  {}", one_line(&contract.source));
            if let Some(message) = &contract.message {
                println!("    on failure: {message}");
            }
        }
        println!();
    }
    println!("Ensures (checked against the returned value; failure is a defect, exit 13):");
    for contract in &node.ensures {
        println!("  {}", one_line(&contract.source));
    }
    println!();

    let s = node.strength();
    println!(
        "Contract strength: {} postcondition{}, covering {}/{} numeric output field{}",
        s.ensures,
        if s.ensures == 1 { "" } else { "s" },
        s.numeric_fields_referenced,
        s.numeric_output_fields,
        if s.numeric_output_fields == 1 {
            ""
        } else {
            "s"
        },
    );
    if !s.unreferenced_numeric_fields.is_empty() {
        println!("  unchecked: {}", s.unreferenced_numeric_fields.join(", "));
    }
    println!();

    if !m.reads.is_empty() {
        println!("Reads:");
        for read in &m.reads {
            println!("  {}", read.path);
        }
        println!();
    }
    if !m.examples.is_empty() {
        println!("Examples:");
        for example in &m.examples {
            println!("  ask:  {}", example.ask);
            println!("  call: {}", toml_to_json(&example.call));
        }
        println!();
    }
    println!("Timeout: {}ms", m.timeout_ms);
}

/// One verified call: the pipeline in `verify`, plus recording, plus output. Every path out
/// of here is either a value that satisfied its contracts or a refusal/defect with a
/// machine-readable reason — there is no third outcome (§1.3).
pub async fn call(registry: &Registry, name: &str, input_arg: &str) -> Result<i32> {
    let node = registry.load(name)?;
    let input = read_input(input_arg)?;

    let attempt = verify::attempt(&node, &input).await;

    // Everything past the preconditions is recorded, whatever the outcome — the ledger is an
    // account of the session, not a highlight reel of the calls that worked.
    if attempt.executed {
        let entry = ledger::entry(
            &node,
            &input,
            attempt.outcome(),
            attempt.code(),
            attempt.produced.as_ref(),
        );
        let written = ledger::append(&registry.root, &ledger::session_id(), &entry);

        if attempt.verdict.is_ok() {
            // §1.2 makes recording part of the guarantee, not a side effect: the result passed
            // its contracts *and* its origin is recorded. If it cannot be recorded there is
            // nothing to vouch for, so this failure is fatal and nothing reaches stdout.
            written?;
        } else if let Err(e) = written {
            // On a failing path the original refusal or defect matters more to the caller than
            // a write that did not happen, so it is a warning and the error still stands.
            eprintln!("warning: this call was not recorded: {}", e.reason);
        }
    }

    attempt.verdict?;

    // The value satisfied every contract and its provenance is on disk. Only now.
    let value = attempt
        .produced
        .expect("a verdict that held always has a value");
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
    Ok(OK)
}

/// Run node fixtures (§7.1).
///
/// Deterministic and boolean: every case passes or the command reports which did not. Nothing
/// is written to the ledger, because a fixture is a rehearsal rather than a call anyone is
/// entitled to quote a number from.
///
/// Exit 0 when everything passed, 1 when a case failed, 2 when the run itself could not
/// happen — the same split `attest` uses, and for the same reason.
pub async fn test(registry: &Registry, name: Option<&str>, as_json: bool) -> Result<i32> {
    let recode = |e: VouchError| e.with_code(TEST_ERROR);

    let known = registry.node_names().map_err(recode)?;
    let names = match name {
        // A name that is not in the collection is a typo, not a finding about the collection:
        // the run could not happen, so it is an error rather than a failed node.
        Some(name) if !known.iter().any(|n| n == name) => {
            return Err(recode(registry.load(name).err().unwrap_or_else(|| {
                VouchError::error(format!("no node named '{name}'"))
            })));
        }
        Some(name) => vec![name.to_string()],
        None => known,
    };

    let mut reports: Vec<(String, Vec<cases::Outcome>)> = Vec::new();
    let mut without_cases: Vec<String> = Vec::new();
    // A node that will not load is a failure of the collection, not a reason to abandon the
    // run. Aborting would let one broken node hide every other result; skipping it silently
    // would let a node that cannot be called at all pass `vouch test`.
    let mut unloadable: Vec<(String, String)> = Vec::new();

    for node_name in &names {
        let node = match registry.load(node_name) {
            Ok(node) => node,
            Err(e) => {
                unloadable.push((node_name.clone(), e.reason));
                continue;
            }
        };
        match cases::load(&node.dir).map_err(recode)? {
            None => without_cases.push(node_name.clone()),
            Some(list) => {
                let mut outcomes = Vec::with_capacity(list.len());
                for case in &list {
                    outcomes.push(cases::run(&node, case).await);
                }
                reports.push((node_name.clone(), outcomes));
            }
        }
    }

    // A checking tool that reports success having checked nothing is the same false assurance
    // §3.3 refuses to load a vacuous postcondition over. Say so, and fail the run.
    if reports.is_empty() && unloadable.is_empty() {
        return Err(recode(VouchError::error(match name {
            Some(name) => format!("{name} has no cases.toml; there is nothing to test"),
            None => format!(
                "no node in {} has a cases.toml; there is nothing to test",
                registry.nodes_dir().display()
            ),
        })));
    }

    let total: usize = reports.iter().map(|(_, o)| o.len()).sum();
    let passed: usize = reports
        .iter()
        .flat_map(|(_, o)| o)
        .filter(|o| o.passed())
        .count();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "cases": total,
                "passed": passed,
                "failed": total - passed,
                "without_cases": without_cases,
                "unloadable": unloadable.iter()
                    .map(|(node, reason)| json!({ "node": node, "reason": reason }))
                    .collect::<Vec<_>>(),
                "nodes": reports.iter().map(|(node, outcomes)| json!({
                    "node": node,
                    "cases": outcomes.iter().map(|o| json!({
                        "name": o.name,
                        "passed": o.passed(),
                        "failures": o.failures,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }))
            .unwrap()
        );
    } else {
        print_test_report(&reports, &without_cases, &unloadable, total, passed);
    }

    Ok(if passed == total && unloadable.is_empty() {
        OK
    } else {
        TEST_FAILED
    })
}

fn print_test_report(
    reports: &[(String, Vec<cases::Outcome>)],
    without_cases: &[String],
    unloadable: &[(String, String)],
    total: usize,
    passed: usize,
) {
    for (node, outcomes) in reports {
        eprintln!("{node}");
        for outcome in outcomes {
            if outcome.passed() {
                eprintln!("  ok    {}", outcome.name);
                continue;
            }
            eprintln!("  FAIL  {}", outcome.name);
            for failure in &outcome.failures {
                eprintln!("          {failure}");
            }
        }
    }

    for (node, reason) in unloadable {
        eprintln!("{node}\n  FAIL  will not load\n          {reason}");
    }

    // Named rather than counted: a node with no fixtures is the gap most worth seeing, and
    // it is invisible in a pass count.
    if !without_cases.is_empty() {
        eprintln!("\nno cases.toml: {}", without_cases.join(", "));
    }

    eprint!(
        "\n{total} case{}, {passed} passed, {} failed",
        if total == 1 { "" } else { "s" },
        total - passed,
    );
    if unloadable.is_empty() {
        eprintln!();
    } else {
        eprintln!(
            "; {} node{} will not load",
            unloadable.len(),
            if unloadable.len() == 1 { "" } else { "s" },
        );
    }
}

/// Run agent routing evals (§7.2).
///
/// A model is in the loop, so the result is a **rate**: each case runs `n` times and reports
/// `9/10`. This is the regression signal on the failure mode that is otherwise invisible —
/// reword a `use_when`, watch routing accuracy fall, and without this nobody finds out until
/// a user does.
///
/// Costs tokens on every run, which is why it is not part of `cargo test`.
#[allow(clippy::too_many_arguments)]
pub async fn eval(
    registry: &Registry,
    file: Option<&str>,
    agent: &str,
    runs: usize,
    min_rate: f64,
    as_json: bool,
    resume: bool,
    max_calls: Option<usize>,
) -> Result<i32> {
    let recode = |e: VouchError| e.with_code(TEST_ERROR);

    let path = match file {
        Some(path) => std::path::PathBuf::from(path),
        None => eval::default_path(registry),
    };
    if !path.is_file() {
        return Err(recode(VouchError::error(format!(
            "no eval suite at {}; write one, or name it with --file",
            path.display()
        ))));
    }

    let suite = eval::load(&path).map_err(recode)?;
    if suite.is_empty() {
        return Err(recode(VouchError::error(format!(
            "{} declares no [[eval]] cases; there is nothing to run",
            path.display()
        ))));
    }
    if runs == 0 {
        return Err(recode(VouchError::error("-n must be at least 1")));
    }

    let agent = eval::Agent::new(agent).map_err(recode)?.budgeted(max_calls);

    // What a previous invocation already paid for. Without --resume the record starts empty,
    // so an ordinary run is unchanged and a rerun is a rerun.
    let progress = eval::state::path(&registry.root, &path);
    let already = if resume {
        eval::state::done(&progress)
    } else {
        eval::state::clear(&progress);
        Default::default()
    };
    if resume && !already.is_empty() && !as_json {
        eprintln!("resuming: {} run(s) already finished are skipped\n", already.len());
    }
    let nodes = registry.load_all().map_err(recode)?;
    let context = eval::context(registry, &nodes).map_err(recode)?;

    // The human report is printed case by case as each finishes. A suite is minutes of model
    // calls with nothing to show for them otherwise, and a run that has to be waited out in
    // silence is one nobody interrupts early when the first case is already going wrong. The
    // JSON report stays a single document, because a consumer wants one object.
    let live = !as_json;
    if live {
        eprintln!(
            "suite: {} ({} case{}, {runs} run{} each)\n",
            path.display(),
            suite.len(),
            if suite.len() == 1 { "" } else { "s" },
            if runs == 1 { "" } else { "s" },
        );
    }

    let mut results: Vec<(usize, Vec<eval::Run>)> = Vec::new();
    for (i, case) in suite.iter().enumerate() {
        let mut runs_of_case = Vec::with_capacity(runs);
        for repeat in 0..runs {
            if already.contains(&(i, repeat)) {
                continue;
            }
            // An agent command that will not run is a broken harness, not a failed case:
            // there is no rate to report, so the command fails. But whatever already ran is
            // still a result, and throwing it away wastes real model calls — a suite that
            // dies on its last case should not read the same as one that died on its first.
            match eval::run_once(&nodes, &context, &agent, case).await {
                Ok(run) => {
                    eval::state::record(&progress, i, repeat);
                    runs_of_case.push(run);
                }
                Err(e) => {
                    if live && !runs_of_case.is_empty() {
                        print_eval_case(case, &runs_of_case);
                    }
                    eprintln!(
                        "run cut short in case {} of {}, after {} of {} complete case{}. \
                         No rate is reported: the cases below are what ran, not the suite.",
                        i + 1,
                        suite.len(),
                        results.len(),
                        suite.len(),
                        if results.len() == 1 { "" } else { "s" },
                    );
                    return Err(recode(e));
                }
            }
        }
        if live && !runs_of_case.is_empty() {
            print_eval_case(case, &runs_of_case);
        }
        results.push((i, runs_of_case));
    }

    // A resumed suite reports what this invocation ran. Nothing here replays a previous one's
    // verdicts: a stale answer from an older binary reported as a current result is worse than
    // an honest partial, and the point of the record is to avoid paying twice, not to
    // reconstruct a rate from runs nobody watched.
    let skipped = already.len();
    if live {
        // What the suite cost, in the only unit this command can count. A number nobody has
        // to estimate is worth printing even when no budget was set: it is what makes the
        // next `--max-calls` an informed figure rather than a guess.
        eprintln!("\n{} agent call(s) made.", agent.spent());
        if skipped > 0 {
            eprintln!("{skipped} run(s) were skipped as already finished and are not scored.");
        }
    }

    let total: usize = results.iter().map(|(_, r)| r.len()).sum();
    let passed: usize = results
        .iter()
        .flat_map(|(_, r)| r)
        .filter(|run| run.passed())
        .count();
    let rate = passed as f64 / total as f64;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "suite": path.display().to_string(),
                "runs": runs,
                "total": total,
                "passed": passed,
                "rate": rate,
                "min_rate": min_rate,
                "cases": results.iter().map(|(i, case_runs)| {
                    let case = &suite[*i];
                    json!({
                        "ask": case.ask,
                        "passed": case_runs.iter().filter(|r| r.passed()).count(),
                        "runs": case_runs.len(),
                        "failures": case_runs.iter()
                            .flat_map(|r| r.failures.iter().cloned())
                            .collect::<Vec<_>>(),
                        "runs_detail": case_runs.iter().map(|r| json!({
                            "ending": r.ending.label(),
                            "calls": r.calls.iter()
                                .map(|(node, input)| json!({ "node": node, "input": input }))
                                .collect::<Vec<_>>(),
                            "answer": r.answer,
                            "passed": r.passed(),
                            "failures": r.failures,
                        })).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            }))
            .unwrap()
        );
    } else {
        eprintln!(
            "{passed}/{total} runs passed ({:.0}%); the floor is {:.0}%",
            rate * 100.0,
            min_rate * 100.0
        );
    }

    Ok(if rate + f64::EPSILON >= min_rate {
        OK
    } else {
        TEST_FAILED
    })
}

/// One case's result, printed as soon as it is known.
fn print_eval_case(case: &eval::Case, runs: &[eval::Run]) {
    let ok = runs.iter().filter(|r| r.passed()).count();
    eprintln!("{}\n  {ok}/{} runs passed", case.ask, runs.len());

    // The route each run took, deduplicated. A rate that has dropped is unactionable without
    // knowing which node the agent went to instead.
    let mut routes: Vec<String> = Vec::new();
    for run in runs {
        let route = match run.calls.is_empty() {
            true => format!("{}, no calls", run.ending.label()),
            false => format!(
                "{}: {}",
                run.ending.label(),
                run.calls
                    .iter()
                    .map(|(node, _)| node.as_str())
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
        };
        if !routes.contains(&route) {
            routes.push(route);
        }
    }
    for route in &routes {
        eprintln!("    [{route}]");
    }

    // Distinct reasons rather than one line per run: ten runs failing the same way is one
    // fact, and printing it ten times buries the run that failed differently.
    let mut seen: Vec<&String> = Vec::new();
    for failure in runs.iter().flat_map(|r| &r.failures) {
        if !seen.contains(&failure) {
            seen.push(failure);
            eprintln!("    {failure}");
        }
    }
    eprintln!();
}

/// Reconcile prose against the ledger (§6.2).
///
/// No model is involved — every numeral in the text is checked against the numbers the ledger
/// recorded. Exit 0 if all are accounted for, 1 if any are not.
pub fn attest_text(
    registry: &Registry,
    ledger_arg: Option<&str>,
    text_arg: &str,
    question: Option<&str>,
    include_inputs: bool,
    as_json: bool,
) -> Result<i32> {
    let recode = |e: VouchError| e.with_code(ATTEST_ERROR);

    let file = match ledger_arg {
        Some(path) => std::path::PathBuf::from(path),
        None => ledger::newest(&registry.root).ok_or_else(|| {
            recode(VouchError::error(format!(
                "no ledger found in {}; run a call first, or name one with --ledger",
                registry.root.join(".vouch/ledger").display()
            )))
        })?,
    };

    let entries = ledger::load(&file).map_err(recode)?;
    let scalars = attest::ledger_scalars(&entries, include_inputs);

    let text = read_text(text_arg).map_err(recode)?;
    let question = match question {
        Some(arg) => Some(read_text(arg).map_err(recode)?),
        None => None,
    };

    let report = attest::attest(&text, &scalars, question.as_deref());

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ledger": file.display().to_string(),
                "entries": entries.len(),
                "scalars": scalars.len(),
                "checked": report.checked,
                "matched": report.matched,
                "ignored": report.ignored,
                "unmatched": report.unmatched.iter().map(|u| json!({
                    "numeral": u.numeral.raw,
                    "line": u.numeral.line,
                    "column": u.numeral.column,
                    "offset": u.numeral.offset,
                    "context": u.context,
                })).collect::<Vec<_>>(),
                // What accounted for each figure that did check out. "Attested" is a verdict;
                // this is the working, and it is the difference between a check that can be
                // audited and one that can only be believed.
                "accounted": report.accounted.iter().map(|a| json!({
                    "numeral": a.numeral.raw,
                    "line": a.numeral.line,
                    "column": a.numeral.column,
                    "paths": a.paths,
                    "accounted_by": a.accounted_by,
                })).collect::<Vec<_>>(),
                // How many figures more than one recorded value could account for. Zero is
                // the strong reading of "attested"; a high count against a large ledger means
                // the check passed for reasons that may have nothing to do with the answer.
                "ambiguous": report.ambiguous(),
                "clean": report.is_clean(),
            }))
            .unwrap()
        );
    } else {
        print_attestation(&report, &file, entries.len(), scalars.len());
    }

    Ok(if report.is_clean() {
        OK
    } else {
        ATTEST_UNMATCHED
    })
}

fn print_attestation(
    report: &attest::Report,
    file: &std::path::Path,
    entries: usize,
    scalars: usize,
) {
    let plural = |n: usize| if n == 1 { "" } else { "s" };

    eprintln!(
        "ledger: {} ({} entr{}, {} scalar{})",
        file.display(),
        entries,
        if entries == 1 { "y" } else { "ies" },
        scalars,
        plural(scalars),
    );

    if report.is_clean() {
        eprintln!(
            "clean: {} numeral{} checked, {} matched, {} ignored",
            report.checked,
            plural(report.checked),
            report.matched,
            report.ignored,
        );
        // A clean result can be clean for weak reasons. Attestation asks whether *any*
        // recorded value rounds to a figure, so in a large ledger a wrong number can be
        // accounted for by something it has nothing to do with. Saying how often that
        // happened is the difference between "attested" and "attested, and here is how
        // firmly" — and it is the only warning a reader gets that the ledger has grown big
        // enough to start passing things on its own.
        let ambiguous = report.ambiguous();
        if ambiguous > 0 {
            eprintln!(
                "  {ambiguous} of those matched more than one recorded value; \
                 --json lists what accounted for each"
            );
        }
        return;
    }

    eprintln!(
        "UNATTESTED: {} of {} numeral{} did not come from the ledger\n",
        report.unmatched.len(),
        report.checked,
        plural(report.checked),
    );
    for found in &report.unmatched {
        eprintln!(
            "  line {}, column {}: {}\n    {}",
            found.numeral.line, found.numeral.column, found.numeral.raw, found.context,
        );
    }
}

/// `@file`, `-` for stdin, or a literal string.
fn read_text(arg: &str) -> Result<String> {
    if arg == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| VouchError::error(format!("cannot read stdin: {e}")))?;
        return Ok(buf);
    }
    match arg.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| VouchError::error(format!("cannot read {path}: {e}"))),
        None => Ok(arg.to_string()),
    }
}

/// `@file`, `-` for stdin, or a literal JSON string.
fn read_input(arg: &str) -> Result<Json> {
    let (text, origin) = if arg == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            VouchError::caller_error(INPUT_SCHEMA, format!("cannot read stdin: {e}"))
        })?;
        (buf, "stdin".to_string())
    } else if let Some(path) = arg.strip_prefix('@') {
        let text = std::fs::read_to_string(path).map_err(|e| {
            VouchError::caller_error(INPUT_SCHEMA, format!("cannot read {path}: {e}"))
        })?;
        (text, path.to_string())
    } else {
        (arg.to_string(), "--input".to_string())
    };

    serde_json::from_str(&text).map_err(|e| {
        VouchError::caller_error(INPUT_SCHEMA, format!("{origin} is not valid JSON: {e}"))
    })
}
