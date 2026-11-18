//! A folded call, resolved and checked before anything runs: the branch its
//! `op` names (or the tool's implicit one), the arguments repaired where a
//! model typed a scalar as a string, then refused if a field is unknown,
//! belongs to another branch, is missing, or has a value its schema does
//! not allow. Every refusal says what was wrong and the form that works;
//! nothing is guessed — not an `op`, a date or a permission.

use super::schema::form;
use super::{Branch, Catalog, View};
use serde_json::{json, Map, Value};

/// What a folded call resolved to.
pub enum Resolved<'a, D: Copy + 'static> {
    /// No such tool for this view.
    Unknown,
    /// Refused before anything ran; the text is for the caller.
    Refused(String),
    /// Run `branch` with `args` (`op` removed, fixed arguments added).
    Call { branch: &'a Branch<D>, args: Value },
}

pub(super) fn resolve<'a, D: Copy + 'static>(
    cat: &'a Catalog<D>,
    view: View,
    name: &str,
    args: Value,
) -> Resolved<'a, D> {
    let Some(doc) = cat.tools().iter().find(|d| d.name == name) else {
        return Resolved::Unknown;
    };
    let visible = cat.visible(view, name);
    if visible.is_empty() {
        return Resolved::Unknown;
    }
    let mut obj = match args {
        Value::Object(obj) => obj,
        Value::Null => Map::new(),
        _ => return Resolved::Refused(format!("{name}: arguments must be an object")),
    };
    obj.retain(|_, v| !v.is_null());
    let multi = cat.branches_of(name).count() > 1;
    let branch = if multi {
        match pick(name, doc.implicit_op, &visible, &mut obj) {
            Ok(b) => b,
            Err(why) => return Resolved::Refused(why),
        }
    } else if obj.contains_key("op") {
        return Resolved::Refused(format!("{name} takes no op"));
    } else {
        visible[0]
    };
    match check(name, branch, &visible, obj) {
        Ok(args) => Resolved::Call { branch, args },
        Err(why) => Resolved::Refused(why),
    }
}

/// The branch a multi-branch call names.
fn pick<'a, D: Copy + 'static>(
    name: &str,
    implicit: Option<&str>,
    visible: &[&'a Branch<D>],
    obj: &mut Map<String, Value>,
) -> Result<&'a Branch<D>, String> {
    let ops = || {
        visible
            .iter()
            .map(|b| b.op)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match obj.remove("op") {
        Some(Value::String(op)) => visible
            .iter()
            .copied()
            .find(|b| b.op == op)
            .ok_or_else(|| format!("{name}: op \"{op}\" is not one of: {}", ops())),
        Some(_) => Err(format!("{name}: op must be a string, one of: {}", ops())),
        None => implied(name, implicit, visible, obj),
    }
}

/// A call without `op`: the tool's implicit branch when the fields fit it,
/// else a refusal naming the branch the fields do fit.
fn implied<'a, D: Copy + 'static>(
    name: &str,
    implicit: Option<&str>,
    visible: &[&'a Branch<D>],
    obj: &Map<String, Value>,
) -> Result<&'a Branch<D>, String> {
    let fits = |b: &Branch<D>| obj.keys().all(|k| b.accepts(k));
    if let Some(b) = implicit.and_then(|op| visible.iter().copied().find(|b| b.op == op)) {
        let complete = b.required().iter().all(|k| obj.contains_key(k));
        if fits(b) && complete {
            return Ok(b);
        }
    }
    let fitting: Vec<&&Branch<D>> = visible
        .iter()
        .filter(|b| !obj.is_empty() && Some(b.op) != implicit && fits(b))
        .collect();
    if let Some(b) = fitting.first() {
        return Err(format!(
            "{name} needs op for these fields: pass op=\"{}\" ({})",
            b.op,
            form(b)
        ));
    }
    let forms: Vec<String> = visible.iter().map(|b| form(b)).collect();
    Err(format!("{name} needs op — {}", forms.join(" · ")))
}

/// The branch's own rules over `obj`; the arguments it runs with.
fn check<D: Copy + 'static>(
    name: &str,
    b: &Branch<D>,
    siblings: &[&Branch<D>],
    mut obj: Map<String, Value>,
) -> Result<Value, String> {
    let props = b.properties();
    let required = b.required();
    let at = || {
        if siblings.len() > 1 {
            format!("{name} op={}", b.op)
        } else {
            name.to_string()
        }
    };
    // A soft-empty optional field ("" or []) is a field not given.
    obj.retain(|k, v| required.contains(k) || !soft_empty(v));
    for key in obj.keys() {
        if b.accepts(key) {
            continue;
        }
        let owners: Vec<&str> = siblings
            .iter()
            .filter(|s| s.op != b.op && s.accepts(key))
            .map(|s| s.op)
            .collect();
        return Err(if owners.is_empty() {
            format!("{}: unknown field `{key}` ({})", at(), form(b))
        } else {
            format!(
                "{}: `{key}` belongs to op={}, not op={} ({})",
                at(),
                owners.join("/"),
                b.op,
                form(b)
            )
        });
    }
    for (key, value) in obj.iter_mut() {
        match props.get(key) {
            Some(field) => repair(field, value),
            None => repair(&json!({"type": "boolean"}), value),
        }
    }
    if let Some(missing) = required.iter().find(|k| !obj.contains_key(*k)) {
        return Err(format!("{} needs `{missing}` ({})", at(), form(b)));
    }
    for (key, value) in &obj {
        if let Some(field) = props.get(key) {
            valid(field, value).map_err(|why| format!("{}: `{key}` {why}", at()))?;
        }
    }
    if let Some(rule) = b.check {
        rule(&obj).map_err(|why| format!("{}: {why}", at()))?;
    }
    for (key, value) in b.fixed {
        obj.insert((*key).to_string(), Value::String((*value).to_string()));
    }
    Ok(Value::Object(obj))
}

