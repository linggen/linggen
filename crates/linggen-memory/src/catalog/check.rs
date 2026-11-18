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
    let Some((implicit, visible)) = tool_of(cat, view, name) else {
        return Resolved::Unknown;
    };
    let mut obj = match args {
        Value::Object(obj) => obj,
        Value::Null => Map::new(),
        _ => return Resolved::Refused(format!("{name}: arguments must be an object")),
    };
    obj.retain(|_, v| !v.is_null());
    let branch = match choose(cat, name, implicit, &visible, &mut obj) {
        Ok(b) => b,
        Err(why) => return Resolved::Refused(why),
    };
    match check(name, branch, &visible, obj) {
        Ok(args) => Resolved::Call { branch, args },
        Err(why) => Resolved::Refused(why),
    }
}

/// The branch a call names, chosen as [`resolve`] chooses it, its fields
/// not checked.
pub(super) fn branch_of<'a, D: Copy + 'static>(
    cat: &'a Catalog<D>,
    view: View,
    name: &str,
    args: &Value,
) -> Option<&'a Branch<D>> {
    let (implicit, visible) = tool_of(cat, view, name)?;
    let mut obj = match args {
        Value::Object(obj) => obj.clone(),
        Value::Null => Map::new(),
        _ => return None,
    };
    obj.retain(|_, v| !v.is_null());
    choose(cat, name, implicit, &visible, &mut obj).ok()
}

/// A folded tool's implicit `op` and the branches `view` has of it.
fn tool_of<'a, D: Copy + 'static>(
    cat: &'a Catalog<D>,
    view: View,
    name: &str,
) -> Option<(Option<&'static str>, Vec<&'a Branch<D>>)> {
    let doc = cat.tools().iter().find(|d| d.name == name)?;
    let visible = cat.visible(view, name);
    (!visible.is_empty()).then_some((doc.implicit_op, visible))
}

