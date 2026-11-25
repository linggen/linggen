//! The MCP catalogue: one table entry per branch decides a tool's name, its
//! schema, its timeout, what runs it, and who can see and call it.
//!
//! A *branch* is one operation a host can ask for, published as one `op` of
//! a tool that groups several (`memory_search` with `semantic` / `get` /
//! `list`), or as a tool of its own when its tool has one branch
//! (`memory_add`). Calls are checked strictly: an unknown `op`, a field of
//! another branch, a missing required field or a bad value is refused with
//! the form that works.
//!
//! Tools that were published before the tools were folded (`memory_list`,
//! `browser_click`) are [`Retired`] names: listed so a call by one is
//! answered "update the plugin", never run and never forwarded.
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

/// How this Linggen serves agents outside it (`[memory] outside_agents`):
/// the v1 way (the host writes into Linggen's store) or the v2 way (each
/// agent keeps its own memory and Linggen only reads it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    V1,
    V2,
}

/// Who is at the door and how this Linggen is serving them: the one value
/// `tools/list`, `tools/call` and the server text are all read from.
/// A caller inside Linggen (its own sessions, the memory daemon's `/mem/mcp`)
/// passes a bare [`View`], which is the door in [`Mode::V1`]: the store's
/// tools are theirs whichever way outside agents are served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Door {
    pub view: View,
    pub mode: Mode,
}

impl From<View> for Door {
    fn from(view: View) -> Self {
        Door { view, mode: Mode::V1 }
    }
}

impl Door {
    /// The same caller as the door of the v1 way.
    pub fn in_v1(self) -> Door {
        Door { mode: Mode::V1, ..self }
    }

    fn bit(self) -> u8 {
        match (self.view, self.mode) {
            (View::Local, Mode::V1) => 1,
            (View::Local, Mode::V2) => 2,
            (View::Remote, Mode::V1) => 4,
            (View::Remote, Mode::V2) => 8,
        }
    }
}

/// The doors a branch is visible and callable at: a set of (view, mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Views(u8);

impl Views {
    /// This machine, served either way.
    pub const LOCAL: Views = Views(1 | 2);
    /// This machine and the computers it approved, served either way.
    pub const ALL: Views = Views(1 | 2 | 4 | 8);
    /// This machine while outside agents are served the v1 way, and every
    /// approved computer: the store's own memory tools. Under v2 an agent
    /// outside Linggen keeps its own memory, so this machine's door stops
    /// offering them (an approved computer's agent has its memory on its
    /// own machine, out of this one's reach, so it keeps them).
    pub const MEMORY_V1: Views = Views(1 | 4 | 8);
    /// This machine, served the v1 way, only.
    pub const LOCAL_V1: Views = Views(1);

    pub fn has(self, door: impl Into<Door>) -> bool {
        self.0 & door.into().bit() != 0
    }
}

/// A cross-field rule a branch's schema cannot say (`ref` or `coordinate`,
/// not both). `Err` is the sentence the caller reads.
pub type Check = fn(&Map<String, Value>) -> Result<(), String>;

