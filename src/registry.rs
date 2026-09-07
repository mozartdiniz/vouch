//! Node discovery (§2.2).

use crate::contracts::Contract;
use crate::error::{Result, VouchError};
use crate::manifest::Node;
use serde_json::Value as Json;
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
    /// Contracts the whole collection makes, rather than one node (§3.5).
    #[serde(default)]
    pub requires: Vec<Shared>,
    #[serde(default)]
    pub ensures: Vec<Shared>,
}

/// A contract stated once and applied to every node that takes the parameter it is about (§3.5).
///
/// A contract written in one node is a contract about that node. The thing a collection
/// actually wants to say is *"a stat is 1 to 99, wherever a stat appears"* — and the only way
/// to say it was to write it into every manifest and remember to write it into the next one
/// too. That is not a guard, it is a habit, and habits are how the same defect arrives in a
/// node written months after it was fixed elsewhere.
///
/// `when` names the property the contract is about, in the object that contract inspects: a
/// `[[requires]]` attaches to every node whose **input** schema declares it, a `[[ensures]]`
/// to every node whose **output** schema does. Each is evaluated only when that property is
/// actually present — an optional one left out would otherwise make the expression
/// unevaluable, which fails closed (§3.2) and refuses a perfectly legal call, so the runtime
/// scopes it rather than asking every author to remember `!has(input.x) || ...`.
///
/// What this cannot do is read the node's data. A CEL contract sees `input` and `result` and
/// nothing else, so *"the weapon must exist"* is not expressible here and never was; that is
/// what a node refusing on its own data is for (exit 3, §4.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shared {
    /// The input property this contract is about. Nodes without it are untouched.
    pub when: String,
    pub expr: String,
    pub message: Option<String>,
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
        let mut node = Node::load(&dir)?;
        self.apply_shared(&mut node)?;

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

    /// Attach the collection's shared contracts to a node that takes the parameter each is
    /// about (§3.5).
    ///
    /// Shared contracts go **first**. A collection-wide rule is the broader statement, and a
    /// caller who breaks both should be told about the general one — "a stat is 1 to 99"
    /// reads better than a node's more specific complaint about the same value.
    fn apply_shared(&self, node: &mut Node) -> Result<()> {
        let Some(preamble) = self.preamble()? else {
            return Ok(());
        };
        if preamble.requires.is_empty() && preamble.ensures.is_empty() {
            return Ok(());
        }

        // `when` names the property the contract is about, in the object that contract
        // inspects: the input for a precondition, the result for a postcondition. So a
        // precondition attaches to nodes that *take* the parameter and a postcondition to
        // nodes that *return* it, and each is scoped to that object.
        //
        // Getting this wrong is quiet rather than loud, which is why it is spelled out: an
        // `ensures` scoped to the input would skip every call that left an optional parameter
        // out, and the node would return the very value the collection said it never returns
        // with nothing to catch it.
        let declares = |schema: &Json, name: &str| {
            schema.get("properties").and_then(|p| p.get(name)).is_some()
        };

        let compile = |shared: &Shared, object: &str| -> Result<Contract> {
            // Scoped to the property's presence, so an optional one left out cannot turn a
            // collection-wide rule into a refusal of a legal call — an unevaluable contract
            // fails closed (§3.2), which is right for a rule someone wrote about this node
            // and wrong for one written about the collection.
            let source = format!("!has({object}.{}) || ({})", shared.when, shared.expr);
            Contract::compile(&source, shared.message.clone()).map_err(|e| {
                VouchError::error(format!(
                    "cannot compile the collection contract on `{}`: {e}",
                    shared.when
                ))
            })
        };

        let mut requires = Vec::new();
        for shared in preamble
            .requires
            .iter()
            .filter(|s| declares(&node.input_schema, &s.when))
        {
            requires.push(compile(shared, "input")?);
        }
        requires.append(&mut node.requires);
        node.requires = requires;

        let mut ensures = Vec::new();
        for shared in preamble
            .ensures
            .iter()
            .filter(|s| declares(&node.output_schema, &s.when))
        {
            ensures.push(compile(shared, "result")?);
        }
        ensures.append(&mut node.ensures);
        node.ensures = ensures;

        Ok(())
    }
}
