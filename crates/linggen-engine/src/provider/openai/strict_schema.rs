//! OpenAI strict-mode tool schemas: when a tool's args schema can be sent
//! with `strict: true`, and the conversion that makes it compliant.
//! Provider-side — the engine hands over plain JSON Schema.

use serde_json::{json, Value};
use std::collections::HashSet;

/// Convert a tool's args_schema into OpenAI-strict-mode shape:
/// - every property in `properties` is listed in `required[]`
/// - properties that were not in the author's original `required` get
///   their `type` widened to a nullable union (`["X", "null"]`), and any
///   `enum` list gets `null` appended — so the model can opt out with
///   `null` instead of being forced to invent an enum value
/// - `additionalProperties: false`
/// - recurses into nested object schemas and array `items` so the same
///   rules apply at every level (OpenAI requires deep compliance)
///
/// Authors keep writing simple JSON Schema with a short `required[]` —
/// this helper does the boilerplate conversion right before the wire
/// hop. Non-OpenAI providers (Anthropic, Gemini, Ollama) don't go
/// through this path; they read the original optional-style schema
/// where their models behave correctly.
pub fn strictify_for_openai(schema: Value) -> Value {
    let Value::Object(mut obj) = schema else {
        return schema;
    };

    // Recurse into nested object schemas + array item schemas before
    // rewriting this level, so nested constraints land first.
    if let Some(items) = obj.get_mut("items") {
        let taken = std::mem::take(items);
        *items = strictify_for_openai(taken);
    }

    let is_object = obj
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s == "object")
        .unwrap_or_else(|| obj.contains_key("properties"));
    if !is_object {
        return Value::Object(obj);
    }

    // Snapshot author's `required` set before we rewrite it.
    let original_required: HashSet<String> = obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Recurse into each property's schema, then make optional ones nullable.
    let mut all_names: Vec<String> = Vec::new();
    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        all_names = props.keys().cloned().collect();
        for name in &all_names {
            let Some(prop) = props.get_mut(name) else {
                continue;
            };
            let taken = std::mem::take(prop);
            let mut rewritten = strictify_for_openai(taken);

            if !original_required.contains(name) {
                // Widen `type` to include "null".
                if let Some(prop_obj) = rewritten.as_object_mut() {
                    match prop_obj.get_mut("type") {
                        Some(Value::String(s)) => {
                            let kept = s.clone();
                            prop_obj.insert("type".to_string(), json!([kept, "null"]));
                        }
                        Some(Value::Array(arr))
                            if !arr.iter().any(|v| v.as_str() == Some("null")) =>
                        {
                            arr.push(json!("null"));
                        }
                        _ => {}
                    }
                    if let Some(Value::Array(enum_arr)) = prop_obj.get_mut("enum") {
                        if !enum_arr.iter().any(|v| v.is_null()) {
                            enum_arr.push(Value::Null);
                        }
                    }
                }
            }
            *prop = rewritten;
        }
    }

    // Every property is now required.
    obj.insert(
        "required".to_string(),
        Value::Array(all_names.into_iter().map(Value::String).collect()),
    );

    obj.insert("additionalProperties".to_string(), Value::Bool(false));

    Value::Object(obj)
}

