//! Whose rows a call reaches — decided here, once, for every caller of
//! [`Memory::call`](crate::Memory::call).
//!
//! The store's owner names whose rows they mean in the args (an account, or
//! every account); a person reaches only their own. A person's call runs only
//! a verb the table keeps per person (`ops::verbs`, [`Reach::Person`]), with a
//! body the stamp can mark, and the stamp then puts the person on it: `add`
//! and `add_batch` mark the rows they write, every other verb is scoped to
//! the person's rows. Whatever the args said about accounts comes off first.

use crate::error::MemError;
use crate::ops::verbs::{self, Reach};
use serde_json::{json, Map, Value};

/// Who a call is for. Every call names one; there is no default.
#[derive(Debug, Clone, PartialEq)]
pub enum Who {
    /// The store's owner: the args go as given.
    Owner,
    /// One person who is not the owner: their own rows, and only those.
    Person(Person),
}

/// A person as the store names them: the id rows are kept under, and the
/// name a row they write is labelled with.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: Option<String>,
}

impl Who {
    /// Fit `args` for `verb` to this caller, or refuse.
    pub fn scope(&self, verb: &str, args: &mut Value) -> Result<(), MemError> {
        let Who::Person(person) = self else {
            return Ok(());
        };
        if reach(verb) != Some(Reach::Person) {
            return Err(MemError::forbidden(format!(
                "{verb} is not kept per person, so only the Mac's owner may run it"
            )));
        }
        if !stampable(verb, args) {
            return Err(MemError::bad_request(format!(
                "{verb} takes a JSON object (and add_batch a list of them in facts)"
            )));
        }
        stamp(verb, args, Some(person));
        Ok(())
    }
}

fn reach(verb: &str) -> Option<Reach> {
    verbs::find(verb).map(|v| v.reach)
}

/// May only this machine's owner run `verb` — not a phone, even the
/// owner's? (A verb that crosses account lines by design.)
pub fn machine_only(verb: &str) -> bool {
    reach(verb) == Some(Reach::ThisMachine)
}

/// Can the stamp mark this body? It must be an object, and every element of
/// `add_batch`'s `facts` an object too — anything else would reach the store
/// unmarked, which it reads as the owner.
fn stampable(verb: &str, args: &Value) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    if verb != "add_batch" {
        return true;
    }
    obj.get("facts")
        .and_then(Value::as_array)
        .is_some_and(|facts| facts.iter().all(Value::is_object))
}

/// What a body may say about accounts; the stamp takes it all off.
const CLAIMS: &[&str] = &["account", "account_id", "account_name", "all_accounts"];

fn strip(obj: &mut Map<String, Value>) {
    for k in CLAIMS {
        obj.remove(*k);
    }
}

fn mark(obj: &mut Map<String, Value>, person: Option<&Person>) {
    let Some(p) = person else {
        return;
    };
    obj.insert("account_id".into(), json!(p.id));
    obj.insert("account_name".into(), json!(p.name));
}

/// Take every account claim off `args` and put `person` on instead —
/// `None` puts nothing on, which the store reads as the owner (how a host
/// sends the owner's own phone, whose claims count for nothing either).
pub fn stamp(verb: &str, args: &mut Value, person: Option<&Person>) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    strip(obj);
    match verb {
        "add" => mark(obj, person),
        "add_batch" => stamp_facts(obj, person),
        _ => {
            if let Some(p) = person {
                obj.insert("account".into(), json!(p.id));
            }
        }
    }
}

fn stamp_facts(obj: &mut Map<String, Value>, person: Option<&Person>) {
    let Some(facts) = obj.get_mut("facts").and_then(Value::as_array_mut) else {
        return;
    };
    for fo in facts.iter_mut().filter_map(Value::as_object_mut) {
        strip(fo);
        mark(fo, person);
    }
}

#[cfg(test)]
#[path = "who_tests.rs"]
mod tests;
