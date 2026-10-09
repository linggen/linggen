//! Agents that appear only once met.
//!
//! An agent may declare `met_when` in its frontmatter: a state a skill keeps
//! (`skill`, `file`, `path` — the same flag a skill's `absent_until` reads).
//! Until that state has been seen, the agent is absent from every surface the
//! engine controls — the agent list and `@` picker, chats, heralds, its own
//! turns. Once seen it is met **for good**: a one-way latch under
//! `~/.linggen/met/<agent>.json`, written once and never cleared, whatever the
//! skill's state later reads (a save forgotten, a game restarted).
//!
//! The latch is also set at startup, before the server binds, when the agent
//! already has chat history with this person ("history") — an upgrade never
//! makes someone's companion vanish. The engine names no agent and no skill:
//! it reads what the agent declares.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::engine::agent::AgentManager;
use crate::engine::skill::record::StateFlag;
use crate::engine::skill::SkillRegistry;
use crate::state_fs::SessionStore;
use crate::util::LockExt;

/// What an agent's `met_when` names: the skill, and the state in its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetWhen {
    pub skill: String,
    #[serde(flatten)]
    pub flag: StateFlag,
}

/// Why the latch was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetReason {
    /// The declared state was seen set.
    Trigger,
    /// The agent already had chat history with this person.
    History,
}

/// The latch file's content: when, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Met {
    pub at: String,
    pub reason: MetReason,
}

/// One declaring agent as the surfaces report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentMet {
    pub id: String,
    pub present: bool,
    pub met: Option<Met>,
}

#[derive(Default)]
struct Book {
    /// The agents that declare `met_when`.
    declared: HashMap<String, MetWhen>,
    /// The latches seen so far; never removed.
    latched: HashMap<String, Met>,
}

/// The gate. Cheap to ask, synchronous: a declared agent is present once
/// its latch is in the book.
pub struct MetGate {
    dir: PathBuf,
    book: Mutex<Book>,
}

