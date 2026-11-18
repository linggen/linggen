//! The MCP catalogue: one table entry per branch decides a tool's name, its
//! schema, its timeout, what runs it, and who can see and call it.
//!
//! A *branch* is one operation a host can ask for. It is published two ways
//! from the same entry:
//!
//! - **folded** — as one `op` of a tool that groups several (`memory_search`
//!   with `semantic` / `get` / `list`), or as a tool of its own when its
//!   tool has one branch (`memory_add`). The folded tools are checked
//!   strictly: an unknown `op`, a field of another branch, a missing
//!   required field or a bad value is refused with the form that works.
//! - **legacy** — as the tool the branch was before the tools were folded
//!   (`memory_list`), with its schema exactly as it was published and its
//!   arguments taken as they always were.
//!
//! The shape lives in the memory crate because the engine depends on memory
//! and never the other way: memory defines its own branches here
//! ([`memory`]), the engine defines its browser, agent and dream branches
//! with its own dispatcher type `D`, and composes both into one
//! [`Catalog`]. Nothing outside a catalogue keeps a list of tool names.

mod check;
pub mod memory;
mod schema;

pub use check::Resolved;

use serde_json::{Map, Value};
use std::collections::HashSet;

/// Who is looking at the catalogue: this machine, or another computer the
/// owner approved (memory only — delegation and browser control stay with
/// this machine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Local,
    Remote,
}

/// The views a branch is visible and callable in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Views(u8);

impl Views {
    /// This machine only.
    pub const LOCAL: Views = Views(1);
    /// This machine and the computers it approved.
    pub const ALL: Views = Views(3);

    pub fn has(self, view: View) -> bool {
        let bit = match view {
            View::Local => 1,
            View::Remote => 2,
        };
        self.0 & bit != 0
    }
}

/// A cross-field rule a branch's schema cannot say (`ref` or `coordinate`,
/// not both). `Err` is the sentence the caller reads.
pub type Check = fn(&Map<String, Value>) -> Result<(), String>;

/// One operation, as the catalogue publishes it.
#[derive(Clone, Copy)]
pub struct Branch<D: Copy + 'static> {
    /// The folded tool this branch belongs to; `None` for a tool that is
    /// published only in the legacy surface (it retires with it).
    pub tool: Option<&'static str>,
    /// The branch's `op` inside its folded tool.
    pub op: &'static str,
    /// The tool this branch is published as in the legacy surface.
    pub legacy: Option<&'static str>,
    /// The legacy tool's description; the folded tool's own text is its
    /// [`ToolDoc`].
    pub description: &'static str,
    /// `{type: object, properties, required}` — the legacy tool's input
    /// schema, and the branch's fields in the folded tool.
    pub schema: fn() -> Value,
    /// Properties only the folded tool advertises (a field the legacy tool
    /// never showed).
    pub extra: Option<fn() -> Value>,
    /// Fields a programmatic caller sends that the schema does not show
    /// to a model; accepted, never advertised.
    pub hidden: &'static [&'static str],
    /// Arguments the folded branch adds before it runs (the legacy
    /// `browser_tabs` action a tab branch stands for).
    pub fixed: &'static [(&'static str, &'static str)],
    pub check: Option<Check>,
    pub timeout_ms: u64,
    pub views: Views,
    pub dispatch: D,
}