/// True iff `schema` can be sent to OpenAI with `strict: true` WITHOUT
/// `strictify_for_openai` having to change which fields are required —
/// i.e. every object property, at every nesting level, is already listed
/// in its `required[]`, and the schema is free of composite keywords.
///
/// Why this gate exists: OpenAI strict mode demands EVERY property be in
/// `required[]`. `strictify_for_openai` satisfies that by widening optional
/// fields to required-nullable unions. But that distorts the tool's real
/// contract — and reasoning models (e.g. gpt-5.x on the Responses API)
/// respond to "every field is required" by null-filling every optional or
/// emitting empty/degenerate calls, which trips the consecutive-empty-
/// response bail. So we only opt a tool into strict when strictify is a
/// no-op on `required` (a fully-required schema). Tools with ANY optional
/// field — or any composite schema — are sent `strict:false` with their
/// original schema, which OpenAI fully supports and lets the model omit
/// fields it isn't using.
///
/// Composite keywords are excluded too: `oneOf`/`allOf` are rejected under
/// strict, and `anyOf` would need each branch strictified (which we don't
/// do) — so any schema using them is treated as not-strict-safe.
pub fn is_fully_required(schema: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return true;
    };
    for key in ["anyOf", "oneOf", "allOf", "not", "$ref"] {
        if obj.contains_key(key) {
            return false;
        }
    }
    if let Some(items) = obj.get("items") {
        if !is_fully_required(items) {
            return false;
        }
    }
    let is_object = obj.get("type").and_then(|v| v.as_str()) == Some("object")
        || obj.contains_key("properties");
    if !is_object {
        return true;
    }
    // A free-form object — `{type: object}` with no `properties` (a data
    // tool's array of loosely-shaped items) — has no strict form: strictify
    // would close it (`additionalProperties: false`, nothing allowed) and
    // OpenAI rejects the result outright.
    let Some(props) = obj.get("properties").and_then(|v| v.as_object()) else {
        return false;
    };
    if props.is_empty() {
        return true;
    }
    let required: HashSet<&str> = obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    props
        .iter()
        .all(|(name, sub)| required.contains(name.as_str()) && is_fully_required(sub))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strictify_adds_null_to_optional_type() {
        let input = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "max_bytes": {"type": "integer"}
            },
            "required": ["path"]
        });
        let out = strictify_for_openai(input);
        assert_eq!(out["properties"]["path"]["type"], json!("string"));
        assert_eq!(
            out["properties"]["max_bytes"]["type"],
            json!(["integer", "null"])
        );
        assert_eq!(out["required"], json!(["path", "max_bytes"]));
        assert_eq!(out["additionalProperties"], json!(false));
    }

    #[test]
    fn strictify_adds_null_to_optional_enum() {
        let input = json!({
            "type": "object",
            "properties": {
                "verb": {"type": "string", "enum": ["list", "search", "get"]},
                "type": {"type": "string", "enum": ["fact", "preference"]}
            },
            "required": ["verb"]
        });
        let out = strictify_for_openai(input);
        assert_eq!(
            out["properties"]["verb"]["enum"],
            json!(["list", "search", "get"])
        );
        assert_eq!(out["properties"]["verb"]["type"], json!("string"));
        assert_eq!(out["properties"]["type"]["type"], json!(["string", "null"]));
        assert_eq!(
            out["properties"]["type"]["enum"],
            json!(["fact", "preference", null])
        );
    }

    #[test]
    fn strictify_recurses_into_array_items() {
        let input = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "title": {"type": "string"}
                        },
                        "required": ["id"]
                    }
                }
            },
            "required": []
        });
        let out = strictify_for_openai(input);
        let item_schema = &out["properties"]["items"]["items"];
        assert_eq!(item_schema["required"], json!(["id", "title"]));
        assert_eq!(
            item_schema["properties"]["title"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(item_schema["additionalProperties"], json!(false));
    }

    #[test]
    fn strictify_preserves_already_nullable_type() {
        let input = json!({
            "type": "object",
            "properties": {
                "opt": {"type": ["string", "null"]}
            },
            "required": []
        });
        let out = strictify_for_openai(input);
        assert_eq!(out["properties"]["opt"]["type"], json!(["string", "null"]));
    }

    #[test]
    fn strictify_handles_empty_object() {
        let input = json!({"type": "object"});
        let out = strictify_for_openai(input);
        assert_eq!(out["required"], json!([]));
        assert_eq!(out["additionalProperties"], json!(false));
    }

    #[test]
    fn is_fully_required_classifies_schemas() {
        // All properties required → strict-safe.
        assert!(is_fully_required(&json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"]
        })));
        // An optional property → not strict-safe.
        assert!(!is_fully_required(&json!({
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "integer"}},
            "required": ["a"]
        })));
        // No properties / no params → trivially strict-safe.
        assert!(is_fully_required(
            &json!({"type": "object", "properties": {}})
        ));
        // Nested optional inside a required object → not strict-safe.
        assert!(!is_fully_required(&json!({
            "type": "object",
            "properties": {
                "cfg": {
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                    "required": ["a"]
                }
            },
            "required": ["cfg"]
        })));
        // Required array of fully-required objects → strict-safe.
        assert!(is_fully_required(&json!({
            "type": "object",
            "properties": {
                "items": {"type": "array", "items": {
                    "type": "object",
                    "properties": {"x": {"type": "string"}},
                    "required": ["x"]
                }}
            },
            "required": ["items"]
        })));
        // A required array of free-form objects (a skill data tool's
        // `items: {type: object}`) → not strict-safe: strict would close the
        // items to nothing, and OpenAI rejects the schema (seen live with
        // gpt-5.6 on a skill's Show tool).
        assert!(!is_fully_required(&json!({
            "type": "object",
            "properties": {
                "cards": {"type": "array", "items": {"type": "object"}}
            },
            "required": ["cards"]
        })));
        // Composite keywords are never strict-safe (oneOf/allOf rejected,
        // anyOf needs per-branch strictify we don't do).
        assert!(!is_fully_required(&json!({"anyOf": [{"type": "string"}]})));
        assert!(!is_fully_required(&json!({
            "type": "object",
            "properties": {"a": {"oneOf": [{"type": "string"}]}},
            "required": ["a"]
        })));
    }
}
