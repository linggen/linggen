//! Who started a turn: one closed set, and everything a start decides read
//! from it here — what the turn may read without asking, whether its words
//! are a person's, and which kind of turn it is for Yinyue's reply (held or
//! shown, which speaking contract, which finalizer).
//!
//! A door says its start once, when it builds the turn's [`ChatRunCtx`]
//! (`start`); nothing else names a kind of start. Each projection below is
//! one exhaustive `match`, so a new kind of start does not compile until it
//! says what it reads, how it is attributed and whether it is held.
//! Lock: `structure_lock::a_start_is_decided_and_projected_in_one_place`.
//!
//! Not decided here, and why: the session policy (`policy::TurnBy` — decided
//! before the start, because the owner's chat needs the policy's preset), and
//! the starts the engine makes itself (a mission, a Task child, the MCP door's
//! headless run). Those are members of the engine crate's own closed set,
//! `ReadStart`, which `reads` projects onto and which alone says what any
//! start may read (lock: `a_standing_for_reads_is_named_in_one_place`).

use super::handler::TurnOrigin;
use crate::engine::permission::{ReadOrigin, ReadStart};
use crate::engine::turn_place::TurnSettings;
use crate::engine::AgentEngine;
use crate::server::resident::Origin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnStart {
    /// The owner typed it, at this Mac, in their own chat. The only start a
    /// secret file may be asked of its owner for.
    OwnerHere,
    /// A person's words through any other door: a remote surface, a room's
    /// guest, an app's or a skill's page, her avatar.
    Person,
    /// A chat message a speaker relayed in (`sender`): not the sender's words.
    Relayed,
    /// A message kept in a restart's plan and sent again.
    Resumed,
    /// What the engine or another agent started for an agent that is not hers:
    /// a task's end told, a delivered `agent_chat`, a delegated run, a light
    /// reply, an approved plan's run.
    Engine,
    /// A turn of hers the engine started (never `Origin::Person`: a person's
    /// words to her are [`Self::Person`]). Built by [`Self::told`],
    /// [`Self::at_her_door`] and [`Self::resent_at_her_door`] alone.
    Hers(Origin),
}

impl TurnStart {
    /// A chat message's start. `door`: where it was admitted; `relayed`: a
    /// speaker's `sender` came with it; `owners_own_chat`: the session is the
    /// owner's own plain chat — theirs, no room guest, no skill's page, the
    /// chat preset.
    pub(super) fn of_chat(door: TurnOrigin, relayed: bool, owners_own_chat: bool) -> Self {
        match (door, relayed) {
            (TurnOrigin::Resumed, _) => Self::Resumed,
            (_, true) => Self::Relayed,
            (TurnOrigin::LocalPerson, false) if owners_own_chat => Self::OwnerHere,
            (TurnOrigin::LocalPerson | TurnOrigin::Person, false) => Self::Person,
        }
    }

    /// The start of a turn told to `agent` (a task's end, or the turn a
    /// restart continues): hers when it is her turn, else the engine's.
    pub(crate) fn told(agent: &str, told: Origin) -> Self {
        Self::of_her_kind(Origin::for_agent(agent, told), Self::Engine)
    }

    /// The start of a turn at her door, of the origin her door names: a
    /// `Person` origin there is a person's words (her avatar chat).
    pub(crate) fn at_her_door(origin: Origin) -> Self {
        Self::of_her_kind(origin, Self::Person)
    }

    /// [`Self::at_her_door`] for a turn kept in a restart's plan and run
    /// again: a person's words sent again are not spoken now.
    pub(crate) fn resent_at_her_door(origin: Origin) -> Self {
        Self::of_her_kind(origin, Self::Resumed)
    }

    /// `origin` as a start: her kinds are `Hers`; `Origin::Person` is
    /// `person`, which the door chose.
    fn of_her_kind(origin: Origin, person: Self) -> Self {
        match origin.is_hers() {
            true => Self::Hers(origin),
            false => person,
        }
    }

    /// What the turn stands as for reads, given what the session stood as
    /// when the turn before it ended: only the owner's own typing stands as
    /// the owner (a secret file may be asked of them); any other start keeps
    /// how freely it may look and nothing more. Projected onto the engine's
    /// own closed set of starts, which says what each reads (`ReadStart`).
    fn reads(self, was: ReadOrigin) -> ReadOrigin {
        match self {
            Self::OwnerHere => ReadStart::OwnerTyped,
            Self::Person | Self::Relayed | Self::Resumed | Self::Engine | Self::Hers(_) => {
                ReadStart::Unattended(was)
            }
        }
        .standing()
    }

