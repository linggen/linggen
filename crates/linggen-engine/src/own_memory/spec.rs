//! The tool's words and fields, written once: its description and the
//! fields each op takes. A Linggen session's built-in `memory_notes`
//! (`engine::tools::builtin::memory_notes`) and the MCP door's (`mcp::catalog`)
//! are both made from this, so the two cannot say different things
//! (`tests::the_door_and_a_session_say_the_same_about_the_tool`).

use serde_json::{json, Map, Value};

/// The tool's ops. Closed: a new one is a new variant, and the compiler
/// finds every place that answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Index,
    Read,
    Search,
    /// Only the titles that match words; for a host's per-turn hook.
    Titles,
    Write,
}

impl Op {
    pub const ALL: [Op; 5] = [Op::Index, Op::Read, Op::Search, Op::Titles, Op::Write];

    pub fn word(self) -> &'static str {
        match self {
            Op::Index => "index",
            Op::Read => "read",
            Op::Search => "search",
            Op::Titles => "titles",
            Op::Write => "write",
        }
    }

    pub fn parse(word: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.word() == word.trim())
    }
}

pub const DESCRIPTION: &str = "Linggen's own notes about the person and the work. \
op=index lists the notes for a project; op=read opens one note (id) or a topic file (path); \
op=search finds notes by words, any language, and what other agents remember; \
op=titles lists only the summaries (with ids) whose words match, and counts no use; \
op=write adds a note (scope, topic, summary, kind, body) or changes one by id (action update, void, no_raise, yearly). \
Write only a fact the person asked to remember, a decision or rule with its why, or a gotcha — never what was done or where work stands. \
A note in the person's own voice changes only after you show it and they agree (user_directed true). \
Notes about the body, mind and people close to the person, life moments, and the chronicle (what the person did, month by month) are Yinyue's alone.";

/// The fields `op` takes, with their definitions.
pub fn fields(op: Op) -> Vec<(&'static str, Value)> {
    let names: &[&'static str] = match op {
        Op::Index => &["cwd"],
        Op::Read => &["id", "path"],
        Op::Search => &["words", "limit"],
        Op::Titles => &["words", "limit", "exclude"],
        Op::Write => &["action", "scope", "topic", "summary", "kind", "voice", "body", "id", "reason", "on", "user_directed", "base", "day"],
    };
    names.iter().map(|name| (*name, definition(name))).collect()
}

fn definition(name: &str) -> Value {
    match name {
        "cwd" => json!({"type": "string", "description": "The project folder; its notes come first"}),
        "id" => json!({"type": "string", "description": "A note's id, from the index"}),
        "path" => json!({"type": "string", "description": "A topic file inside the store, e.g. person/diet.md"}),
        "words" => json!({"type": "string", "description": "Words to find (any language, any case); a note with more of them ranks higher"}),
        "exclude" => json!({"type": "array", "items": {"type": "string"}, "description": "Note ids to leave out (already shown)"}),
        "limit" => json!({"type": "integer", "description": "Most hits (default 10, at most 30)"}),
        "action" => json!({"type": "string", "enum": ["add", "update", "void", "no_raise", "yearly"], "description": "Default add (update when an id is given)"}),
        "scope" => json!({"type": "string", "description": "add: person | area:<body|mind|people|music|money|play|home> | project:<absolute path> | moments | chronicle"}),
        "topic" => json!({"type": "string", "description": "add: the topic file's name; in the chronicle a month (2026-11) or a year (2026)"}),
        "summary" => json!({"type": "string", "description": "One line, at most 80 characters"}),
        "kind" => json!({"type": "string", "enum": ["person", "rule", "decision", "gotcha", "moment", "chronicle"]}),
        "voice" => json!({"type": "string", "enum": ["user", "agent"], "description": "add: user for the person's own words"}),
        "body" => json!({"type": "string", "description": "What, and for a rule, decision or gotcha the why"}),
        "reason" => json!({"type": "string", "description": "void: why the note is wrong"}),
        "on" => json!({"type": "boolean", "description": "no_raise: true (default) = never bring it up unasked"}),
        "user_directed" => json!({"type": "boolean", "description": "true only after the person agreed to change a note in their own voice"}),
        "day" => json!({"type": "string", "description": "yearly: the day the note repeats every year, MM-DD or YYYY-MM-DD (a birthday, an anniversary); empty clears it"}),
        "base" => json!({"type": "string", "description": "The version of the topic file you read"}),
        other => panic!("memory_notes has no field {other}"),
    }
}

/// The fields of one op as a JSON object's `properties`.
pub fn properties(op: Op) -> Value {
    Value::Object(fields(op).into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Map<_, _>>())
}

/// A session's schema for the whole tool: `op` and every op's fields.
pub fn session_schema() -> Value {
    let mut props = Map::new();
    let words: Vec<&str> = Op::ALL.iter().map(|op| op.word()).collect();
    props.insert("op".into(), json!({"type": "string", "enum": words}));
    for op in Op::ALL {
        for (name, definition) in fields(op) {
            props.entry(name).or_insert(definition);
        }
    }
    json!({"type": "object", "properties": Value::Object(props), "required": ["op"]})
}