impl<D: Copy + 'static> Branch<D> {
    /// The same branch, run by another dispatcher — how the engine takes
    /// memory's branches into its own catalogue.
    pub fn with<E: Copy + 'static>(self, dispatch: E) -> Branch<E> {
        Branch {
            tool: self.tool,
            op: self.op,
            legacy: self.legacy,
            description: self.description,
            schema: self.schema,
            extra: self.extra,
            hidden: self.hidden,
            fixed: self.fixed,
            check: self.check,
            timeout_ms: self.timeout_ms,
            views: self.views,
            dispatch,
        }
    }

    /// The branch's advertised properties, folded extras included.
    pub(crate) fn properties(&self) -> Map<String, Value> {
        let mut props = (self.schema)()["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if let Some(extra) = self.extra {
            if let Some(more) = extra().as_object() {
                props.extend(more.clone());
            }
        }
        props
    }

    pub(crate) fn required(&self) -> Vec<String> {
        (self.schema)()["required"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every field the folded branch takes: advertised and hidden.
    pub(crate) fn accepts(&self, key: &str) -> bool {
        self.properties().contains_key(key) || self.hidden.contains(&key)
    }

    /// The legacy tool definition, exactly as `tools/list` published it.
    pub fn legacy_def(&self) -> Option<Value> {
        let name = self.legacy?;
        Some(serde_json::json!({
            "name": name,
            "description": self.description,
            "inputSchema": (self.schema)(),
        }))
    }
}

/// A folded tool's own name and text. Its branches are the entries naming
/// it; `implicit_op` is the branch a call without `op` means, when its
/// fields fit that branch (`memory_search` with only a `query`).
#[derive(Clone, Copy)]
pub struct ToolDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub implicit_op: Option<&'static str>,
}

/// The tools and their branches, checked once when composed.
pub struct Catalog<D: Copy + 'static> {
    tools: Vec<ToolDoc>,
    branches: Vec<Branch<D>>,
}