    /// Whether the turn answers a person's own words in this session: what
    /// it files as the user's voice, and whether it may relay their answer to
    /// a waiting question (`answer_prompt`).
    fn person_spoke(self) -> bool {
        match self {
            Self::OwnerHere | Self::Person => true,
            Self::Relayed | Self::Resumed | Self::Engine | Self::Hers(_) => false,
        }
    }

    /// The kind of turn for her reply: held, contract, finalizer
    /// ([`Origin`]). Every start but hers is the answer path.
    pub(crate) fn origin(self) -> Origin {
        match self {
            Self::OwnerHere | Self::Person | Self::Relayed | Self::Resumed | Self::Engine => {
                Origin::Person
            }
            Self::Hers(origin) => origin,
        }
    }

    /// Put this start's standing for reads on the engine's session — decided
    /// here, where every turn of a session meets, and never left over from
    /// the turn before.
    pub(crate) fn stand_for_reads(self, engine: &mut AgentEngine) {
        let was = engine.session_spec().read_origin;
        let stands = self.reads(was);
        if stands != was {
            engine.respec(|spec| spec.read_origin = stands);
        }
    }

    /// `turn` as this start begins it: whether a person spoke is the
    /// start's, whatever the caller built.
    pub(crate) fn settings(self, turn: TurnSettings) -> TurnSettings {
        TurnSettings { person_spoke: self.person_spoke(), ..turn }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TurnOrigin::{LocalPerson, Person as Remote, Resumed};

    /// Every kind of start × what it says — the design table, executable.
    /// (reads after an owner's turn, reads after a task, a person spoke, the
    /// origin's name.)
    #[test]
    fn every_start_projects_as_the_table_says() {
        use ReadOrigin::{LocalOwner, LocalTask, Other};
        let rows = [
            (TurnStart::OwnerHere, LocalOwner, LocalOwner, true, "user"),
            (TurnStart::Person, LocalTask, LocalTask, true, "user"),
            (TurnStart::Relayed, LocalTask, LocalTask, false, "user"),
            (TurnStart::Resumed, LocalTask, LocalTask, false, "user"),
            (TurnStart::Engine, LocalTask, LocalTask, false, "user"),
            (TurnStart::Hers(Origin::Asked), LocalTask, LocalTask, false, "asked"),
            (TurnStart::Hers(Origin::Glance), LocalTask, LocalTask, false, "glance"),
        ];
        for (start, after_owner, after_task, spoke, name) in rows {
            assert_eq!(start.reads(LocalOwner), after_owner, "{start:?}");
            assert_eq!(start.reads(LocalTask), after_task, "{start:?}");
            assert_eq!(start.reads(Other), if start == TurnStart::OwnerHere { LocalOwner } else { Other }, "{start:?}");
            assert_eq!(start.person_spoke(), spoke, "{start:?}");
            assert_eq!(start.origin().name(), name, "{start:?}");
        }
    }

    #[test]
    fn a_chat_message_is_started_by_its_door_and_session() {
        assert_eq!(TurnStart::of_chat(LocalPerson, false, true), TurnStart::OwnerHere);
        // Not the owner's own plain chat (a skill's page, a room's guest).
        assert_eq!(TurnStart::of_chat(LocalPerson, false, false), TurnStart::Person);
        // Not at this Mac: a person's words, no standing as the owner.
        assert_eq!(TurnStart::of_chat(Remote, false, true), TurnStart::Person);
        assert_eq!(TurnStart::of_chat(LocalPerson, true, true), TurnStart::Relayed);
        assert_eq!(TurnStart::of_chat(Remote, true, false), TurnStart::Relayed);
        // A resent message is nobody's typing now, whoever typed it once.
        assert_eq!(TurnStart::of_chat(Resumed, false, true), TurnStart::Resumed);
        assert_eq!(TurnStart::of_chat(Resumed, true, true), TurnStart::Resumed);
    }

    #[test]
    fn a_turn_told_is_hers_only_to_her() {
        assert_eq!(TurnStart::told("ling", Origin::TaskEnd), TurnStart::Engine);
        assert_eq!(TurnStart::told("ling", Origin::Person), TurnStart::Engine);
        assert_eq!(TurnStart::told("yinyue", Origin::TaskEnd), TurnStart::Hers(Origin::TaskEnd));
        // A delegated run in her session is not her person's words.
        assert_eq!(TurnStart::told("yinyue", Origin::Person), TurnStart::Engine);
    }

    #[test]
    fn her_kinds_are_hers_and_a_persons_is_the_answer_path() {
        assert_eq!(TurnStart::at_her_door(Origin::Person).origin(), Origin::Person);
        assert_eq!(TurnStart::at_her_door(Origin::Glance), TurnStart::Hers(Origin::Glance));
        assert_eq!(TurnStart::resent_at_her_door(Origin::Asked), TurnStart::Hers(Origin::Asked));
    }
}

#[cfg(test)]
mod structure_lock {
    use crate::server::lock_scan::production_code;

