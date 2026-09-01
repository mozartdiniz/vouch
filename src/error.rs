//! Exit-code taxonomy and structured stderr reporting (§4.1).
//!
//! Three families, and the distinction between them is the product:
//!
//! - **refusal** — no answer is available; the agent should try another approach.
//! - **defect** — the node is broken; the agent should stop trusting it.
//! - **caller_error** — the call itself was malformed.
//!
//! `error` is a fourth, undocumented-in-spec family reserved for problems with the
//! collection rather than with a call: a missing node, an unparseable manifest. It never
//! overlaps a call outcome.

use serde::Serialize;
use std::io::Write;

pub const OK: i32 = 0;
pub const ERROR: i32 = 1;
pub const INPUT_SCHEMA: i32 = 10;
pub const PRECONDITION: i32 = 11;
pub const OUTPUT_SCHEMA: i32 = 12;
pub const POSTCONDITION: i32 = 13;
pub const TIMEOUT: i32 = 14;
pub const CONTRACT_UNEVALUABLE: i32 = 15;
pub const NODE_CRASHED: i32 = 20;
pub const PROTOCOL: i32 = 21;

/// `vouch attest` exit codes (§6.2). Unmatched numerals are a *finding*, so they take exit 1
/// and the command's own failures move to 2.
pub const ATTEST_UNMATCHED: i32 = 1;
pub const ATTEST_ERROR: i32 = 2;

/// `vouch test` and `vouch eval` follow the same convention, for the same reason: a failing
/// fixture is a finding about the collection, not a failure of the command, and the two must
/// be distinguishable by a script.
pub const TEST_FAILED: i32 = 1;
pub const TEST_ERROR: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Refusal,
    Defect,
    CallerError,
    Error,
}

impl Outcome {
    /// The name the ledger records for this outcome (§6.1).
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Refusal => "refusal",
            Outcome::Defect => "defect",
            Outcome::CallerError => "caller_error",
            Outcome::Error => "error",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VouchError {
    pub outcome: Outcome,
    pub code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub type Result<T> = std::result::Result<T, VouchError>;

impl VouchError {
    fn new(outcome: Outcome, code: i32, reason: impl Into<String>) -> Self {
        VouchError {
            outcome,
            code,
            node: None,
            reason: reason.into(),
            details: None,
        }
    }

    /// No answer is available. The question is outside this node's competence, or the
    /// runtime could not establish that an answer would be sound.
    pub fn refusal(code: i32, reason: impl Into<String>) -> Self {
        Self::new(Outcome::Refusal, code, reason)
    }

    /// The node is broken. No value reaches the caller.
    pub fn defect(code: i32, reason: impl Into<String>) -> Self {
        Self::new(Outcome::Defect, code, reason)
    }

    /// The caller sent something the node cannot accept.
    pub fn caller_error(code: i32, reason: impl Into<String>) -> Self {
        Self::new(Outcome::CallerError, code, reason)
    }

    /// A problem with the collection itself, not with any particular call.
    pub fn error(reason: impl Into<String>) -> Self {
        Self::new(Outcome::Error, ERROR, reason)
    }

    /// Re-code an error for a subcommand with its own exit convention.
    ///
    /// `attest` needs this: §6.2 gives exit 1 to "found unmatched numerals", which is a
    /// finding rather than a failure, so its genuine errors move to 2.
    pub fn with_code(mut self, code: i32) -> Self {
        self.code = code;
        self
    }

    pub fn with_node(mut self, node: impl Into<String>) -> Self {
        self.node = Some(node.into());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Everything that is not the payload goes to stderr (§4).
    pub fn emit(&self) {
        let line = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"outcome":"error","code":1,"reason":"{}"}}"#,
                self.reason
            )
        });
        let mut err = std::io::stderr();
        let _ = writeln!(err, "{line}");
        let _ = err.flush();
    }
}