fn soft_empty(v: &Value) -> bool {
    match v {
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// A scalar the model typed as a string, as the field's declared type:
/// `"true"` on a boolean, `"5"` on an integer, a lone string on an array.
/// What does not convert cleanly is left for [`valid`] to name.
pub(crate) fn repair(field: &Value, value: &mut Value) {
    let Some(kind) = field.get("type").and_then(Value::as_str) else {
        return;
    };
    if let Some(fixed) = coerce_value(kind, value) {
        *value = fixed;
    }
}

pub(crate) fn coerce_value(kind: &str, value: &Value) -> Option<Value> {
    let s = value.as_str()?.trim();
    match kind {
        "boolean" => match s.to_ascii_lowercase().as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        "integer" => s.parse::<i64>().ok().map(Value::from),
        "number" => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        "array" => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Array(_)) => Some(v),
            _ if s.is_empty() => None,
            _ => Some(json!([s])),
        },
        _ => None,
    }
}

/// Does `value` satisfy `field` (the JSON Schema subset the catalogue
/// writes: type, enum, items, min/max items, minimum/maximum, nested
/// object properties)? `Err` says what is wrong.
fn valid(field: &Value, value: &Value) -> Result<(), String> {
    if let Some(kind) = field.get("type").and_then(Value::as_str) {
        if !is_type(kind, value) {
            return Err(format!("must be {}", article(kind)));
        }
    }
    if let Some(choices) = field.get("enum").and_then(Value::as_array) {
        if !choices.contains(value) {
            let shown: Vec<String> = choices.iter().map(Value::to_string).collect();
            return Err(format!("must be one of {}", shown.join(", ")));
        }
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = field.get("minimum").and_then(Value::as_f64) {
            if n < min {
                return Err(format!("must be at least {min}"));
            }
        }
        if let Some(max) = field.get("maximum").and_then(Value::as_f64) {
            if n > max {
                return Err(format!("must be at most {max}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        let len = items.len() as u64;
        if let Some(min) = field.get("minItems").and_then(Value::as_u64) {
            if len < min {
                return Err(format!("needs at least {min} items"));
            }
        }
        if let Some(max) = field.get("maxItems").and_then(Value::as_u64) {
            if len > max {
                return Err(format!("takes at most {max} items"));
            }
        }
        if let Some(each) = field.get("items") {
            for item in items {
                valid(each, item).map_err(|why| format!("item {why}"))?;
            }
        }
    }
    if let (Some(obj), Some(props)) = (
        value.as_object(),
        field.get("properties").and_then(Value::as_object),
    ) {
        for (key, inner) in obj {
            let Some(spec) = props.get(key) else {
                let known: Vec<&String> = props.keys().collect();
                return Err(format!("has unknown field `{key}` (takes {known:?})"));
            };
            valid(spec, inner).map_err(|why| format!(".{key} {why}"))?;
        }
    }
    Ok(())
}

fn is_type(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

fn article(kind: &str) -> String {
    match kind {
        "integer" | "array" | "object" => format!("an {kind}"),
        _ => format!("a {kind}"),
    }
}
