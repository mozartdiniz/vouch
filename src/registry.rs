//! Node discovery (§2.2).

use crate::error::{Result, VouchError};
use crate::manifest::Node;
use std::path::{Path, PathBuf};

pub struct Registry {
    pub root: PathBuf,
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