    /// Files (with a count each) whose production code holds `needle`.
    fn found(needle: &str) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = production_code(&["src", "crates/linggen-engine/src"])
            .into_iter()
            // A test file is a module its parent marks as one.
            .filter(|(rel, _)| !rel.ends_with("tests.rs"))
            // A whole word's start: `Origin::Person` is not `TurnOrigin::Person`.
            .map(|(rel, code)| {
                let n = code
                    .match_indices(needle)
                    .filter(|(at, _)| !code[..*at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
                    .count();
                (rel, n)
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        out.sort();
        out
    }

    fn only_here(needle: &str) {
        let here = "src/server/chat/turn_start.rs";
        let elsewhere: Vec<_> = found(needle)
            .into_iter()
            .filter(|(rel, _)| rel != here && rel.starts_with("src/"))
            .collect();
        assert!(elsewhere.is_empty(), "`{needle}` is decided in `turn_start` alone: {elsewhere:?}");
    }

    /// A tripwire for the plain forms, not a proof (the exhaustive matches
    /// are the compiler's part): what a start decides is written in
    /// `turn_start.rs` and nowhere else.
    #[test]
    fn a_start_is_decided_and_projected_in_one_place() {
        // Only `of_chat` says the owner typed a turn (the owner's standing is
        // `ReadStart::OwnerTyped`, held to its one door by
        // `a_standing_for_reads_is_named_in_one_place`).
        only_here("OwnerHere");
        // Whether a person spoke is the start's.
        only_here("person_spoke");
        // A turn of hers is made only from an origin the constructors read.
        only_here("Hers(");
        // The standing is put on the engine by the start, at the turn core.
        let calls: Vec<_> = found("stand_for_reads(")
            .into_iter()
            .filter(|(rel, _)| rel != "src/server/chat/turn_start.rs")
            .collect();
        assert_eq!(
            calls,
            [("src/server/chat/handler.rs".to_string(), 1), ("src/server/chat/plan_flow.rs".to_string(), 1)],
        );
    }

    /// What a turn may read without asking is decided by one exhaustive
    /// `match` (`ReadStart::standing`, engine crate): no other production code
    /// names a standing, derives one from another, or builds a start the
    /// match does not list. A new kind of start does not compile until it
    /// says what it reads.
    #[test]
    fn a_standing_for_reads_is_named_in_one_place() {
        let home = "crates/linggen-engine/src/engine/permission/read_scope.rs";
        let outside = |needle: &str| -> Vec<(String, usize)> {
            found(needle).into_iter().filter(|(rel, _)| rel != home).collect()
        };
        for word in ["LocalOwner", "LocalDelegate", "LocalTask", "ReadOrigin::Other", "ReadOrigin::{", ".task_child(", ".unattended("] {
            let named = outside(word);
            assert!(named.is_empty(), "`{word}` names a standing for reads outside `read_scope.rs`: {named:?}");
        }
        // The starts, one by one: each is made at its own door and nowhere
        // else — this is the design table. (A start moved to another door, or
        // a door made to use another start, fails here.)
        let want = |rel: &str, n: usize| (rel.to_string(), n);
        let engine = "crates/linggen-engine/src/engine/";
        let table: [(&str, Vec<(String, usize)>); 5] = [
            // The owner typed it, at this Mac, in their own chat.
            ("ReadStart::OwnerTyped", vec![want("src/server/chat/turn_start.rs", 1)]),
            // A host program at this Mac, through the owner's own door.
            ("ReadStart::HostAtThisMac", vec![want("src/server/mcp/mod.rs", 1)]),
            // Any other start of a turn nobody typed; an instruction file the engine reads for the prompt.
            (
                "ReadStart::Unattended(",
                vec![want(&format!("{engine}permission/model.rs"), 1), want("src/server/chat/turn_start.rs", 1)],
            ),
            // A Task child, a Task tool call's and a batch of delegations' alike.
            ("ReadStart::TaskChild(", vec![want(&format!("{engine}tool_exec.rs"), 1)]),
            // Not this Mac's door; a mission, a private app's or a skill's page (`read_origin()`);
            // a check that names no turn (`check_permission_for`, no caller outside tests).
            (
                "ReadStart::NoStanding",
                vec![
                    want(&format!("{engine}permission/model.rs"), 1),
                    want(&format!("{engine}tool_exec.rs"), 1),
                    want("src/server/mcp/mod.rs", 1),
                ],
            ),
        ];
        for (start, sites) in table {
            assert_eq!(outside(start), sites, "`{start}` is made at its own doors only");
        }
        // A session no start has been put on yet stands as no one: the
        // default, which is `NoStanding`, held in these places only.
        assert_eq!(outside("read_origin: Default::default()"), [want(&format!("{engine}session_spec/mod.rs"), 2)]);
        assert_eq!(
            outside("parent_read_origin: Default::default()"),
            [want(&format!("{engine}tools/mod.rs"), 1)],
        );
        // Where a standing is put on an engine or a child: it is only passed.
        assert_eq!(
            outside("read_origin ="),
            [
                want("crates/linggen-engine/src/engine/tools/delegation.rs", 1), // the child's, as given
                want("src/server/chat/turn_start.rs", 1), // `stand_for_reads`
                want("src/server/mcp_agent.rs", 1), // the door's, as given
            ],
        );
    }

    /// A turn that can read gets its standing from its start, afresh: never
    /// what the turn before it left. Every file that builds a turn's context
    /// puts the standing on the engine (the turn core does) — but the light
    /// reply, which has no tools and reads nothing.
    #[test]
    fn a_turn_that_can_read_stands_for_reads_from_its_own_start() {
        const READS_NOTHING: &[&str] = &["src/server/chat/light.rs"];
        for (rel, code) in production_code(&["src"]) {
            let builds = code.replace("struct ChatRunCtx {", "").replace("impl ChatRunCtx {", "").contains("ChatRunCtx {");
            if !builds || rel.ends_with("tests.rs") || READS_NOTHING.contains(&rel.as_str()) {
                continue;
            }
            assert!(
                code.contains("stand_for_reads(") || code.contains("run_session_turn("),
                "{rel} builds a turn's context and never stands it for reads: it would read as the turn before it did"
            );
        }
    }

    /// Which guardrail an unprompted wake answers to is read from
    /// `unprompted_source` — before the turn and after it alike — never named
    /// beside a check. (A `Source` named to write its log word is no check.)
    #[test]
    fn a_guardrail_is_chosen_by_one_mapping_only() {
        for (rel, code) in production_code(&["src"]) {
            let own = ["src/server/resident/guard.rs", "src/server/resident/triggers.rs"];
            if rel.ends_with("tests.rs") || own.contains(&rel.as_str()) {
                continue;
            }
            for line in code.lines().filter(|l| l.contains("Source::")) {
                let checks = ["held_now(", "pass_guards(", ".check("].iter().any(|c| line.contains(c));
                assert!(!checks, "{rel}: `{}` picks a guardrail by hand", line.trim());
            }
        }
    }

    /// `Origin::Person` has been the default for "not hers". Each use is
    /// looked at, then listed: a new one fails here.
    #[test]
    fn origin_person_is_never_a_new_default() {
        let want = |rel: &str, n: usize| (rel.to_string(), n);
        let used: Vec<_> = found("Origin::Person")
            .into_iter()
            .filter(|(rel, _)| rel != "src/server/chat/turn_start.rs")
            .collect();
        assert_eq!(
            used,
            [
                want("src/server/api/yinyue.rs", 1),    // her avatar chat: a person's words
                want("src/server/mcp_agent.rs", 1),     // a delegated run told into a session
                want("src/server/resident/triggers.rs", 1), // `unprompted_source`: not unprompted
                want("src/server/resident/turn.rs", 1), // the sender string
                want("src/server/restart.rs", 1),       // a plan from before origins were kept
            ],
        );
    }
}
