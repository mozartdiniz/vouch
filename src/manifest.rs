//! `node.toml` parsing and node loading (§3.1).

use crate::contracts::Contract;
use crate::error::{Result, VouchError};
use crate::schema;
use jsonschema::Validator;
use serde::Deserialize;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn default_timeout() -> u64 {
    5000
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub purpose: String,

    // --- routing context (§5), published but never executed ---
    #[serde(default)]
    pub use_when: Vec<String>,
    #[serde(default)]
    pub not_for: Vec<String>,

    // --- execution ---
    pub run: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// Declared inputs, for provenance. Not a cache key (§6.1).
    #[serde(default)]
    pub reads: Vec<ReadDecl>,

    // --- interface ---
    pub input: toml::Value,
    pub output: toml::Value,
    #[serde(default)]
    pub params: BTreeMap<String, Param>,

    // --- contracts ---
    #[serde(default)]
    pub requires: Vec<ContractDecl>,
    pub ensures: Vec<ContractDecl>,

    #[serde(default)]
    pub examples: Vec<Example>,
}

/// A contract is written either as a bare expression:
///
/// ```toml
/// requires = ["input.soul_level >= 1"]
/// ```
///
/// or as a table carrying the message a caller should see when it does not hold (§4.2):
///
/// ```toml
/// [[requires]]
/// expr = "input.weapon in ['Uchigatana']"
/// message = "unknown weapon; call weapon-lookup for valid names"
/// ```
///
/// The table form must come after every bare top-level key in the manifest — TOML puts
/// subsequent keys inside the last-opened table — which is why `[[requires]]` and
/// `[[ensures]]` conventionally sit at the end of a `node.toml`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ContractDecl {
    Expr(String),
    Detailed {
        expr: String,
        #[serde(default)]
        message: Option<String>,
    },
}

impl ContractDecl {
    pub fn expr(&self) -> &str {
        match self {
            ContractDecl::Expr(e) => e,
            ContractDecl::Detailed { expr, .. } => expr,
        }
    }

