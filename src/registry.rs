//! Node discovery (§2.2).

use crate::error::{Result, VouchError};
use crate::manifest::Node;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub struct Registry {
    pub root: PathBuf,
}

/// Collection-level context (§5.2).
///
/// Per-node `use_when` and `not_for` cannot say what is true of the *set*: what the
/// collection covers as a whole, and how to disambiguate nodes that overlap. "Call triage
/// first for any ticket question" belongs to no single node, so without this it lives only
/// in a README that no agent reads.
///
/// Every field is optional, and so is the file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preamble {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Registry {
    /// Find the project root by walking up from `start` looking for a `nodes/` directory or
    /// a `.vouch/` directory, so the CLI works from anywhere inside a project.
    pub fn discover(start: &Path) -> Result<Registry> {
        let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        for dir in start.ancestors() {
            if dir.join("nodes").is_dir() || dir.join(".vouch").is_dir() {
                return Ok(Registry {
                    root: dir.to_path_buf(),
                });
            }
        }
        Err(VouchError::error(format!(
            "no vouch project found in {} or any parent directory (looking for a `nodes/` or \
             `.vouch/` directory)",
            start.display()
        )))
    }

    pub fn nodes_dir(&self) -> PathBuf {
        self.root.join("nodes")
    }

    /// The collection's own description, if it has written one.
    ///
    /// A missing file is not an error — a collection of well-described nodes is usable
    /// without one. A *malformed* file is an error, because silently ignoring context the
    /// author meant to publish is worse than refusing to start.
    pub fn preamble(&self) -> Result<Option<Preamble>> {
        let path = self.root.join(".vouch").join("registry.toml");
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| VouchError::error(format!("cannot read {}: {e}", path.display())))?;

        toml::from_str(&text)
            .map(Some)
            .map_err(|e| VouchError::error(format!("cannot parse {}: {e}", path.display())))
    }

    /// Every node that loads, in listing order. Nodes that fail their contract-strength gate
    /// are skipped and named on stderr: they cannot be called, so publishing them to an agent
    /// would only invite a failure.
    pub fn load_all(&self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        for name in self.node_names()? {
            match self.load(&name) {
                Ok(node) => nodes.push(node),
                Err(e) => eprintln!("warning: skipping {name}: {}", e.reason),
            }
        }
        Ok(nodes)
    }

    /// Directory names of every node in the collection, sorted. A directory without a
    /// `node.toml` is not a node and is skipped silently.
    pub fn node_names(&self) -> Result<Vec<String>> {
        let dir = self.nodes_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| VouchError::error(format!("cannot read {}: {e}", dir.display())))?;

        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("node.toml").is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn load(&self, name: &str) -> Result<Node> {
        let dir = self.nodes_dir().join(name);
        if !dir.join("node.toml").is_file() {
            let known = self.node_names().unwrap_or_default();
            let suggestion = if known.is_empty() {
                "the collection has no nodes".to_string()
            } else {
                format!("known nodes: {}", known.join(", "))
            };
            return Err(VouchError::error(format!(
                "no node named '{name}'; {suggestion}"
            )));
        }
        let node = Node::load(&dir)?;

        // The directory name is the routing key an agent uses; a mismatch means `describe`
        // and `call` would disagree about what to type.
        if node.manifest.name != name {
            return Err(VouchError::error(format!(
                "node in nodes/{name}/ declares name '{}'; the manifest name must match its \
                 directory",
                node.manifest.name
            )));
        }
        Ok(node)
    }
}
