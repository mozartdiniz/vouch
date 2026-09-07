//! The markdown routing pack (§5.3).
//!
//! `vouch describe --all --md` emits the registry preamble plus every node's routing context
//! as markdown. A user pastes it into `CLAUDE.md`, or an agent runs the command at session
//! start. That is the entire integration story — no protocol, and no config file format for
//! anyone to learn.
//!
//! What goes in is *advisory* material: purpose, when to reach for a node, when not to, what
//! its parameters mean, and worked examples. What stays out is the contracts. They are
//! enforcing rather than advisory, and a caller does not need to read a precondition to make
//! a good call — if they get it wrong the refusal will tell them, in words written for
//! someone who can act on them (§4.2). Publishing them here would bloat the context an agent
//! carries in every turn to no purpose.

use crate::manifest::Node;
use crate::registry::Preamble;
use crate::schema;
use serde_json::Value as Json;
use std::fmt::Write;

/// The whole pack: preamble, usage, then one section per node.
pub fn pack(collection: &str, preamble: Option<&Preamble>, nodes: &[Node]) -> String {
    let mut out = String::new();
    let title = preamble
        .and_then(|p| p.name.as_deref())
        .unwrap_or(collection);

    let _ = writeln!(out, "# {title}\n");

    if let Some(preamble) = preamble {
        if let Some(description) = &preamble.description {
            let _ = writeln!(out, "{description}\n");
        }
        if !preamble.notes.is_empty() {
            for note in &preamble.notes {
                let _ = writeln!(out, "- {note}");
            }
            out.push('\n');
        }
    }

    out.push_str(USAGE);

    if nodes.is_empty() {
        let _ = writeln!(out, "\nThis collection currently has no callable nodes.");
        return out;
    }

    for node in nodes {
        out.push('\n');
        out.push_str(section(node).trim_end());
        out.push('\n');
    }
    out
}

/// How to actually call these. A pack that describes nodes without saying how to invoke one
/// is not an integration story, and the exit-code families are the part a caller most needs
/// to act on differently.
const USAGE: &str = "\
Call these with `vouch call <node> --input '{...}'`. The result is a JSON object on stdout.
Everything else — including the reason for any failure — goes to stderr.

Every call ends in one of three things, and they call for different responses:

- **exit 0** — the value satisfied the node's contracts. Use it. Do not recompute anything
  from it; quote its figures as they are.
- **exit 11, 14 or 15** — a refusal. No answer is available from this node. The reason is
  written to be acted on: fix the arguments, call a different node, or tell the user the
  answer does not exist. Never substitute your own number.
- **exit 10, 12, 13, 20 or 21** — a caller error or a broken node. Correct the input if it
  was rejected; otherwise report the node as broken rather than working around it.

A refusal is a good outcome when it is the true one. No answer beats a wrong one.
";

fn section(node: &Node) -> String {
    let m = &node.manifest;
    let mut out = String::new();

    let _ = writeln!(out, "## {}\n", m.name);
    let _ = writeln!(out, "{}\n", m.purpose);

    if !m.use_when.is_empty() {
        let _ = writeln!(out, "**Use when**\n");
        for line in &m.use_when {
            let _ = writeln!(out, "- {line}");
        }
        out.push('\n');
    }
    if !m.not_for.is_empty() {
        let _ = writeln!(out, "**Not for**\n");
        for line in &m.not_for {
            let _ = writeln!(out, "- {line}");
        }
        out.push('\n');
    }

    let parameters = parameters(node);
    if !parameters.is_empty() {
        let _ = writeln!(out, "**Parameters**\n");
        for line in parameters {
            let _ = writeln!(out, "{line}");
        }
        out.push('\n');
    }

    if !m.examples.is_empty() {
        let _ = writeln!(out, "**Examples**\n");
        for example in &m.examples {
            let call = crate::manifest::toml_to_json(&example.call);
            let _ = writeln!(out, "- \"{}\"", example.ask);
            let _ = writeln!(
                out,
                "  ```\n  vouch call {} --input '{}'\n  ```",
                m.name,
                serde_json::to_string(&call).unwrap_or_default()
            );
        }
        out.push('\n');
    }
    out
}

/// One line per input field: name, type, whether it is required, and the author's guidance.
///
/// The full JSON Schema is deliberately not reproduced. A caller needs to know that
/// `soul_level` is a required integer and what it means; the rest is noise in a context
/// window, and `describe <node> --json` has it for anyone who needs more.
fn parameters(node: &Node) -> Vec<String> {
    let schema = &node.input_schema;
    let Some(properties) = schema.get("properties").and_then(Json::as_object) else {
        return Vec::new();
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Json::as_array)
        .map(|names| names.iter().filter_map(Json::as_str).collect())
        .unwrap_or_default();

    properties
        .iter()
        .map(|(name, property)| {
            let property = schema::deref(property, schema);
            let kind = schema::types(property).join(" or ");
            let kind = if kind.is_empty() {
                "any".to_string()
            } else {
                kind
            };

            let obligation = if required.contains(&name.as_str()) {
                "required"
            } else {
                "optional"
            };

            let guidance = node
                .manifest
                .params
                .get(name)
                .map(|p| p.guidance.clone())
                .or_else(|| {
                    property
                        .get("description")
                        .and_then(Json::as_str)
                        .map(str::to_string)
                });

            match guidance {
                Some(text) => format!("- `{name}` ({kind}, {obligation}) — {text}"),
                None => format!("- `{name}` ({kind}, {obligation})"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_without_a_preamble_still_names_the_collection() {
        let out = pack("support-triage", None, &[]);
        assert!(out.starts_with("# support-triage\n"));
        assert!(out.contains("no callable nodes"));
    }

    #[test]
    fn the_preamble_leads_and_its_name_wins() {
        let preamble = Preamble {
            name: Some("ds3-tools".into()),
            description: Some("Build optimization over a local CSV.".into()),
            notes: vec!["All numbers come from patch 1.15.".into()],
            ..Default::default()
        };
        let out = pack("ignored-dir-name", Some(&preamble), &[]);

        assert!(out.starts_with("# ds3-tools\n"));
        assert!(out.contains("Build optimization over a local CSV."));
        assert!(out.contains("- All numbers come from patch 1.15."));
    }

    /// The usage section is what makes the pack an integration story rather than a catalogue.
    #[test]
    fn usage_explains_what_each_outcome_calls_for() {
        let out = pack("c", None, &[]);
        assert!(out.contains("vouch call <node> --input"));
        assert!(out.contains("exit 0"));
        assert!(out.contains("exit 11, 14 or 15"));
        assert!(out.contains("No answer beats a wrong one."));
    }
}
