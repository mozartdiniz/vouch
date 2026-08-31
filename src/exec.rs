//! Subprocess execution (§2). JSON object in on stdin, JSON object out on stdout, any
//! language.
//!
//! Contract enforcement lives outside this module entirely, so the guarantee holds regardless
//! of how the node is implemented. What this module owns is the protocol boundary: a node
//! that violates it produces a defect, never a value.

use crate::error::{NODE_CRASHED, PROTOCOL, Result, TIMEOUT, VouchError};
use crate::manifest::Node;
use serde_json::Value as Json;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

/// How much of a node's stderr to quote back in a defect report.
const STDERR_TAIL: usize = 2000;

pub async fn run(node: &Node, input: &Json) -> Result<Json> {
    let name = node.manifest.name.as_str();
    let program = &node.manifest.run[0];
    let args = &node.manifest.run[1..];

    let mut child = tokio::process::Command::new(program)
        .args(args)
        // Relative paths in `run` and in `[[reads]]` are relative to the node directory, so
        // a node can be invoked from anywhere in the project.
        .current_dir(&node.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            VouchError::defect(NODE_CRASHED, format!("cannot start `{program}`: {e}"))
                .with_node(name)
        })?;

    let payload = serde_json::to_vec(input).expect("input was parsed from JSON");
    if let Some(mut stdin) = child.stdin.take() {
        // A node that reads no stdin and exits leaves a broken pipe here. That is not itself
        // the error — the exit status or the malformed stdout is — so it is not reported.
        let _ = stdin.write_all(&payload).await;
        let _ = stdin.shutdown().await;
    }

    let timeout = std::time::Duration::from_millis(node.manifest.timeout_ms);
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(
                VouchError::defect(NODE_CRASHED, format!("node failed to run: {e}"))
                    .with_node(name),
            );
        }
        Err(_) => {
            // `kill_on_drop` reaps the process when `child` is dropped on the way out.
            return Err(VouchError::refusal(
                TIMEOUT,
                format!(
                    "node exceeded its {}ms timeout and was killed",
                    node.manifest.timeout_ms
                ),
            )
            .with_node(name));
        }
    };

    let stderr = tail(&String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        let status = match output.status.code() {
            Some(code) => format!("exit {code}"),
            None => "killed by signal".to_string(),
        };
        return Err(
            VouchError::defect(NODE_CRASHED, format!("node exited with {status}"))
                .with_node(name)
                .with_details(serde_json::json!({ "stderr": stderr })),
        );
    }

    parse_stdout(&output.stdout, name, &stderr)
}

/// stdout is the payload channel (§8.2). Every language's default logger writes there, and
/// every contributor corrupts it on day one — so a violation gets a message that says what
/// happened, not a parse error.
fn parse_stdout(stdout: &[u8], node: &str, stderr: &str) -> Result<Json> {
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();

    let violation = |reason: String| {
        let details = serde_json::json!({
            "stdout": tail(trimmed),
            "stderr": stderr,
            "hint": "stdout carries the result object and nothing else; send logs and \
                     progress to stderr",
        });
        VouchError::defect(PROTOCOL, reason)
            .with_node(node)
            .with_details(details)
    };

    if trimmed.is_empty() {
        return Err(violation(
            "node wrote nothing to stdout; expected one JSON object".to_string(),
        ));
    }

    match serde_json::from_str::<Json>(trimmed) {
        Ok(Json::Object(map)) => Ok(Json::Object(map)),
        Ok(other) => Err(violation(format!(
            "node wrote a JSON {} to stdout; expected a JSON object",
            json_kind(&other)
        ))),
        Err(e) => Err(violation(format!(
            "stdout is not a single valid JSON object: {e}"
        ))),
    }
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

fn tail(text: &str) -> String {
    let trimmed = text.trim_end();
    if trimmed.len() <= STDERR_TAIL {
        return trimmed.to_string();
    }
    let start = trimmed.len() - STDERR_TAIL;
    // Do not slice through a multi-byte character.
    let start = (start..trimmed.len())
        .find(|i| trimmed.is_char_boundary(*i))
        .unwrap_or(trimmed.len());
    format!("...{}", &trimmed[start..])
}
