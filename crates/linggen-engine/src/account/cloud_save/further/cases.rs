//! The cases every interpreter of `cloud.further` must answer alike. The file is one for three repos: its owner is
//! skills lingjing/tests/further-cases.json, and the phone (linggen-mobile test/lingjing_further_cases_test.dart) runs
//! the same cases through its own reader (lingjing_further.dart). Given the same two saves they name the same winner.
//!
//! Change a case: edit the owner, copy it here and to the phone, and set `PINNED` in the three tests (this one, the
//! phone's, skills tests/further-cases.test.mjs). The pin holds the copies together with no sibling checkout (CI has
//! none): a copy edited on one side only goes red in its own repo.
//!
//! The rule in the file is the resolved form the phone is shipped (each `rank`'s order as its ids). The engine reads an
//! order from a file of the skill, so each list is written out to a scratch folder and the declaration points at it.

use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};

const FILE: &str = include_str!("further-cases.json");
const PINNED: &str = "9730cdfee72c1531490c570821e3576394c67436650bc6e6b8cd18e6074ce16d";

fn doc() -> Value {
    serde_json::from_str(FILE).expect("further-cases.json is JSON")
}

/// The file's rule, read as the engine reads a declaration.
fn shared_rule(doc: &Value) -> Rule {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("further-cases-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut declared = doc["rule"]["further"].clone();
    for (i, e) in declared["by"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let Some(ids) = e.get("order").cloned() else {
            continue;
        };
        let name = format!("order-{i}.json");
        std::fs::write(root.join(&name), json!({ "ids": ids }).to_string()).unwrap();
        e["order"] = json!({"file": name, "list": "ids"});
    }
    rule("shared-cases", &root, &declared).expect("the declared rule is one the engine reads")
}

/// One side of a case: null is no file, an object is the save, a string is the file's raw text.
fn side(file: &str, v: &Value) -> Files {
    let mut f = Files::new();
    match v {
        Value::Null => {}
        Value::String(raw) => {
            f.insert(file.into(), raw.clone().into_bytes());
        }
        save => {
            f.insert(file.into(), save.to_string().into_bytes());
        }
    }
    f
}

#[test]
fn the_cases_are_the_owners_the_copy_is_the_pinned_file() {
    let digest = format!("{:x}", Sha256::digest(FILE.as_bytes()));
    assert_eq!(
        digest, PINNED,
        "further-cases.json changed without its owner (skills lingjing/tests/further-cases.json) and the phone's copy"
    );
}

#[test]
fn every_shared_case_names_the_same_winner_both_ways_round() {
    let doc = doc();
    let r = shared_rule(&doc);
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() > 30, "the file holds the cases");
    let mut wrong = Vec::new();
    for c in cases {
        let (here, there) = (side(r.file(), &c["here"]), side(r.file(), &c["there"]));
        let winner = c["winner"].as_str().unwrap();
        let got = (r.further(&here, &there), r.further(&there, &here));
        if got != (winner == "here", winner == "there") {
            wrong.push(format!(
                "{} (expected {winner}, got here={} there={}): {}",
                c["name"], got.0, got.1, c["why"]
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ:\n{}",
        wrong.len(),
        cases.len(),
        wrong.join("\n")
    );
}
