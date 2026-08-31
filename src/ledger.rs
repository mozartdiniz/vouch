//! The append-only record of what was called and what came back (§6.1).
//!
//! §1.2 states the guarantee as two halves: the result passed its contracts, *and* its origin
//! is recorded. This module is the second half. Every call appends one line — successes,
//! refusals and defects alike — so the ledger is a complete account of a session rather than
//! a highlight reel of the calls that worked.
//!
//! `reads` is an audit record, not a cache key. It says which bytes produced this number, so
//! that tomorrow someone can ask whether the data file has changed since.

use crate::error::{Result, VouchError};
use crate::manifest::Node;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Which session a call belongs to.
///
/// An agent harness should set `VOUCH_SESSION` once per conversation, which is what makes
/// attestation tight: a smaller ledger is a stricter check, because a numeral can only be
/// accounted for by a call that actually happened in *this* session. Falling back to the
/// date keeps a plain shell usable, at the cost of a looser scope.
pub fn session_id() -> String {
    match std::env::var("VOUCH_SESSION") {
        Ok(id) if !id.trim().is_empty() => sanitize(&id),
        _ => chrono::Local::now().format("%Y%m%d").to_string(),
    }
}

/// Session ids reach the filesystem, so they may not wander out of the ledger directory.
fn sanitize(id: &str) -> String {
    id.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn path(root: &Path, session: &str) -> PathBuf {
    root.join(".vouch")
        .join("ledger")
        .join(format!("session-{session}.jsonl"))
}

/// The most recently modified session file, for `attest` when no ledger is named.
pub fn newest(root: &Path) -> Option<PathBuf> {
    let dir = root.join(".vouch").join("ledger");
    let mut sessions: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    sessions.pop().map(|(_, path)| path)
}

/// Every numeric leaf in a value, as dotted paths (§6.1).
///
/// Numbers are kept as JSON numbers rather than floats so that an integer stays an integer:
/// a `strength` of 16 should not be recorded as 16.0 and then quoted back that way.
pub fn scalars(value: &Json, prefix: &str) -> BTreeMap<String, Json> {
    let mut out = BTreeMap::new();
    flatten(value, prefix, &mut out);
    out
}

fn flatten(value: &Json, path: &str, out: &mut BTreeMap<String, Json>) {
    match value {
        Json::Number(_) => {
            out.insert(path.to_string(), value.clone());
        }
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                flatten(item, &format!("{path}[{i}]"), out);
            }
        }
        Json::Object(fields) => {
            for (key, sub) in fields {
                flatten(sub, &format!("{path}.{key}"), out);
            }
        }
        // Booleans, strings and null carry no figure to attest against.
        _ => {}
    }
}

/// Hash each declared `[[reads]]` path, so the entry says which bytes were in play.
fn read_records(node: &Node) -> Vec<Json> {
    node.manifest
        .reads
        .iter()
        .map(|declared| {
            let path = node.dir.join(&declared.path);
            match std::fs::read(&path) {
                Ok(bytes) => json!({
                    "path": declared.path,
                    "sha256": hex(&Sha256::digest(&bytes)),
                    "bytes": bytes.len(),
                }),
                // A declared read that cannot be opened is worth recording as such. It is not
                // fatal — the node may not have needed it — but it is exactly the kind of
                // thing an audit wants to see.
                Err(e) => json!({ "path": declared.path, "error": e.to_string() }),
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One line of the ledger.
///
/// Key order in the serialised JSON is sorted, because `serde_json` maps are BTree-backed by
/// default — the canonical form §8 asks for, without extra work.
pub fn entry(node: &Node, input: &Json, outcome: &str, code: i32, result: Option<&Json>) -> Json {
    let mut entry = serde_json::Map::new();
    entry.insert(
        "ts".into(),
        json!(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
    );
    entry.insert("node".into(), json!(node.manifest.name));
    entry.insert("version".into(), json!(node.manifest.version));
    entry.insert("input".into(), input.clone());
    entry.insert("reads".into(), json!(read_records(node)));
    entry.insert("outcome".into(), json!(outcome));
    entry.insert("code".into(), json!(code));

    if let Some(result) = result {
        entry.insert("result".into(), result.clone());
        entry.insert("scalars".into(), json!(scalars(result, "result")));
    }
    Json::Object(entry)
}

/// Append one entry, creating `.vouch/ledger/` if it is not there yet.
pub fn append(root: &Path, session: &str, entry: &Json) -> Result<PathBuf> {
    let file = path(root, session);
    let dir = file.parent().expect("the ledger path always has a parent");

    std::fs::create_dir_all(dir)
        .map_err(|e| VouchError::error(format!("cannot create {}: {e}", dir.display())))?;

    let mut handle = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|e| VouchError::error(format!("cannot open {}: {e}", file.display())))?;

    writeln!(
        handle,
        "{}",
        serde_json::to_string(entry).expect("an entry serialises")
    )
    .map_err(|e| VouchError::error(format!("cannot write to {}: {e}", file.display())))?;

    Ok(file)
}

/// Read every entry from a ledger file, ignoring blank lines.
pub fn load(file: &Path) -> Result<Vec<Json>> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| VouchError::error(format!("cannot read {}: {e}", file.display())))?;

    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line).map_err(|e| {
                VouchError::error(format!(
                    "{}:{}: not a valid ledger entry: {e}",
                    file.display(),
                    i + 1
                ))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_finds_every_numeric_leaf() {
        let result = json!({
            "attack_rating": 512.4,
            "weapon": "Lothric Knight Sword",
            "requirements_met": true,
            "stats": { "strength": 16, "dexterity": 40 },
            "breakpoints": [10, 20]
        });
        let found = scalars(&result, "result");

        assert_eq!(found["result.attack_rating"], json!(512.4));
        assert_eq!(found["result.stats.strength"], json!(16));
        assert_eq!(found["result.breakpoints[1]"], json!(20));
        // Strings and booleans carry no figure to check.
        assert!(!found.contains_key("result.weapon"));
        assert!(!found.contains_key("result.requirements_met"));
        assert_eq!(found.len(), 5);
    }

    #[test]
    fn integers_stay_integers() {
        let found = scalars(&json!({ "n": 16 }), "result");
        assert_eq!(
            found["result.n"].to_string(),
            "16",
            "16 must not become 16.0"
        );
    }

    #[test]
    fn session_ids_cannot_escape_the_ledger_directory() {
        assert_eq!(sanitize("../../etc/passwd"), "------etc-passwd");
        assert_eq!(sanitize("conversation-42"), "conversation-42");
    }
}
