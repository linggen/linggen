//! The session spec: what a session is, assembled from its preset's row.
//!
//! An occasion yields a [`Preset`]; [`SessionSpec::of`] fills the preset's
//! row of strategies from what the session's meta holds; [`SessionSpec::apply`]
//! writes the spec onto an engine. Each behaviour that varies between kinds
//! of session is one strategy (an enum, one small function each), so a new
//! kind of session is a new row in [`row`] (`doc/session-spec.md` § The
//! session spec).

mod apply;
pub mod delegate;
mod preset;

pub use preset::{is_companion_thread, is_kept_apart, preset_of, Preset, HANDOFF_PREFIX};

use super::session_policy::SessionPolicy;
use crate::state_fs::sessions::{SessionMeta, SessionRuntime};
use std::collections::HashSet;

/// One session's behaviour, as its preset's row and its meta make it.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSpec {
    pub preset: Preset,
    /// The kind of work the session is for, when its preset accepts it.
    pub work: Option<Work>,
    /// Which runtime hosts it (a Claude session never gets a native engine).
    pub runtime: SessionRuntime,
    /// Who leads it.
    pub seat: Seat,
    /// Where it works when it is made.
    pub folder: Folder,
    /// The tools it may use beyond what its lead and place declare.
    pub tools: Tools,
    /// Which of the person's memory its prompt carries.
    pub memory: Memory,
    /// How a turn's words reach the model.
    pub framing: Framing,
    /// Who answers an action beyond its grants.
    pub permission: Permission,
    /// How its thread is kept within the model's window.
    pub context: Context,
    /// Whether it may switch models when its own fails: `None` = the
    /// configured default.
    pub model_fallback: Option<bool>,
    /// Who made it, as stored (`SessionMeta::creator`); `None` for an engine
    /// no stored session owns (a `Task` child). Its world follows from it
    /// ([`SessionSpec::world`]).
    pub creator: Option<String>,
    /// Who is asking (the owner, or a proxy-room consumer under the owner's
    /// ceiling); `None` when no person's policy claimed the run (a mission,
    /// an `agent_run` delegate).
    pub policy: Option<SessionPolicy>,
    /// Reserved for the paired-device design: unused.
    pub ceiling: Option<Ceiling>,
}

/// A session's kind of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Work { Code }

/// A ceiling another design will set. No variant yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceiling {}

/// Who leads a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// Ling, the main agent.
    Ling,
    /// The companion, alone in her own thread.
    Companion,
    /// The bound skill's declared members (Ling when it declares none).
    SkillMembers,
    /// The mission's agent.
    MissionAgent,
    /// The agent the caller named (Ling by default).
    Caller,
    /// The Claude Agent SDK's agent.
    Claude,
}

/// Where a session works when it is made (`doc/session-spec.md` § Default workspace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    /// The default workspace, or a picked project.
    Workspace,
    /// The skill's `cwd:`, else its folder.
    Skill,
    /// The mission's `cwd:`, else its project, else the default workspace.
    Mission,
    /// The caller's `write_dir`, else the default workspace.
    WriteDir,
}

/// The tools a session may use, as the engine's restriction set.
#[derive(Debug, Clone, PartialEq)]
pub enum Tools {
    /// Its lead's declared list, narrowed by the turn's place.
    Lead,
    /// A mission's `allowed-tools` with its skill's own (`None`: unrestricted).
    Mission(Option<HashSet<String>>),
    /// An `agent_run` delegate's set; `write` adds Bash, Write and Edit.
    Delegate { write: bool },
}

impl Tools {
    /// The restriction set the engine enforces; `None` = no restriction.
    pub fn allowlist(&self) -> Option<HashSet<String>> {
        match self {
            Self::Lead => None,
            Self::Mission(scope) => scope.clone(),
            Self::Delegate { write } => delegate::tool_scope(*write),
        }
    }
}

/// Which memory a session's prompt carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    /// Core and the whole biography: a person's own session.
    Full,
    /// Core and the app's own memory: an app's session.
    CoreAndApp,
    /// Neither: a mission.
    None,
}

impl Memory {
    /// The memory of a turn by who it belongs to (`chat::turn_place::turn_creator`):
    /// a mission's keeps none, an app's core and its own, a person's all.
    pub fn of_turn(owner: &str) -> Self {
        match owner {
            "mission" => Self::None,
            "user" => Self::Full,
            _ => Self::CoreAndApp,
        }
    }

    fn narrow(self, profile: &mut super::prompt::profile::PromptProfile) {
        match self {
            Self::Full => {}
            Self::CoreAndApp => profile.narrow_for_creator("skill"),
            Self::None => profile.narrow_for_creator("mission"),
        }
    }
}

/// How a turn's words reach the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// Verbatim, as a chat turn.
    Turn,
    /// Wrapped as a task (workspace listing + step-by-step coda).
    Task,
}

/// Who answers an action beyond the grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// The person, by a prompt.
    Interactive,
    /// The caller (`agent_permit`); refused when it does not answer in time.
    AskCaller,
    /// Nobody: refused at once.
    Unattended,
}

/// How a session's thread is kept within the model's window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Context {
    /// Its own thread, compacted past `threshold` of the window (`None`:
    /// the session's, else the config's, else 0.95).
    Own { threshold: Option<f32> },
    /// The day's one thread, a new one from 04:00 local.
    Daily,
    /// A fresh segment past `fraction` of the window, `live` messages kept
    /// in the prompt.
    Rolling { fraction: f32, live: usize },
}