impl<D: Copy + 'static> Catalog<D> {
    /// Refuses a catalogue in which any name decides two things: a folded
    /// tool named twice, an `op` twice in one tool, a legacy name twice, a
    /// branch whose tool has no text, a tool with no branch, or an implicit
    /// `op` the tool does not have.
    pub fn new(tools: Vec<ToolDoc>, branches: Vec<Branch<D>>) -> Result<Self, String> {
        let mut names = HashSet::new();
        for doc in &tools {
            if !names.insert(doc.name) {
                return Err(format!("tool `{}` is named twice", doc.name));
            }
        }
        let mut ops = HashSet::new();
        let mut legacy = HashSet::new();
        for b in &branches {
            if let Some(tool) = b.tool {
                if !names.contains(tool) {
                    return Err(format!("branch `{}` names unknown tool `{tool}`", b.op));
                }
                if !ops.insert((tool, b.op)) {
                    return Err(format!("tool `{tool}` has op `{}` twice", b.op));
                }
            }
            if let Some(name) = b.legacy {
                if !legacy.insert(name) {
                    return Err(format!("legacy tool `{name}` is named twice"));
                }
            }
            if b.tool.is_none() && b.legacy.is_none() {
                return Err(format!("branch `{}` is published nowhere", b.op));
            }
        }
        for doc in &tools {
            if !branches.iter().any(|b| b.tool == Some(doc.name)) {
                return Err(format!("tool `{}` has no branch", doc.name));
            }
            if let Some(op) = doc.implicit_op {
                if !ops.contains(&(doc.name, op)) {
                    return Err(format!("tool `{}` has no op `{op}`", doc.name));
                }
            }
        }
        Ok(Self { tools, branches })
    }

    pub fn tools(&self) -> &[ToolDoc] {
        &self.tools
    }

    pub fn branches(&self) -> &[Branch<D>] {
        &self.branches
    }

    /// The legacy surface, in the order it was published.
    pub fn legacy(&self) -> impl Iterator<Item = &Branch<D>> {
        self.branches.iter().filter(|b| b.legacy.is_some())
    }

    /// The branch published as legacy tool `name`.
    pub fn legacy_branch(&self, name: &str) -> Option<&Branch<D>> {
        self.legacy().find(|b| b.legacy == Some(name))
    }

    /// Every branch of folded tool `name`, whoever is looking.
    pub(crate) fn branches_of(&self, name: &str) -> impl Iterator<Item = &Branch<D>> {
        let name = name.to_string();
        self.branches
            .iter()
            .filter(move |b| b.tool == Some(name.as_str()))
    }

    /// The branches of `name` that `view` can see and call.
    pub fn visible(&self, view: View, name: &str) -> Vec<&Branch<D>> {
        self.branches_of(name)
            .filter(|b| b.views.has(view))
            .collect()
    }

    /// The folded tools `view` sees, as `tools/list` publishes them: a tool
    /// with no branch in the view is not listed, and a listed tool's schema
    /// names only the branches the view can call.
    pub fn tools_list(&self, view: View) -> Vec<Value> {
        self.tools
            .iter()
            .filter_map(|doc| schema::tool_def(self, view, doc))
            .collect()
    }

    /// The branch a folded call means and the arguments it runs with, or
    /// the refusal a caller reads. A tool or `op` the view cannot see is
    /// answered as though it did not exist.
    pub fn resolve(&self, view: View, name: &str, args: Value) -> Resolved<'_, D> {
        check::resolve(self, view, name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn branch(tool: Option<&'static str>, op: &'static str, legacy: Option<&'static str>) -> Branch<u8> {
        Branch {
            tool,
            op,
            legacy,
            description: "",
            schema: || json!({"type": "object", "properties": {}}),
            extra: None,
            hidden: &[],
            fixed: &[],
            check: None,
            timeout_ms: 0,
            views: Views::ALL,
            dispatch: 0,
        }
    }

    fn doc(name: &'static str) -> ToolDoc {
        ToolDoc {
            name,
            description: "",
            implicit_op: None,
        }
    }

    fn refused(tools: Vec<ToolDoc>, branches: Vec<Branch<u8>>) -> String {
        match Catalog::new(tools, branches) {
            Ok(_) => panic!("a catalogue in which one name decides two things was built"),
            Err(why) => why,
        }
    }

    /// No name may decide two things: each refusal names what collided.
    #[test]
    fn a_name_that_would_decide_two_things_is_refused() {
        let t = || vec![doc("t")];
        let why = refused(vec![doc("t"), doc("t")], vec![branch(Some("t"), "a", None)]);
        assert!(why.contains("named twice"), "{why}");
        let why = refused(t(), vec![branch(Some("t"), "a", None), branch(Some("t"), "a", None)]);
        assert!(why.contains("op `a` twice"), "{why}");
        let why = refused(
            t(),
            vec![branch(Some("t"), "a", Some("old")), branch(Some("t"), "b", Some("old"))],
        );
        assert!(why.contains("legacy tool `old`"), "{why}");
        let why = refused(t(), vec![branch(Some("u"), "a", None)]);
        assert!(why.contains("unknown tool `u`"), "{why}");
        let why = refused(vec![doc("t"), doc("lonely")], vec![branch(Some("t"), "a", None)]);
        assert!(why.contains("`lonely` has no branch"), "{why}");
        let why = refused(t(), vec![branch(Some("t"), "a", None), branch(None, "b", None)]);
        assert!(why.contains("published nowhere"), "{why}");
        let mut implicit = doc("t");
        implicit.implicit_op = Some("zz");
        let why = refused(vec![implicit], vec![branch(Some("t"), "a", None)]);
        assert!(why.contains("no op `zz`"), "{why}");
    }

    /// Memory's own catalogue: its legacy surface is the seventeen tools in
    /// the order they were published, and its folded surface the five.
    #[test]
    fn memory_publishes_seventeen_legacy_tools_and_five_folded_ones() {
        let cat = memory::catalog();
        let legacy: Vec<&str> = cat.legacy().filter_map(|b| b.legacy).collect();
        assert_eq!(legacy.len(), 17);
        assert_eq!(legacy[0], "memory_search");
        let folded: Vec<String> = cat
            .tools_list(View::Local)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            folded,
            ["memory_search", "memory_add", "memory_update", "memory_delete", "memory_upkeep"]
        );
        let upkeep: Vec<&str> = cat.visible(View::Local, "memory_upkeep").iter().map(|b| b.op).collect();
        assert_eq!(
            upkeep,
            [
                "session_start", "harvest_day", "days", "remember_day", "dream_lease", "role",
                "sweep", "issues", "issue_add", "issue_resolve", "chains"
            ]
        );
    }
}