impl MetGate {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            book: Mutex::new(Book::default()),
        }
    }

    /// Whether `agent` is there at all: it declares no `met_when`, or it has
    /// been met. The one function every surface asks.
    pub fn present(&self, agent: &str) -> bool {
        let book = self.book.lock_ok();
        !book.declared.contains_key(agent) || book.latched.contains_key(agent)
    }

    /// Take the agents' declarations as they stand now, and pick up the latch
    /// each one already has on disk. A latch is never dropped: an agent that
    /// stops declaring is simply always present.
    pub fn declare(&self, declared: HashMap<String, MetWhen>) {
        let mut book = self.book.lock_ok();
        for id in declared.keys() {
            if !book.latched.contains_key(id) {
                if let Some(met) = read_latch(&self.dir, id) {
                    book.latched.insert(id.clone(), met);
                }
            }
        }
        book.declared = declared;
    }

    /// Test seam: `ids` declare a trigger and are unmet, whatever this
    /// machine has latched on disk.
    #[cfg(test)]
    pub(crate) fn declare_unmet(&self, ids: &[&str]) {
        let mut book = self.book.lock_ok();
        for id in ids {
            book.latched.remove(*id);
            book.declared.insert(id.to_string(), test_when());
        }
    }

    /// The declaring agents not yet met, with what they wait for.
    pub fn waiting(&self) -> Vec<(String, MetWhen)> {
        let book = self.book.lock_ok();
        let mut out: Vec<(String, MetWhen)> = book
            .declared
            .iter()
            .filter(|(id, _)| !book.latched.contains_key(*id))
            .map(|(id, w)| (id.clone(), w.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Latch `agent` as met. True when this call set it; false when it was
    /// already met (the first date and reason stand).
    pub fn latch(&self, agent: &str, reason: MetReason) -> bool {
        let mut book = self.book.lock_ok();
        if book.latched.contains_key(agent) {
            return false;
        }
        let (met, fresh) = write_latch(&self.dir, agent, reason);
        book.latched.insert(agent.to_string(), met);
        fresh
    }

    /// Every declaring agent and whether it is met (sorted by id).
    pub fn status(&self) -> Vec<AgentMet> {
        let book = self.book.lock_ok();
        let mut out: Vec<AgentMet> = book
            .declared
            .keys()
            .map(|id| {
                let met = book.latched.get(id).cloned();
                AgentMet {
                    id: id.clone(),
                    present: met.is_some(),
                    met,
                }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

fn latch_file(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("{agent}.json"))
}

/// The latch on disk, if the agent has one.
fn read_latch(dir: &Path, agent: &str) -> Option<Met> {
    let text = std::fs::read_to_string(latch_file(dir, agent)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the latch unless one is already there. Returns what the file now
/// holds and whether this call wrote it. A write that fails still latches in
/// memory for this run — the next start sees the trigger or the history again.
fn write_latch(dir: &Path, agent: &str, reason: MetReason) -> (Met, bool) {
    if let Some(existing) = read_latch(dir, agent) {
        return (existing, false);
    }
    let met = Met {
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        reason,
    };
    let bytes = serde_json::to_vec_pretty(&met).unwrap_or_default();
    let written = std::fs::create_dir_all(dir)
        .and_then(|_| crate::state_fs::devices::write_atomic(&latch_file(dir, agent), &bytes));
    match written {
        Ok(()) => tracing::info!("[met] {agent} is met ({reason:?})"),
        Err(e) => tracing::warn!("[met] could not keep the latch for {agent}: {e}"),
    }
    (met, true)
}

/// Whether `agent` already has chat history with this person: a row it
/// spoke itself, in its own rolling sessions (`sess-<agent>-…`) or in any
/// session that seats it. Streams the candidates and stops at the first hit.
pub fn has_history(store: &SessionStore, agent: &str) -> bool {
    let Ok(mut sessions) = store.list_sessions() else {
        return false;
    };
    let own = format!("sess-{agent}-");
    let seats = |m: &crate::state_fs::sessions::SessionMeta| m.agents.iter().any(|a| a.id == agent);
    sessions.retain(|m| m.id.starts_with(&own) || seats(m));
    // Its own threads first: the likeliest hit, and the cheapest.
    sessions.sort_by_key(|m| !m.id.starts_with(&own));
    sessions.iter().any(|m| spoke_in(store, &m.id, agent))
}

fn spoke_in(store: &SessionStore, session_id: &str, agent: &str) -> bool {
    store.get_chat_history(session_id).is_ok_and(|rows| {
        rows.iter()
            .any(|r| r.from_id == agent && r.agent_id == agent && !r.is_observation)
    })
}

/// Whether the skill's declared state reads set. A skill that isn't
/// installed (yet) or keeps no folder reads unset.
async fn trigger_seen(skills: &dyn SkillRegistry, when: &MetWhen) -> bool {
    let Some(skill) = skills.get_skill(&when.skill).await else {
        return false;
    };
    skill
        .skill_dir
        .as_deref()
        .is_some_and(|dir| when.flag.is_set(dir))
}

impl AgentManager {
    /// Whether `agent` is there at all, as last seen. Synchronous.
    pub fn agent_present(&self, agent: &str) -> bool {
        self.met.present(agent)
    }

    /// Whether `agent` is there at all, reading its trigger now when it is
    /// still waiting — so a state a skill has just set counts at once.
    pub async fn agent_present_now(&self, agent: &str) -> bool {
        if self.met.present(agent) {
            return true;
        }
        let waiting = self.met.waiting();
        let Some((_, when)) = waiting.iter().find(|(id, _)| id == agent) else {
            return true;
        };
        if trigger_seen(self.skills.as_ref(), when).await {
            self.met.latch(agent, MetReason::Trigger);
            return true;
        }
        false
    }

    /// Read every agent's `met_when` from the specs installed now.
    pub async fn load_met_declarations(&self) {
        let Ok(specs) = self.agents.list(crate::paths::linggen_home()).await else {
            return;
        };
        let declared = specs
            .into_iter()
            .filter_map(|s| s.spec.met_when.map(|w| (s.agent_id, w)))
            .collect();
        self.met.declare(declared);
    }

    /// Latch each waiting agent whose trigger now reads set. Returns the ids
    /// newly met.
    pub async fn check_met_triggers(&self) -> Vec<String> {
        let mut now_met = Vec::new();
        for (id, when) in self.met.waiting() {
            if trigger_seen(self.skills.as_ref(), &when).await
                && self.met.latch(&id, MetReason::Trigger)
            {
                now_met.push(id);
            }
        }
        now_met
    }

    /// Startup, before the server binds: read the declarations, then latch
    /// every waiting agent that already has history with this person (so an
    /// upgrade keeps her), then whatever the triggers already read set.
    pub async fn settle_met(&self) {
        self.load_met_declarations().await;
        for (id, _) in self.met.waiting() {
            if has_history(&self.global_sessions, &id) {
                self.met.latch(&id, MetReason::History);
            }
        }
        self.check_met_triggers().await;
    }
}

#[cfg(test)]
fn test_when() -> MetWhen {
    serde_norway::from_str("skill: game\nfile: data/state.json\npath: companion.joined\n").unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_fs::sessions::{ChatMsg, SessionMember, SessionMeta};

    fn when() -> MetWhen {
        serde_norway::from_str("skill: game\nfile: data/state.json\npath: companion.joined\n")
            .unwrap()
    }

    fn gate(dir: &Path, declares: &[&str]) -> MetGate {
        let gate = MetGate::new(dir.to_path_buf());
        gate.declare(declares.iter().map(|id| (id.to_string(), when())).collect());
        gate
    }

    #[test]
    fn met_when_reads_as_one_map() {
        let w = when();
        assert_eq!(w.skill, "game");
        assert_eq!(w.flag.file, "data/state.json");
        assert_eq!(w.flag.path, "companion.joined");
        let spec: crate::engine::agent::record::AgentSpec = serde_norway::from_str(
            "name: her\ndescription: d\ntools: []\nmet_when: {skill: game, file: s.json, path: a.b}\n",
        )
        .unwrap();
        assert_eq!(spec.met_when.unwrap().flag.path, "a.b");
    }

    /// Declares nothing: always there. Declares it: there once met.
    #[test]
    fn an_agent_is_present_only_once_met_when_it_declares_so() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate(dir.path(), &["her"]);
        assert!(g.present("ling"), "declares nothing: always there");
        assert!(!g.present("her"));
        assert!(g.latch("her", MetReason::Trigger));
        assert!(g.present("her"));
    }

    /// Set once: the first date and reason stand, on disk and in memory, and
    /// nothing the declaration or the files do afterwards clears it.
    #[test]
    fn the_latch_is_set_once_and_never_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate(dir.path(), &["her"]);
        assert!(g.latch("her", MetReason::Trigger));
        let first = read_latch(dir.path(), "her").unwrap();
        assert_eq!(first.reason, MetReason::Trigger);
        assert!(!g.latch("her", MetReason::History), "already met");
        assert_eq!(read_latch(dir.path(), "her").unwrap(), first);

        // Declared again, or no longer declared, or a new start: still met.
        g.declare([("her".to_string(), when())].into());
        assert!(g.present("her"));
        g.declare(HashMap::new());
        g.declare([("her".to_string(), when())].into());
        assert!(g.present("her"));
        let restarted = gate(dir.path(), &["her"]);
        assert!(restarted.present("her"), "the latch outlives the process");
        assert!(!restarted.latch("her", MetReason::History));
        assert!(restarted.waiting().is_empty());
        assert_eq!(restarted.status()[0].met.as_ref(), Some(&first));
    }

    #[test]
    fn a_waiting_agent_is_listed_with_what_it_waits_for() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate(dir.path(), &["her"]);
        assert_eq!(g.waiting(), vec![("her".to_string(), when())]);
        let s = g.status();
        assert_eq!((s[0].id.as_str(), s[0].present), ("her", false));
        assert!(s[0].met.is_none());
    }

    fn state_json(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/state.json"), body).unwrap();
    }

    struct Skills(Option<crate::engine::skill::Skill>);
    #[async_trait::async_trait]
    impl crate::engine::skill::SkillRegistry for Skills {
        async fn get_skill(&self, name: &str) -> Option<crate::engine::skill::Skill> {
            self.0.clone().filter(|s| s.name == name)
        }
        async fn reload_one(&self, name: &str) -> Option<crate::engine::skill::Skill> {
            self.get_skill(name).await
        }
        async fn list_skills(&self) -> Vec<crate::engine::skill::Skill> {
            self.0.iter().cloned().collect()
        }
        async fn match_trigger(&self, _: &str) -> Option<(String, String)> {
            None
        }
        async fn list_metadata(&self) -> Vec<(String, String, bool)> {
            Vec::new()
        }
    }

    fn game(dir: &Path) -> crate::engine::skill::Skill {
        let mut skill = crate::extensions::skills::parse_skill_text(
            "---\nname: game\ndescription: d\n---\nBody.",
            crate::engine::skill::SkillSource::Global,
        )
        .unwrap();
        skill.skill_dir = Some(dir.to_path_buf());
        skill
    }

    async fn seen(skills: Skills) -> bool {
        trigger_seen(&skills, &when()).await
    }

    /// The trigger is the skill's own state read through its folder: unset
    /// until the value is set, unset when the skill isn't installed.
    #[tokio::test]
    async fn the_trigger_reads_the_skills_state() {
        let skill_dir = tempfile::tempdir().unwrap();
        assert!(!seen(Skills(None)).await, "no such skill: unset");
        assert!(!seen(Skills(Some(game(skill_dir.path())))).await, "no file");
        state_json(skill_dir.path(), r#"{"companion":{"joined":null}}"#);
        assert!(!seen(Skills(Some(game(skill_dir.path())))).await);
        state_json(skill_dir.path(), r#"{"companion":{"joined":"2026-10-09"}}"#);
        assert!(seen(Skills(Some(game(skill_dir.path())))).await);
    }

    fn row(from: &str, to: &str, observation: bool) -> ChatMsg {
        ChatMsg {
            agent_id: "her".into(),
            from_id: from.into(),
            to_id: to.into(),
            content: "hi".into(),
            timestamp: 1,
            is_observation: observation,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
        }
    }

    fn session(store: &SessionStore, id: &str, seated: &[&str], rows: &[ChatMsg]) {
        let mut meta = SessionMeta {
            id: id.into(),
            title: id.into(),
            created_at: 1,
            updated_at: 1,
            skill: None,
            creator: "user".into(),
            agents: seated.iter().map(|a| SessionMember::new(a)).collect(),
            legacy: Default::default(),
            cwd: None,
            project: None,
            project_name: None,
            mission_id: None,
            user_id: None,
            compact_threshold: None,
            compact_focus: None,
            title_locked: false,
            withheld_tools: Vec::new(),
        };
        meta.legacy = Default::default();
        store.add_session(&meta).unwrap();
        for r in rows {
            store.add_chat_message(id, r).unwrap();
        }
    }

    /// A line she spoke is history; what was only said to her, or her tool
    /// results, is not. Her own threads and any session that seats her count.
    #[test]
    fn history_is_a_line_the_agent_spoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        assert!(!has_history(&store, "her"), "a fresh install: none");

        session(
            &store,
            "sess-her-2026-10-01",
            &[],
            &[row("user", "her", false), row("her", "user", true)],
        );
        session(
            &store,
            "sess-other",
            &["ling"],
            &[row("her", "user", false)],
        );
        assert!(
            !has_history(&store, "her"),
            "only said to her, or a tool result, or a thread she is not in"
        );

        session(
            &store,
            "sess-seated",
            &["ling", "her"],
            &[row("her", "user", false)],
        );
        assert!(
            has_history(&store, "her"),
            "spoke at a table that seats her"
        );
    }

    #[test]
    fn history_in_her_own_thread_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        session(
            &store,
            "sess-her-2026-10-02",
            &[],
            &[row("user", "her", false), row("her", "user", false)],
        );
        assert!(has_history(&store, "her"));
        assert!(!has_history(&store, "someone-else"));
    }
}