/// The companion's own thread rolls to a fresh segment past this fraction
/// of the window: she starts clean and leans on memory.
pub const COMPANION_ROLL_FRACTION: f32 = 0.7;
/// The companion's last messages kept live in her own thread's prompt.
pub const COMPANION_LIVE_MESSAGES: usize = 10;
/// The companion's own thread's context.
pub const COMPANION_CONTEXT: Context = Context::Rolling {
    fraction: COMPANION_ROLL_FRACTION,
    live: COMPANION_LIVE_MESSAGES,
};

/// A preset's row: its strategies before the meta fills them.
struct Row {
    seat: Seat,
    folder: Folder,
    tools: Tools,
    memory: Memory,
    framing: Framing,
    permission: Permission,
    context: Context,
}

/// The table: one row per preset.
fn row(preset: Preset) -> Row {
    use Preset::*;
    let own = Context::Own { threshold: None };
    let (seat, folder, memory, context) = match preset {
        Chat => (Seat::Ling, Folder::Workspace, Memory::Full, own),
        App | OneOff => (Seat::SkillMembers, Folder::Skill, Memory::CoreAndApp, own),
        Daily => (Seat::Ling, Folder::Workspace, Memory::Full, Context::Daily),
        Companion => (
            Seat::Companion,
            Folder::Workspace,
            Memory::Full,
            COMPANION_CONTEXT,
        ),
        Handoff => (Seat::SkillMembers, Folder::Skill, Memory::CoreAndApp, own),
        Mission => (Seat::MissionAgent, Folder::Mission, Memory::None, own),
        Delegate => (Seat::Caller, Folder::WriteDir, Memory::Full, own),
        Claude => (Seat::Claude, Folder::Workspace, Memory::Full, own),
    };
    let (tools, framing, permission) = match preset {
        Mission => (Tools::Mission(None), Framing::Task, Permission::Unattended),
        Delegate => (
            Tools::Delegate { write: false },
            Framing::Task,
            Permission::AskCaller,
        ),
        _ => (Tools::Lead, Framing::Turn, Permission::Interactive),
    };
    Row {
        seat,
        folder,
        tools,
        memory,
        framing,
        permission,
        context,
    }
}

impl SessionSpec {
    /// The spec of a session kept on disk: its preset's row, filled from
    /// what its meta holds.
    pub fn of(meta: &SessionMeta) -> Self {
        let preset = preset_of(meta);
        let row = row(preset);
        let tools = match (preset, delegate::kept_toolset(meta)) {
            (Preset::Delegate, Some(write)) => Tools::Delegate { write },
            (Preset::Delegate, None) => Tools::Lead,
            _ => row.tools,
        };
        let memory = match (preset, meta.skill.is_some()) {
            (Preset::Handoff, false) => Memory::Full,
            _ => row.memory,
        };
        let context = match row.context {
            Context::Own { .. } => Context::Own {
                threshold: meta.compact_threshold,
            },
            other => other,
        };
        let policy = matches!(row.framing, Framing::Turn).then(SessionPolicy::owner);
        Self {
            preset,
            work: if preset.takes_work() { meta.work } else { None },
            runtime: meta.runtime,
            seat: row.seat,
            folder: row.folder,
            tools,
            memory,
            framing: row.framing,
            permission: row.permission,
            context,
            model_fallback: meta.model_fallback,
            creator: Some(meta.creator.clone()),
            policy,
            ceiling: None,
        }
    }

    /// An engine no session has claimed yet: what a fresh engine is — the
    /// owner's sections framed as a task, no restriction, made by `creator`.
    pub fn unclaimed(creator: Option<String>) -> Self {
        Self {
            preset: Preset::Chat,
            work: None,
            runtime: SessionRuntime::Linggen,
            seat: Seat::Ling,
            folder: Folder::Workspace,
            tools: Tools::Lead,
            memory: Memory::Full,
            framing: Framing::Task,
            permission: Permission::Interactive,
            context: Context::Own { threshold: None },
            model_fallback: None,
            creator,
            policy: None,
            ceiling: None,
        }
    }

    /// A turn a person typed, under `policy`, its memory the turn's
    /// (`Memory::of_turn`), verbatim. A delegate's session keeps its toolset
    /// and its caller; any other gets the lead's tools and asks the person,
    /// unless a consumer's policy locks it.
    pub fn typed(mut self, policy: SessionPolicy, memory: Memory) -> Self {
        let keeps_caller = self.preset == Preset::Delegate;
        self.permission = match (policy.locked, keeps_caller) {
            (true, _) => Permission::Unattended,
            (false, true) => Permission::AskCaller,
            (false, false) => Permission::Interactive,
        };
        if !keeps_caller {
            self.tools = Tools::Lead;
        }
        self.policy = Some(policy);
        self.memory = memory;
        self.framing = Framing::Turn;
        self
    }

    /// A delegated `Task` child of a parent run under `policy`: a task
    /// whatever the parent's session, prompting only when the parent could.
    pub fn delegated_child(policy: Option<SessionPolicy>, parent_interactive: bool, work: Option<Work>) -> Self {
        let mut spec = Self::unclaimed(None);
        spec.work = work;
        spec.policy = policy;
        spec.permission = match parent_interactive {
            true => Permission::Interactive,
            false => Permission::Unattended,
        };
        spec
    }

    /// Whether its turns carry the world block (`prompt::world::world_rides`).
    pub fn world(&self) -> bool {
        let consumer = self
            .policy
            .as_ref()
            .is_some_and(|p| p.prompt_profile.consumer_frame);
        super::prompt::world::world_rides(self.creator.as_deref(), consumer)
    }

    /// Whether an action beyond the grants may ask the person.
    pub fn may_ask_person(&self) -> bool {
        self.permission == Permission::Interactive
    }
}

#[cfg(test)]
mod lock_tests;
#[cfg(test)]
mod tests;
