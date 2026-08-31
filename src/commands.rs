//! `list`, `describe`, and `call` (§4).

use crate::attest;
use crate::contracts::{self, Verdict};
use crate::error::{
    ATTEST_ERROR, ATTEST_UNMATCHED, CONTRACT_UNEVALUABLE, INPUT_SCHEMA, OK, OUTPUT_SCHEMA,
    POSTCONDITION, PRECONDITION, Result, VouchError,
};
use crate::exec;
use crate::ledger;
use crate::manifest::{Node, toml_to_json};
use crate::markdown;
use crate::registry::Registry;
use crate::schema;
use cel::Context;
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

/// One verified call. Every path out of here is either a value that satisfied its contracts
/// or a refusal/defect with a machine-readable reason — there is no third outcome (§1.3).
pub async fn call(registry: &Registry, name: &str, input_arg: &str) -> Result<i32> {
    let node = registry.load(name)?;
    let node_name = node.manifest.name.as_str();

    let input = read_input(input_arg)?;
    if !input.is_object() {
        return Err(
            VouchError::caller_error(INPUT_SCHEMA, "input must be a JSON object")
                .with_node(node_name),
        );
    }

    // --- input schema (exit 10) ---
    let errors = schema::errors(&node.input_validator, &input);
    if !errors.is_empty() {
        return Err(VouchError::caller_error(
            INPUT_SCHEMA,
            format!(
                "input does not satisfy the input schema: {}",
                errors.join("; ")
            ),
        )
        .with_node(node_name)
        .with_details(json!({ "violations": errors })));
    }

    // Types come from the schema, not from the payload (§3.2).
    let input_cel = contracts::to_cel(&input, Some(&node.input_schema), &node.input_schema);

    // --- preconditions (exit 11 / 15) ---
    let mut ctx = Context::default();
    ctx.add_variable_from_value("input", input_cel.clone());
    for contract in &node.requires {
        match contract.evaluate(&ctx) {
            Verdict::Held => {}
            Verdict::Failed => {
                return Err(VouchError::refusal(
                    PRECONDITION,
                    contract.explain("precondition failed"),
                )
                .with_node(node_name)
                .with_details(json!({ "expression": contract.source })));
            }
            Verdict::Unevaluable(why) => return Err(unevaluable(node_name, contract, &why)),
        }
    }

    // --- execution (exit 14 / 20 / 21) ---
    //
    // Everything from here is recorded, whatever the outcome — the ledger is an account of
    // the session, not a highlight reel of the calls that worked. On a failing path a ledger
    // problem is reported but does not replace the original error: a defect report matters
    // more to the caller than a write that did not happen.
    let session = ledger::session_id();
    let log = |outcome: &str, code: i32, result: Option<&Json>| {
        let entry = ledger::entry(&node, &input, outcome, code, result);
        ledger::append(&registry.root, &session, &entry)
    };

    let result = match exec::run(&node, &input).await {
        Ok(result) => result,
        Err(e) => {
            warn_if_unlogged(log(e.outcome.as_str(), e.code, None));
            return Err(e);
        }
    };

    match verify(&node, &input, input_cel, &result) {
        Err(e) => {
            warn_if_unlogged(log(e.outcome.as_str(), e.code, Some(&result)));
            Err(e)
        }
        Ok(()) => {
            // §1.2 makes recording part of the guarantee, not a side effect: the result passed
            // its contracts *and* its origin is recorded. If it cannot be recorded there is
            // nothing to vouch for, so this failure is fatal and nothing reaches stdout.
            log("ok", OK, Some(&result))?;

            // The value satisfied every contract and its provenance is on disk. Only now.
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            Ok(OK)
        }
    }
}

/// A ledger write that failed on a path already carrying a more important error.
fn warn_if_unlogged(outcome: Result<std::path::PathBuf>) {
    if let Err(e) = outcome {
        eprintln!("warning: this call was not recorded: {}", e.reason);
    }
}

/// Output schema (exit 12) and postconditions (exit 13 / 15).
fn verify(node: &Node, input: &Json, input_cel: cel::Value, result: &Json) -> Result<()> {
    let node_name = node.manifest.name.as_str();

    // `NaN` and `Infinity` (§8) are already gone by this point: the JSON parser in `exec`
    // rejects every spelling of them, so a node that emits one gets a protocol violation
    // (exit 21). No contract can ever evaluate against a non-finite number.
    let errors = schema::errors(&node.output_validator, result);
    if !errors.is_empty() {
        return Err(VouchError::defect(
            OUTPUT_SCHEMA,
            format!(
                "node returned a value that does not satisfy its output schema: {}",
                errors.join("; ")
            ),
        )
        .with_node(node_name)
        .with_details(json!({ "violations": errors })));
    }

    let result_cel = contracts::to_cel(result, Some(&node.output_schema), &node.output_schema);
    let mut ctx = Context::default();
    ctx.add_variable_from_value("input", input_cel);
    ctx.add_variable_from_value("result", result_cel);
    let _ = input;

    for contract in &node.ensures {
        match contract.evaluate(&ctx) {
            Verdict::Held => {}
            Verdict::Failed => {
                // A broken node must not leak a value to the caller, so the result is
                // reported as a detail on stderr and never written to stdout.
                return Err(VouchError::defect(
                    POSTCONDITION,
                    contract.explain("postcondition failed"),
                )
                .with_node(node_name)
                .with_details(
                    json!({ "expression": contract.source, "rejected_result": result }),
                ));
            }
            Verdict::Unevaluable(why) => return Err(unevaluable(node_name, contract, &why)),
        }
    }
    Ok(())
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

/// Fail closed (§3.2): a contract that could not be evaluated is a contract that did not
/// hold, and the call refuses.
fn unevaluable(node: &str, contract: &contracts::Contract, why: &str) -> VouchError {
    VouchError::refusal(
        CONTRACT_UNEVALUABLE,
        format!(
            "contract could not be evaluated: {} — {why}",
            contract.source
        ),
    )
    .with_node(node)
    .with_details(json!({ "expression": contract.source, "error": why }))
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