    pub fn message(&self) -> Option<String> {
        match self {
            ContractDecl::Expr(_) => None,
            ContractDecl::Detailed { message, .. } => message.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadDecl {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub guidance: String,

    /// This parameter is a judgement the collection will not make for the caller (§3.4).
    ///
    /// Some parameters have no right answer in the data. How much vigor a build should hold
    /// back, which starting class to assume, whether the weapon is two-handed — the answer
    /// moves and no table settles it. A node that picks one is presenting an opinion as a
    /// calculation, and a node that leaves it required without saying anything is worse: the
    /// caller has to produce a value from nowhere, and a model asked to do that produces a
    /// different one each time. That was measured — fourteen of fifteen questions where a
    /// model had to invent one drifted between runs, against two of five where none did.
    ///
    /// Marking it `judgement` makes the refusal a *question*: exit 17, the parameter named,
    /// and `options` carried through so a caller can put them in front of a person without
    /// parsing prose for them.
    #[serde(default)]
    pub judgement: bool,

    /// The choices to offer when a judgement parameter is missing.
    ///
    /// Free-form JSON, because only the collection knows what a choice looks like — one value,
    /// or a set of them that go together. A caller renders these; it does not interpret them.
    #[serde(default)]
    pub options: Vec<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub ask: String,
    pub call: toml::Value,
}

/// A manifest that has been parsed, had its schemas compiled, and had its contracts checked
/// for strength. Holding one of these means the node is loadable; `call` does no further
/// validation of the node itself.
pub struct Node {
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub input_schema: Json,
    pub output_schema: Json,
    pub input_validator: Validator,
    pub output_validator: Validator,
    pub requires: Vec<Contract>,
    pub ensures: Vec<Contract>,
}

/// How much a node's postconditions actually check, reported by `describe` so a consumer can
/// weight the answer (§3.3).
#[derive(Debug, serde::Serialize)]
pub struct Strength {
    pub ensures: usize,
    pub numeric_output_fields: usize,
    pub numeric_fields_referenced: usize,
    pub unreferenced_numeric_fields: Vec<String>,
}

impl Node {
    pub fn load(dir: &Path) -> Result<Node> {
        let path = dir.join("node.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| VouchError::error(format!("cannot read {}: {e}", path.display())))?;
        let manifest: Manifest = toml::from_str(&text)
            .map_err(|e| VouchError::error(format!("cannot parse {}: {e}", path.display())))?;

        let named = |e: VouchError| e.with_node(manifest.name.clone());

        let input_schema = load_schema(&manifest.input, dir, "input").map_err(named)?;
        let output_schema = load_schema(&manifest.output, dir, "output").map_err(named)?;
        let input_validator = schema::compile(&input_schema, "input schema").map_err(named)?;
        let output_validator = schema::compile(&output_schema, "output schema").map_err(named)?;

        let requires = compile_all(&manifest.requires, "requires").map_err(named)?;
        let ensures = compile_all(&manifest.ensures, "ensures").map_err(named)?;

        // §3.3: refuse to load a node whose postconditions check nothing. A vacuous
        // `ensures` gives an agent *more* confidence than no `ensures` while checking
        // nothing, and false assurance is worse than absent assurance.
        if ensures.is_empty() {
            return Err(named(VouchError::error(
                "node declares no `ensures`; a node with no postcondition cannot be vouched for",
            )));
        }
        if !ensures.iter().any(|c| c.references("result")) {
            let listed = manifest
                .ensures
                .iter()
                .map(ContractDecl::expr)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(named(VouchError::error(format!(
                "no `ensures` expression references `result`; these postconditions check nothing \
                 about the returned value: {listed}"
            ))));
        }

        // A `requires` expression that mentions `result` is a manifest bug: preconditions run
        // before the subprocess, so `result` does not exist and every call would refuse
        // with exit 15.
        if let Some(bad) = requires.iter().find(|c| c.references("result")) {
            return Err(named(VouchError::error(format!(
                "`requires` expression references `result`, which does not exist before the \
                 node runs: {}",
                bad.source
            ))));
        }

        if manifest.run.is_empty() {
            return Err(named(VouchError::error(
                "`run` is empty; nothing to execute",
            )));
        }

        Ok(Node {
            manifest,
            dir: dir.to_path_buf(),
            input_schema,
            output_schema,
            input_validator,
            output_validator,
            requires,
            ensures,
        })
    }

    /// Which numeric output fields the postconditions mention.
    ///
    /// This is a textual scan of the expression sources, not an AST walk, so it is advisory:
    /// it can miss a field reached through an unusual expression. It is only ever used to
    /// *report* strength, never to decide whether a contract held.
    pub fn strength(&self) -> Strength {
        let numeric = schema::numeric_paths(&self.output_schema, &self.output_schema, "result");
        let sources = self
            .ensures
            .iter()
            .map(|c| c.source.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let unreferenced: Vec<String> = numeric
            .iter()
            .filter(|path| !sources.contains(path.trim_end_matches("[]")))
            .cloned()
            .collect();

        Strength {
            ensures: self.ensures.len(),
            numeric_output_fields: numeric.len(),
            numeric_fields_referenced: numeric.len() - unreferenced.len(),
            unreferenced_numeric_fields: unreferenced,
        }
    }
}

fn compile_all(decls: &[ContractDecl], label: &str) -> Result<Vec<Contract>> {
    decls
        .iter()
        .map(|decl| {
            Contract::compile(decl.expr(), decl.message()).map_err(|e| {
                VouchError::error(format!(
                    "`{label}` expression does not compile: {}\n  {e}",
                    decl.expr()
                ))
            })
        })
        .collect()
}

/// An `[input]`/`[output]` section is either a pointer to a schema file
/// (`schema = "input.schema.json"`) or an inline JSON Schema written as TOML.
fn load_schema(section: &toml::Value, dir: &Path, label: &str) -> Result<Json> {
    let table = section
        .as_table()
        .ok_or_else(|| VouchError::error(format!("[{label}] must be a table")))?;

    if let Some(rel) = table.get("schema") {
        let rel = rel.as_str().ok_or_else(|| {
            VouchError::error(format!("[{label}] `schema` must be a path string"))
        })?;
        let path = dir.join(rel);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            VouchError::error(format!(
                "cannot read {label} schema {}: {e}",
                path.display()
            ))
        })?;
        return serde_json::from_str(&text).map_err(|e| {
            VouchError::error(format!(
                "{label} schema {} is not valid JSON: {e}",
                path.display()
            ))
        });
    }

    if table.is_empty() {
        return Err(VouchError::error(format!(
            "[{label}] declares neither a `schema` path nor an inline schema"
        )));
    }
    Ok(toml_to_json(section))
}

/// TOML values map cleanly onto JSON except for datetimes, which have no JSON counterpart
/// and become strings.
pub fn toml_to_json(value: &toml::Value) -> Json {
    match value {
        toml::Value::String(s) => Json::String(s.clone()),
        toml::Value::Integer(i) => Json::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        toml::Value::Boolean(b) => Json::Bool(*b),
        toml::Value::Datetime(d) => Json::String(d.to_string()),
        toml::Value::Array(items) => Json::Array(items.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Json::Object(
            table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect(),
        ),
    }
}
