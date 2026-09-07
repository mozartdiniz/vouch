//! `describe --compact` and `--index` (§5.4).
//!
//! `describe --all --json` is read once and then re-sent on every routing decision an agent
//! makes. In the collection this was sized against that was 28,000 tokens going out six to
//! sixteen times per question, and most of it was not read by anything routing. These two
//! shapes exist so that a caller does not have to write its own trimmer — which is what that
//! collection had done, character for character.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

const SHAPES: &str = "tests/fixtures-describe";

fn vouch(args: &[&str]) -> Output {
    let mut full = vec!["-C", SHAPES];
    full.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_vouch"))
        .args(&full)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("vouch runs")
}

fn json(args: &[&str]) -> Value {
    let out = vouch(args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

/// The `verbose` entry, by name. Not by index: the collection gained a second node and every
/// test that reached for `nodes[0]` started reading a different one.
fn only_node(args: &[&str]) -> Value {
    json(args)["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .find(|n| n["node"] == "verbose")
        .expect("the verbose node is in the pack")
        .clone()
}

#[test]
fn compact_drops_what_only_a_validator_reads() {
    let schema = &only_node(&["describe", "--all", "--compact"])["input_schema"];
    assert!(schema.get("$schema").is_none(), "$schema is for a validator");
    assert!(schema.get("title").is_none(), "title is for a doc generator");
    // The parts a caller fills arguments from are all still here.
    assert!(schema["properties"]["n"].is_object());
    assert_eq!(schema["required"][0], "n");
}

/// `params.*.guidance` and a property's `description` are two fields for one job. Folding them
/// is the largest single saving, and dropping either would lose text: guidance is where the
/// traps get recorded, descriptions are what a schema reader looks at.
#[test]
fn compact_folds_guidance_into_the_description_without_losing_either() {
    let properties = &only_node(&["describe", "--all", "--compact"])["input_schema"]["properties"];

    let n = properties["n"]["description"].as_str().unwrap();
    assert!(n.contains("How many."), "the description survives: {n}");
    assert!(n.contains("never one you worked out"), "the guidance survives: {n}");

    // Guidance that only restates the description must not be printed twice.
    assert_eq!(properties["tag"]["description"], "A label.");
}

#[test]
fn compact_keeps_one_example_and_leaves_the_enforcing_material_out() {
    let node = only_node(&["describe", "--all", "--compact"]);
    assert_eq!(node["examples"].as_array().unwrap().len(), 1);
    assert_eq!(node["examples"][0]["ask"], "the first example, which is a shape to copy");

    for absent in ["requires", "ensures", "output_schema", "contract_strength", "reads", "params"] {
        assert!(node.get(absent).is_none(), "{absent} is not routing context");
    }
}

/// The point of the shape. Pretty-printing this cost 25,000 characters per decision in the
/// collection it was measured against, for whitespace nothing reads.
#[test]
fn compact_is_not_pretty_printed_and_json_still_is() {
    let compact = String::from_utf8(vouch(&["describe", "--all", "--compact"]).stdout).unwrap();
    assert!(!compact.contains("\n  "), "compact must not be indented");

    let full = String::from_utf8(vouch(&["describe", "--all", "--json"]).stdout).unwrap();
    assert!(full.contains("\n  "), "--json stays readable");
    assert!(compact.len() < full.len() / 2, "{} vs {}", compact.len(), full.len());
}

#[test]
fn the_index_is_enough_to_choose_a_node_and_nothing_to_call_one() {
    let node = only_node(&["describe", "--all", "--index"]);
    let keys: Vec<&str> = node.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["node", "not_for", "purpose", "use_when"]);

    // Collection notes are routing advice worth carrying in the pack and would dwarf a
    // shortlist, so the index leaves them out and the compact pack keeps them.
    assert!(json(&["describe", "--all", "--index"]).get("notes").is_none());
    assert!(json(&["describe", "--all", "--compact"])["notes"][0]
        .as_str()
        .unwrap()
        .contains("collection-wide note"));
}

/// Both shapes work for one node too, so a caller can re-read a single entry after picking it
/// out of the index without pulling the whole collection again.
#[test]
fn both_shapes_work_for_a_single_node() {
    assert_eq!(json(&["describe", "verbose", "--compact"])["node"], "verbose");
    assert_eq!(json(&["describe", "verbose", "--index"])["node"], "verbose");
}

/// `markdown::pack` has always preferred the preamble's name and the JSON renderer used the
/// directory, so one collection answered to two names depending on the flag.
#[test]
fn every_shape_agrees_on_what_the_collection_is_called() {
    for shape in [["describe", "--all", "--json"], ["describe", "--all", "--compact"],
                  ["describe", "--all", "--index"]] {
        assert_eq!(json(&shape)["collection"], "describe-shapes", "{shape:?}");
    }
    let pack = String::from_utf8(vouch(&["describe", "--all", "--md"]).stdout).unwrap();
    assert!(pack.starts_with("# describe-shapes\n"), "{pack}");
}

// ---------------------------------------------------------------- judgements (§3.5)

/// Some parameters have no right answer in the data — how much to hold back, which class to
/// assume. A node that picks one presents an opinion as a calculation; a node that requires
/// one and says nothing makes the caller produce a value from nowhere, and a model asked to do
/// that produces a different one per run. The refusal is a question, and says so.
#[test]
fn a_missing_judgement_is_a_question_not_a_bad_call() {
    let out = vouch(&["call", "judged", "--input", "{}"]);
    assert_eq!(out.status.code(), Some(17));

    let text = String::from_utf8_lossy(&out.stderr);
    let report: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(report["outcome"], "refusal");
    assert_eq!(report["details"]["judgement"], "floor");
    // Carried through as the collection wrote them, so a caller can render the choices
    // without parsing the reason for them.
    assert_eq!(report["details"]["options"][0]["floor"], 40);
    assert_eq!(report["details"]["options"][1]["label"], "survivability first");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_judgement_that_was_supplied_is_not_asked_about() {
    let out = vouch(&["call", "judged", "--input", r#"{"floor": 40}"#]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["n"], 40);
}

/// The distinction that makes exit 17 worth having. An ordinary optional parameter left out is
/// not a question; treating it as one would interrogate a user about things that have answers.
#[test]
fn an_ordinary_optional_parameter_is_not_a_judgement() {
    let out = vouch(&["call", "judged", "--input", r#"{"floor": 1}"#]);
    assert_eq!(out.status.code(), Some(0), "`tag` was left out and must not be asked about");
}

/// A router that learns this from a refusal has already spent a decision. The pack says which
/// parameters will ask, and what to offer, before the first call.
#[test]
fn the_compact_pack_says_which_parameters_will_ask() {
    let nodes = json(&["describe", "--all", "--compact"])["nodes"].clone();
    let judged = nodes
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["node"] == "judged")
        .expect("the judged node is in the pack");

    assert_eq!(judged["judgements"]["floor"][0]["label"], "balanced");
    // A node with nothing to ask about carries an empty map, not a surprise.
    let verbose = nodes.as_array().unwrap().iter().find(|n| n["node"] == "verbose").unwrap();
    assert_eq!(verbose["judgements"].as_object().unwrap().len(), 0);
}