/// The branch of a call's `obj` (its `op` taken out of it).
fn choose<'a, D: Copy + 'static>(
    cat: &Catalog<D>,
    name: &str,
    implicit: Option<&str>,
    visible: &[&'a Branch<D>],
    obj: &mut Map<String, Value>,
) -> Result<&'a Branch<D>, String> {
    if cat.branches_of(name).count() > 1 {
        return pick(name, implicit, visible, obj);
    }
    if obj.contains_key("op") {
        return Err(format!("{name} takes no op"));
    }
    Ok(visible[0])
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
    // A soft-empty field is a field not given (`check` drops it), so it
    // decides nothing about the branch.
    let given: Vec<&String> = obj.iter().filter(|(_, v)| !soft_empty(v)).map(|(k, _)| k).collect();
    let fits = |b: &Branch<D>| given.iter().all(|k| b.accepts(k));
    if let Some(b) = implicit.and_then(|op| visible.iter().copied().find(|b| b.op == op)) {
        let complete = b.required().iter().all(|k| obj.contains_key(k));
        if fits(b) && complete {
            return Ok(b);
        }
    }
    let fitting: Vec<&&Branch<D>> = visible
        .iter()
        .filter(|b| !given.is_empty() && Some(b.op) != implicit && fits(b))
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

#[cfg(test)]
mod tests {
    use super::super::memory::catalog;
    use super::super::View;
    use super::*;

    /// The branch a call resolves to and the arguments it runs with.
    fn call(tool: &str, args: Value) -> (&'static str, Value) {
        match catalog().resolve(View::Local, tool, args.clone()) {
            Resolved::Call { branch, args } => (branch.op, args),
            Resolved::Refused(why) => panic!("{tool} {args}: refused: {why}"),
            Resolved::Unknown => panic!("{tool}: unknown"),
        }
    }

    fn refused(tool: &str, args: Value) -> String {
        match catalog().resolve(View::Local, tool, args.clone()) {
            Resolved::Refused(why) => why,
            Resolved::Call { branch, .. } => panic!("{tool} {args}: ran as {}", branch.op),
            Resolved::Unknown => panic!("{tool}: unknown"),
        }
    }

    /// The commonest call needs no op; get and list name theirs, and an
    /// id or list filters without one are refused with the form that works.
    #[test]
    fn a_query_alone_is_a_semantic_search() {
        assert_eq!(call("memory_search", json!({"query": "tea"})).0, "semantic");
        let (op, args) = call("memory_search", json!({"query": "tea", "limit": "3"}));
        assert_eq!((op, args), ("semantic", json!({"query": "tea", "limit": 3})));
        assert_eq!(
            call("memory_search", json!({"op": "semantic", "query": "tea"})).0,
            "semantic"
        );
        assert_eq!(call("memory_search", json!({"op": "get", "id": "a"})).0, "get");
        assert_eq!(call("memory_search", json!({"op": "list"})).0, "list");

        let why = refused("memory_search", json!({"id": "a"}));
        assert!(why.contains("op=\"get\""), "{why}");
        let why = refused("memory_search", json!({"tier": "core", "sort": "oldest"}));
        assert!(why.contains("op=\"list\""), "{why}");
        let why = refused("memory_search", json!({}));
        assert!(why.contains("needs op") && why.contains("semantic: query"), "{why}");
    }

    /// The branch a client fills fields in for is the one that will run:
    /// picked as a call is resolved, implicit op included; a call that
    /// names no branch has none.
    #[test]
    fn the_branch_of_a_call_is_the_one_that_runs() {
        let op = |tool: &str, args: Value| catalog().branch_of(View::Local, tool, &args).map(|b| b.op);
        assert_eq!(op("memory_search", json!({"query": "tea"})), Some("semantic"));
        assert_eq!(op("memory_search", json!({"op": "get", "id": "a"})), Some("get"));
        assert_eq!(op("memory_search", json!({"op": "list", "day": "2026-11-01"})), Some("list"));
        assert_eq!(op("memory_search", json!({"id": "a"})), None, "no op, not a search");
        assert_eq!(op("memory_add", json!({"content": "x"})), Some("add"));
        assert_eq!(op("memory_add", json!({"content": "x", "op": "add"})), None);
        assert_eq!(op("memory_upkeep", json!({"op": "days"})), Some("days"));
        assert_eq!(op("memory_upkeep", json!({"op": "dream_run"})), None, "not memory's");
        assert_eq!(op("memory_get", json!({"id": "a"})), None, "a retired name is no tool");
        let get = catalog().branch_of(View::Local, "memory_search", &json!({"op": "get"})).unwrap();
        assert!(!get.advertises("scope_root") && get.advertises("id"));
        let semantic = catalog().branch_of(View::Local, "memory_search", &json!({})).map(|b| b.op);
        assert_eq!(semantic, None, "nothing given fits no branch");
        let search = catalog().branch_of(View::Local, "memory_search", &json!({"query": "q"})).unwrap();
        assert!(search.accepts("from") && !search.advertises("from"), "a hidden filter is accepted, never shown");
        // Auto-recall's exact shapes, the app's one-type search included.
        for recall in [
            json!({"query": "q", "limit": 20, "min_score": 0.3, "scope_root": "/w"}),
            json!({"query": "q", "limit": 20, "scope_root": "/w", "type": "fact"}),
        ] {
            assert_eq!(call("memory_search", recall).0, "semantic");
        }
    }

    /// Unknown ops, fields of another branch, missing and bad values are
    /// refused — never ignored, never guessed.
    #[test]
    fn a_bad_search_is_refused_with_the_form_that_works() {
        let why = refused("memory_search", json!({"op": "fly"}));
        assert!(why.contains("\"fly\" is not one of: semantic, list, get"), "{why}");
        let why = refused("memory_search", json!({"op": 3}));
        assert!(why.contains("op must be a string"), "{why}");
        let why = refused("memory_search", json!({"op": "get"}));
        assert!(why.contains("needs `id`") && why.contains("get: id"), "{why}");
        let why = refused("memory_search", json!({"op": "get", "id": "a", "query": "q"}));
        assert!(why.contains("`query` belongs to op=semantic, not op=get"), "{why}");
        let why = refused("memory_search", json!({"op": "list", "bogus": 1}));
        assert!(why.contains("unknown field `bogus`"), "{why}");
        let why = refused("memory_search", json!({"op": "list", "tier": "bogus"}));
        assert!(why.contains("`tier` must be one of"), "{why}");
        let why = refused("memory_search", json!({"op": "list", "limit": -1}));
        assert!(why.contains("`limit` must not be negative"), "{why}");
        let why = refused("memory_search", json!({"op": "list", "unjudged": true}));
        assert!(why.contains("`unjudged` needs `day`"), "{why}");
        let why = refused("memory_search", json!({"op": "list", "past_ttl": true, "tier": "core"}));
        assert!(why.contains("`past_ttl` reads episodic rows"), "{why}");
        let why = refused("memory_search", json!({"query": 5}));
        assert!(why.contains("`query` must be a string"), "{why}");
        let why = refused("memory_search", json!("tea"));
        assert!(why.contains("arguments must be an object"), "{why}");
    }

    /// The scalar repairs and soft-empty clean-up memory's tools always
    /// had, on the branch's own schema.
    #[test]
    fn a_models_sloppy_types_are_repaired_per_branch() {
        let (_, args) = call(
            "memory_search",
            json!({"op": "list", "types": "fact", "indexed": "true", "offset": "10", "day": "", "tier": null}),
        );
        assert_eq!(args, json!({"types": ["fact"], "indexed": true, "offset": 10}));
        let (_, args) = call(
            "memory_add",
            json!({"content": "x", "indexed": "false", "replace_ids": "abc", "skip_dedup": "true", "scope": ""}),
        );
        assert_eq!(
            args,
            json!({"content": "x", "indexed": false, "replace_ids": ["abc"], "skip_dedup": true})
        );
        // The required field is kept even when empty: the store names it.
        let (_, args) = call("memory_add", json!({"content": ""}));
        assert_eq!(args, json!({"content": ""}));
    }

    /// A one-branch tool takes no op; its hidden programmatic fields and the
    /// folded-only `occurred_at` are taken, anything else is refused.
    #[test]
    fn the_single_memory_tools_are_strict() {
        let (op, _) = call(
            "memory_add",
            json!({"content": "x", "occurred_at": "2026-11-01T12:00:00Z", "force": true}),
        );
        assert_eq!(op, "add");
        let why = refused("memory_add", json!({"content": "x", "op": "add"}));
        assert!(why.contains("memory_add takes no op"), "{why}");
        let why = refused("memory_add", json!({}));
        assert!(why.contains("memory_add needs `content`"), "{why}");
        let why = refused("memory_add", json!({"content": "x", "colour": ["a"]}));
        assert!(why.contains("unknown field `colour`"), "{why}");
        let why = refused("memory_add", json!({"content": "x", "from": "someone"}));
        assert!(why.contains("`from` must be one of"), "{why}");
        assert_eq!(call("memory_update", json!({"id": "a", "summary": "s"})).0, "update");
        let why = refused("memory_update", json!({"summary": "s"}));
        assert!(why.contains("needs `id`"), "{why}");
        assert_eq!(call("memory_delete", json!({"id": "a"})).0, "delete");
        let why = refused("memory_delete", json!({"id": "a", "replace_ids": ["b"]}));
        assert!(why.contains("unknown field `replace_ids`"), "{why}");
    }

    /// Every upkeep op with its minimal arguments, then its bad values and
    /// a field of a sibling op.
    #[test]
    fn every_upkeep_op_takes_its_minimal_arguments_and_refuses_the_rest() {
        let minimal = [
            ("session_start", json!({})),
            ("harvest_day", json!({"date": "2026-11-01"})),
            ("days", json!({})),
            ("remember_day", json!({"date": "2026-11-01"})),
            ("dream_lease", json!({"day": "2026-11-01", "holder": "h"})),
            ("role", json!({})),
            ("sweep", json!({})),
            ("issues", json!({})),
            ("issue_add", json!({"kind": "chain", "note": "n"})),
            ("issue_resolve", json!({"id": "i", "outcome": "resolved"})),
            ("chains", json!({})),
        ];
        for (op, args) in minimal {
            let mut sent = args.clone();
            sent["op"] = json!(op);
            assert_eq!(call("memory_upkeep", sent), (op, args), "{op}");
        }
        let bad = [
            (json!({"op": "harvest_day"}), "needs `date`"),
            (json!({"op": "remember_day", "date": "d", "judged": -1}), "`judged` must not be negative"),
            (json!({"op": "dream_lease", "day": "d", "holder": "h", "action": "steal"}), "`action` must be one of"),
            (json!({"op": "issues", "status": "closed"}), "`status` must be one of"),
            (json!({"op": "issues", "limit": -5}), "must not be negative"),
            (json!({"op": "issue_add", "kind": "rumour", "note": "n"}), "`kind` must be one of"),
            (json!({"op": "issue_add", "kind": "chain"}), "needs `note`"),
            (json!({"op": "issue_resolve", "id": "i", "outcome": "maybe"}), "`outcome` must be one of"),
            (json!({"op": "chains", "kind": "chain"}), "`kind` must be one of"),
            (json!({"op": "sweep", "date": "2026-11-01"}), "`date` belongs to op=harvest_day/remember_day"),
            (json!({"op": "role", "dry_run": true}), "`dry_run` belongs to op=sweep"),
            (json!({"date": "2026-11-01"}), "needs op"),
        ];
        for (args, want) in bad {
            let why = refused("memory_upkeep", args.clone());
            assert!(why.contains(want), "{args}: {why}");
        }
    }

    /// The folded schemas: flat, strict, `op` an enum of the view's
    /// branches with each branch's form, and a field two branches define
    /// differently published with both meanings.
    #[test]
    fn the_folded_schemas_come_from_the_branches() {
        let tools = catalog().tools_list(View::Local);
        let search = &tools[0]["inputSchema"];
        assert_eq!(search["additionalProperties"], json!(false));
        assert!(search.get("required").is_none(), "op is optional on memory_search");
        assert_eq!(search["properties"]["op"]["enum"], json!(["semantic", "list", "get"]));
        let index = search["properties"]["op"]["description"].as_str().unwrap();
        assert!(index.starts_with("omit op for semantic"), "{index}");
        assert!(index.contains("get: id"), "{index}");
        let add = &tools[1]["inputSchema"];
        assert_eq!(add["required"], json!(["content"]));
        assert!(add["properties"].get("op").is_none());
        assert!(add["properties"].get("occurred_at").is_some());
        assert!(add["properties"].get("skip_dedup").is_none(), "hidden stays hidden");
        let upkeep = &tools[4]["inputSchema"];
        assert_eq!(upkeep["required"], json!(["op"]));
        let kinds = upkeep["properties"]["kind"]["enum"].as_array().unwrap();
        for k in ["chain", "contradiction", "cited", "subject"] {
            assert!(kinds.contains(&json!(k)), "{k}");
        }
        let text = upkeep["properties"]["kind"]["description"].as_str().unwrap();
        assert!(text.contains("op=issue_add:") && text.contains("op=chains:"), "{text}");
    }
}
