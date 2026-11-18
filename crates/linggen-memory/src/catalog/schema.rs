//! A folded tool's input schema, generated from its branches.
//!
//! One flat object: `op` (an enum of the branches the view can call, its
//! description the short index of each branch's fields) beside the union of
//! the branches' fields. Not a top-level `oneOf`: model APIs refuse one in a
//! tool's input schema, and the server checks each call strictly against
//! its own branch anyway (`check`). A field two branches define differently
//! is published once, its enums joined and its description naming each
//! branch's meaning.

use super::{Branch, Catalog, ToolDoc, View};
use serde_json::{json, Map, Value};

pub(super) fn tool_def<D: Copy + 'static>(
    cat: &Catalog<D>,
    view: View,
    doc: &ToolDoc,
) -> Option<Value> {
    let visible = cat.visible(view, doc.name);
    if visible.is_empty() {
        return None;
    }
    let multi = cat.branches_of(doc.name).count() > 1;
    let schema = if multi {
        folded_schema(doc, &visible)
    } else {
        single_schema(visible[0])
    };
    Some(json!({
        "name": doc.name,
        "description": doc.description,
        "inputSchema": schema,
    }))
}

fn single_schema<D: Copy + 'static>(b: &Branch<D>) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": Value::Object(b.properties()),
        "additionalProperties": false,
    });
    let required = b.required();
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

fn folded_schema<D: Copy + 'static>(doc: &ToolDoc, branches: &[&Branch<D>]) -> Value {
    let ops: Vec<&str> = branches.iter().map(|b| b.op).collect();
    let mut index: Vec<String> = branches.iter().map(|b| form(b)).collect();
    if let Some(op) = doc.implicit_op {
        index.insert(0, format!("omit op for {op}"));
    }
    let mut props = Map::new();
    props.insert(
        "op".into(),
        json!({"type": "string", "enum": ops, "description": index.join(" · ")}),
    );
    let mut owner: std::collections::HashMap<String, &str> = Default::default();
    for b in branches {
        for (key, field) in b.properties() {
            let first = *owner.entry(key.clone()).or_insert(b.op);
            merge(&mut props, (first, b.op), key, field);
        }
    }
    let mut schema = json!({
        "type": "object",
        "properties": Value::Object(props),
        "additionalProperties": false,
    });
    if doc.implicit_op.is_none() {
        schema["required"] = json!(["op"]);
    }
    schema
}

/// `op: field, optional?` — a branch's fields as the caller writes them.
pub(super) fn form<D: Copy + 'static>(b: &Branch<D>) -> String {
    let required = b.required();
    let fields: Vec<String> = b
        .properties()
        .keys()
        .map(|k| {
            if required.contains(k) {
                k.clone()
            } else {
                format!("{k}?")
            }
        })
        .collect();
    if fields.is_empty() {
        format!("{}: no fields", b.op)
    } else {
        format!("{}: {}", b.op, fields.join(", "))
    }
}

/// One field into the union. The same definition twice is kept once; two
/// definitions become one whose enum is both and whose text names each op.
fn merge(props: &mut Map<String, Value>, ops: (&str, &str), key: String, field: Value) {
    let (first, op) = ops;
    let Some(existing) = props.get_mut(&key) else {
        props.insert(key, field);
        return;
    };
    if *existing == field {
        return;
    }
    if existing.get("type") != field.get("type") {
        if let Some(obj) = existing.as_object_mut() {
            obj.remove("type");
        }
    }
    if let (Some(a), Some(b)) = (
        existing.get("enum").and_then(Value::as_array).cloned(),
        field.get("enum").and_then(Value::as_array),
    ) {
        let mut joined = a;
        for v in b {
            if !joined.contains(v) {
                joined.push(v.clone());
            }
        }
        existing["enum"] = Value::Array(joined);
    } else if let Some(obj) = existing.as_object_mut() {
        obj.remove("enum");
    }
    if let Some(text) = field.get("description").and_then(Value::as_str) {
        let before = existing
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let tagged = format!("op={first}: ");
        let before = if before.is_empty() || before.starts_with(&tagged) {
            before
        } else {
            format!("{tagged}{before}")
        };
        let joined = if before.is_empty() {
            format!("op={op}: {text}")
        } else {
            format!("{before} · op={op}: {text}")
        };
        existing["description"] = Value::String(joined);
    }
}
