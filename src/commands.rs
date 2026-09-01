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
use crate::registry::Registry;
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
}

pub fn describe(registry: &Registry, name: &str, format: Format) -> Result<i32> {
    let node = registry.load(name)?;
    match format {
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
        Format::Json => {
            let described: Vec<Json> = nodes.iter().map(describe_json).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "collection": collection_name(registry),
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

fn describe_json(node: &Node) -> Json {
    let m = &node.manifest;
    json!({
        "name": m.name,
        "version": m.version,
        "purpose": m.purpose,
        "use_when": m.use_when,
        "not_for": m.not_for,
        "params": m.params.iter().map(|(k, v)| (k.clone(), json!({"guidance": v.guidance})))
            .collect::<serde_json::Map<_, _>>(),
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
            println!("  {name}: {}", param.guidance);
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
pub async fn eval(
    registry: &Registry,
    file: Option<&str>,
    agent: &str,
    runs: usize,
    min_rate: f64,
    as_json: bool,
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

    let agent = eval::Agent::new(agent).map_err(recode)?;
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
        for _ in 0..runs {
            // An agent command that will not run at all is a broken harness, not a failed
            // case: there is no rate to report, so the whole command fails.
            runs_of_case.push(
                eval::run_once(&nodes, &context, &agent, case)
                    .await
                    .map_err(recode)?,
            );
        }
        if live {
            print_eval_case(case, &runs_of_case);
        }
        results.push((i, runs_of_case));
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
    let values: Vec<f64> = scalars.values().copied().collect();

    let text = read_text(text_arg).map_err(recode)?;
    let question = match question {
        Some(arg) => Some(read_text(arg).map_err(recode)?),
        None => None,
    };

    let report = attest::attest(&text, &values, question.as_deref());

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ledger": file.display().to_string(),
                "entries": entries.len(),
                "scalars": values.len(),
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
                "clean": report.is_clean(),
            }))
            .unwrap()
        );
    } else {
        print_attestation(&report, &file, entries.len(), values.len());
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