/// One operation, as the catalogue publishes it.
#[derive(Clone, Copy)]
pub struct Branch<D: Copy + 'static> {
    /// The tool this branch belongs to.
    pub tool: &'static str,
    /// The branch's `op` inside its tool.
    pub op: &'static str,
    /// A sentence the tool's description carries while this branch is in
    /// the caller's view ("" for none) — what a caller must know of this op
    /// that its fields do not say.
    pub note: &'static str,
    /// `{type: object, properties, required}` — the branch's fields.
    pub schema: fn() -> Value,
    /// More advertised properties, kept apart from `schema` where a table
    /// shares one schema between branches.
    pub extra: Option<fn() -> Value>,
    /// Fields a programmatic caller sends that the schema does not show
    /// to a model; accepted, never advertised.
    pub hidden: &'static [&'static str],
    /// The fields the host stamps onto a call, not the caller — the one
    /// table both hosts' hooks and the engine's own agents follow
    /// ([`Branch::host_fills`]). Never published.
    pub stamps: &'static [Stamp],
    /// Arguments the branch adds before it runs (the bridge's tab `action`
    /// a tab branch stands for).
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
            note: self.note,
            schema: self.schema,
            extra: self.extra,
            hidden: self.hidden,
            stamps: self.stamps,
            fixed: self.fixed,
            check: self.check,
            timeout_ms: self.timeout_ms,
            views: self.views,
            dispatch,
        }
    }

    /// The branch's advertised properties, extras included.
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

    /// Every field the branch takes: advertised and hidden.
    pub fn accepts(&self, key: &str) -> bool {
        self.advertises(key) || self.hidden.contains(&key)
    }

    /// Whether the branch's schema shows `key` — what a host may fill in
    /// for a model (`host`, `cwd`, `scope_root`): never a hidden filter.
    pub fn advertises(&self, key: &str) -> bool {
        self.properties().contains_key(key)
    }

    /// Whether the host stamps `field` on a call of this branch with
    /// `args` — the one rule for both hosts' hooks and the engine's own
    /// agents: the branch's [`stamps`](Self::stamps), each unless the call
    /// names its `unless` field.
    pub fn host_fills(&self, field: &str, args: &Value) -> bool {
        self.stamps
            .iter()
            .any(|s| s.field == field && s.applies(args))
    }

    /// Every field the host stamps on a call of this branch with `args`.
    pub fn host_filled(&self, args: &Value) -> Vec<String> {
        self.stamps
            .iter()
            .filter(|s| s.applies(args))
            .map(|s| s.field.to_string())
            .collect()
    }
}

/// A field the host stamps: unless the call names `unless` (a list that
/// names a `day` is never narrowed to a place).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub field: &'static str,
    pub unless: Option<&'static str>,
}

impl Stamp {
    /// Stamped always.
    pub const fn always(field: &'static str) -> Self {
        Self {
            field,
            unless: None,
        }
    }

    fn applies(&self, args: &Value) -> bool {
        self.unless
            .is_none_or(|u| args.get(u).is_none_or(Value::is_null))
    }
}

/// How a field's description tells a model the host fills it; every such
/// field is one of its branch's [`Branch::stamps`].
pub const HOST_FILLED: &str = "HOST-FILLED";

/// A tool's own name and text. Its branches are the entries naming it;
/// `implicit_op` is the branch a call without `op` means, when its fields
/// fit that branch (`memory_search` with only a `query`). `{ops}` in the
/// text is the list of ops the caller's view has.
#[derive(Clone, Copy)]
pub struct ToolDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub implicit_op: Option<&'static str>,
}

/// A tool name published before the tools were folded, and what does its
/// work now. A call by it runs nothing: the caller is told to update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retired {
    pub name: &'static str,
    pub now: &'static str,
}

/// The tools and their branches, checked once when composed.
pub struct Catalog<D: Copy + 'static> {
    tools: Vec<ToolDoc>,
    branches: Vec<Branch<D>>,
    retired: Vec<Retired>,
}

