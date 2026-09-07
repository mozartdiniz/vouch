//! One attempt at a call: schema, contracts, subprocess, schema, contracts.
//!
//! This is the whole of §1.3's guarantee in one function, and it deliberately knows nothing
//! about the ledger or about stdout. `call` is this plus recording plus printing; `test` is
//! this plus a comparison, and records nothing. Both must run the identical pipeline, because
//! a fixture that passed through a different path than a real call would be testing something
//! nobody ships.

use crate::contracts::{self, Verdict};
use crate::error::{
    CONTRACT_UNEVALUABLE, INPUT_SCHEMA, JUDGEMENT_REQUIRED, OK, OUTPUT_SCHEMA, POSTCONDITION,
    PRECONDITION, Result, VouchError,
};
use crate::exec;
use crate::manifest::Node;
use crate::schema;
use cel::Context;
use serde_json::{Value as Json, json};

/// What one attempt produced, and the runtime's verdict on it.
pub struct Attempt {
    /// Whether the subprocess was started.
    ///
    /// This is exactly the ledger boundary: a precondition that fails costs nothing and
    /// records nothing, because nothing ran. Everything past it is part of the account of the
    /// session, whatever the outcome.
    pub executed: bool,

    /// What the node wrote, when it ran and spoke the protocol.
    ///
    /// Present even when a later check rejected it — the ledger records rejected results, and
    /// a failing fixture is far easier to read when it can show what came back. It never
    /// reaches stdout on a failing path.
    pub produced: Option<Json>,

    /// `Ok(())` only when the output schema and every postcondition held.
    pub verdict: Result<()>,
}

impl Attempt {
    /// The verified value. `None` unless every check held, so a caller cannot reach a result
    /// the runtime rejected by accident.
    pub fn value(&self) -> Option<&Json> {
        match self.verdict {
            Ok(()) => self.produced.as_ref(),
            Err(_) => None,
        }
    }

    pub fn code(&self) -> i32 {
        match &self.verdict {
            Ok(()) => OK,
            Err(e) => e.code,
        }
    }

    /// The name the ledger records for this outcome (§6.1).
    pub fn outcome(&self) -> &'static str {
        match &self.verdict {
            Ok(()) => "ok",
            Err(e) => e.outcome.as_str(),
        }
    }

    /// Nothing ran, so there is nothing to record and no value to report.
    fn rejected(e: VouchError) -> Attempt {
        Attempt {
            executed: false,
            produced: None,
            verdict: Err(e),
        }
    }
}

/// Run one call all the way through. Every path out of here is either a value that satisfied
/// its contracts or an error with a machine-readable reason — there is no third outcome.
pub async fn attempt(node: &Node, input: &Json) -> Attempt {
    let name = node.manifest.name.as_str();

    if !input.is_object() {
        return Attempt::rejected(
            VouchError::caller_error(INPUT_SCHEMA, "input must be a JSON object").with_node(name),
        );
    }

    // --- input schema (exit 10) ---
    let errors = schema::errors(&node.input_validator, input);
    if !errors.is_empty() {
        return Attempt::rejected(
            VouchError::caller_error(
                INPUT_SCHEMA,
                format!(
                    "input does not satisfy the input schema: {}",
                    errors.join("; ")
                ),
            )
            .with_node(name)
            .with_details(json!({ "violations": errors })),
        );
    }

    // --- judgements the collection will not make (exit 17) ---
    //
    // Checked after the schema, because a malformed call is a different conversation, and
    // before the preconditions, because those are about values and this is about a value not
    // being there. Reported one at a time in manifest order: a caller putting a question in
    // front of a person asks about one thing, and a list of four is an interrogation.
    for (param, spec) in node.manifest.params.iter().filter(|(_, p)| p.judgement) {
        if input.get(param).is_some_and(|v| !v.is_null()) {
            continue;
        }
        let mut details = json!({ "judgement": param, "guidance": spec.guidance });
        if !spec.options.is_empty() {
            // Carried through as the collection wrote them. Only it knows what a choice looks
            // like here — one value, or a set that go together — and a caller renders these
            // rather than interpreting them.
            details["options"] = json!(
                spec.options
                    .iter()
                    .map(crate::manifest::toml_to_json)
                    .collect::<Vec<_>>()
            );
        }
        return Attempt::rejected(
            VouchError::refusal(
                JUDGEMENT_REQUIRED,
                format!("`{param}` is a judgement this node will not make: {}", spec.guidance),
            )
            .with_node(name)
            .with_details(details),
        );
    }

    // Types come from the schema, not from the payload (§3.2).
    let input_cel = contracts::to_cel(input, Some(&node.input_schema), &node.input_schema);

    // --- preconditions (exit 11 / 15) ---
    let mut ctx = Context::default();
    ctx.add_variable_from_value("input", input_cel.clone());
    for contract in &node.requires {
        match contract.evaluate(&ctx) {
            Verdict::Held => {}
            Verdict::Failed => {
                return Attempt::rejected(
                    VouchError::refusal(PRECONDITION, contract.explain("precondition failed"))
                        .with_node(name)
                        .with_details(json!({ "expression": contract.source })),
                );
            }
            Verdict::Unevaluable(why) => {
                return Attempt::rejected(unevaluable(name, contract, &why));
            }
        }
    }

    // --- execution (exit 14 / 20 / 21) ---
    let produced = match exec::run(node, input).await {
        Ok(result) => result,
        Err(e) => {
            return Attempt {
                executed: true,
                produced: None,
                verdict: Err(e),
            };
        }
    };

    let verdict = check(node, input_cel, &produced);
    Attempt {
        executed: true,
        produced: Some(produced),
        verdict,
    }
}

/// Output schema (exit 12) and postconditions (exit 13 / 15).
fn check(node: &Node, input_cel: cel::Value, result: &Json) -> Result<()> {
    let name = node.manifest.name.as_str();

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
        .with_node(name)
        .with_details(json!({ "violations": errors })));
    }

    let result_cel = contracts::to_cel(result, Some(&node.output_schema), &node.output_schema);
    let mut ctx = Context::default();
    ctx.add_variable_from_value("input", input_cel);
    ctx.add_variable_from_value("result", result_cel);

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
                .with_node(name)
                .with_details(
                    json!({ "expression": contract.source, "rejected_result": result }),
                ));
            }
            Verdict::Unevaluable(why) => return Err(unevaluable(name, contract, &why)),
        }
    }
    Ok(())
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