impl<D: Copy + 'static> Catalog<D> {
    /// Refuses a catalogue in which any name decides two things: a tool
    /// named twice, an `op` twice in one tool, a branch whose tool has no
    /// text, a tool with no branch, an implicit `op` the tool does not
    /// have, or a retired name that is named twice or is a tool again.
    pub fn new(
        tools: Vec<ToolDoc>,
        branches: Vec<Branch<D>>,
        retired: Vec<Retired>,
    ) -> Result<Self, String> {
        let mut names = HashSet::new();
        for doc in &tools {
            if !names.insert(doc.name) {
                return Err(format!("tool `{}` is named twice", doc.name));
            }
        }
        let mut ops = HashSet::new();
        for b in &branches {
            if !names.contains(b.tool) {
                return Err(format!("branch `{}` names unknown tool `{}`", b.op, b.tool));
            }
            if !ops.insert((b.tool, b.op)) {
                return Err(format!("tool `{}` has op `{}` twice", b.tool, b.op));
            }
        }
        for doc in &tools {
            if !branches.iter().any(|b| b.tool == doc.name) {
                return Err(format!("tool `{}` has no branch", doc.name));
            }
            if let Some(op) = doc.implicit_op {
                if !ops.contains(&(doc.name, op)) {
                    return Err(format!("tool `{}` has no op `{op}`", doc.name));
                }
            }
        }
        let mut gone = HashSet::new();
        for r in &retired {
            if names.contains(r.name) {
                return Err(format!("retired `{}` is a tool again", r.name));
            }
            if !gone.insert(r.name) {
                return Err(format!("retired `{}` is named twice", r.name));
            }
        }
        Ok(Self {
            tools,
            branches,
            retired,
        })
    }

    pub fn tools(&self) -> &[ToolDoc] {
        &self.tools
    }

    pub fn branches(&self) -> &[Branch<D>] {
        &self.branches
    }

    /// Every retired tool name, with what does its work now.
    pub fn retired(&self) -> &[Retired] {
        &self.retired
    }

    /// The retired name `name`, if it is one.
    pub fn retired_name(&self, name: &str) -> Option<&Retired> {
        self.retired.iter().find(|r| r.name == name)
    }

    /// Every branch of tool `name`, whoever is looking.
    pub(crate) fn branches_of(&self, name: &str) -> impl Iterator<Item = &Branch<D>> {
        let name = name.to_string();
        self.branches.iter().filter(move |b| b.tool == name)
    }

    /// The branches of `name` that `view` can see and call.
    pub fn visible(&self, door: impl Into<Door>, name: &str) -> Vec<&Branch<D>> {
        let door = door.into();
        self.branches_of(name)
            .filter(|b| b.views.has(door))
            .collect()
    }

    /// Whether `name` is a tool this door does not offer only because of
    /// the way outside agents are served: the v1 door would offer it.
    pub fn withheld_by_mode(&self, door: impl Into<Door>, name: &str) -> bool {
        let door = door.into();
        self.visible(door, name).is_empty() && !self.visible(door.in_v1(), name).is_empty()
    }

    /// The tools `view` sees, as `tools/list` publishes them: a tool with no
    /// branch in the view is not listed, and a listed tool's schema and
    /// text name only the branches the view can call.
    pub fn tools_list(&self, door: impl Into<Door>) -> Vec<Value> {
        let door = door.into();
        self.tools
            .iter()
            .filter_map(|doc| schema::tool_def(self, door, doc))
            .collect()
    }

    /// The branch a call means and the arguments it runs with, or the
    /// refusal a caller reads. A tool or `op` the view cannot see is
    /// answered as though it did not exist.
    pub fn resolve(&self, door: impl Into<Door>, name: &str, args: Value) -> Resolved<'_, D> {
        check::resolve(self, door.into(), name, args)
    }

    /// The branch a call names, read the way [`resolve`](Self::resolve)
    /// picks it (an `op`, or the implicit one when the fields fit), without
    /// checking its fields. `None` when the call names no branch the view
    /// has. For a client that fills fields in before the call goes out: it
    /// fills only the fields of the branch that will run.
    pub fn branch_of(&self, door: impl Into<Door>, name: &str, args: &Value) -> Option<&Branch<D>> {
        check::branch_of(self, door.into(), name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn branch(tool: &'static str, op: &'static str) -> Branch<u8> {
        Branch {
            tool,
            op,
            note: "",
            schema: || json!({"type": "object", "properties": {}}),
            extra: None,
            hidden: &[],
            stamps: &[],
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

    fn refused(tools: Vec<ToolDoc>, branches: Vec<Branch<u8>>, retired: Vec<Retired>) -> String {
        match Catalog::new(tools, branches, retired) {
            Ok(_) => panic!("a catalogue in which one name decides two things was built"),
            Err(why) => why,
        }
    }

    /// No name may decide two things: each refusal names what collided.
    #[test]
    fn a_name_that_would_decide_two_things_is_refused() {
        let t = || vec![doc("t")];
        let why = refused(vec![doc("t"), doc("t")], vec![branch("t", "a")], vec![]);
        assert!(why.contains("named twice"), "{why}");
        let why = refused(t(), vec![branch("t", "a"), branch("t", "a")], vec![]);
        assert!(why.contains("op `a` twice"), "{why}");
        let why = refused(t(), vec![branch("u", "a")], vec![]);
        assert!(why.contains("unknown tool `u`"), "{why}");
        let why = refused(
            vec![doc("t"), doc("lonely")],
            vec![branch("t", "a")],
            vec![],
        );
        assert!(why.contains("`lonely` has no branch"), "{why}");
        let mut implicit = doc("t");
        implicit.implicit_op = Some("zz");
        let why = refused(vec![implicit], vec![branch("t", "a")], vec![]);
        assert!(why.contains("no op `zz`"), "{why}");
        let old = |name| Retired { name, now: "t" };
        let why = refused(t(), vec![branch("t", "a")], vec![old("t")]);
        assert!(why.contains("retired `t` is a tool again"), "{why}");
        let why = refused(t(), vec![branch("t", "a")], vec![old("o"), old("o")]);
        assert!(why.contains("retired `o` is named twice"), "{why}");
    }

    /// Memory's own catalogue: the five tools, eleven upkeep ops the store
    /// runs, and the thirteen names its tools were published under before
    /// they were folded.
    #[test]
    fn memory_publishes_five_tools_and_retires_thirteen_names() {
        let cat = memory::catalog();
        let folded: Vec<String> = cat
            .tools_list(View::Local)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            folded,
            [
                "memory_search",
                "memory_add",
                "memory_update",
                "memory_delete",
                "memory_upkeep"
            ]
        );
        let upkeep: Vec<&str> = cat
            .visible(View::Local, "memory_upkeep")
            .iter()
            .map(|b| b.op)
            .collect();
        assert_eq!(
            upkeep,
            [
                "session_start",
                "harvest_day",
                "days",
                "remember_day",
                "dream_lease",
                "role",
                "sweep",
                "issues",
                "issue_add",
                "issue_resolve",
                "chains"
            ]
        );
        assert_eq!(cat.retired().len(), 13);
        assert_eq!(
            cat.retired_name("memory_get"),
            Some(&Retired {
                name: "memory_get",
                now: "memory_search op=get"
            })
        );
    }

    /// `{ops}` in a tool's text is the view's own op list, and a branch's
    /// note is in the text only while the branch is in the view.
    #[test]
    fn a_tools_text_names_the_ops_its_view_has() {
        let tools = vec![ToolDoc {
            name: "t",
            description: "Ops: {ops}.",
            implicit_op: None,
        }];
        let mut local = branch("t", "b");
        local.views = Views::LOCAL;
        local.note = "b is local.";
        let cat = Catalog::new(tools, vec![branch("t", "a"), local], vec![]).unwrap();
        let text = |view| {
            cat.tools_list(view)[0]["description"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(text(View::Local), "Ops: a, b. b is local.");
        assert_eq!(text(View::Remote), "Ops: a.");
    }

    /// A branch is visible at the doors its mask names: the store's tools
    /// not to this machine under v2 and to a bare view (a caller inside
    /// Linggen) always; `withheld_by_mode` is that difference alone.
    #[test]
    fn a_door_is_a_view_and_a_mode() {
        let door = |view, mode| Door { view, mode };
        assert!(Views::MEMORY_V1.has(door(View::Local, Mode::V1)));
        assert!(!Views::MEMORY_V1.has(door(View::Local, Mode::V2)));
        assert!(Views::MEMORY_V1.has(door(View::Remote, Mode::V2)));
        assert!(Views::LOCAL.has(door(View::Local, Mode::V2)));
        assert!(!Views::LOCAL.has(door(View::Remote, Mode::V1)));
        assert!(Views::LOCAL_V1.has(View::Local) && !Views::LOCAL_V1.has(door(View::Local, Mode::V2)));
        let mut old = branch("t", "a");
        old.views = Views::MEMORY_V1;
        let cat = Catalog::new(vec![doc("t"), doc("u")], vec![old, branch("u", "a")], vec![]).unwrap();
        let v2 = door(View::Local, Mode::V2);
        assert_eq!(cat.tools_list(v2).len(), 1);
        assert_eq!(cat.tools_list(View::Local).len(), 2);
        assert!(cat.withheld_by_mode(v2, "t"));
        assert!(!cat.withheld_by_mode(v2, "u"));
        assert!(!cat.withheld_by_mode(View::Local, "t"));
        assert!(!cat.withheld_by_mode(v2, "nothing"));
        assert!(matches!(cat.resolve(v2, "t", json!({})), Resolved::Unknown));
    }
}
