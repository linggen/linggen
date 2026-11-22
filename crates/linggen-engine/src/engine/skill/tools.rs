//! Running a skill's declared shell tool outside a model turn — the core
//! behind a page's door (`POST /api/skills/{skill}/tools/{tool}`) and the
//! companion's `AppTool`.
//!
//! Exactly as the model's call would run it: the same command template, PATH
//! and timeout (`SkillToolDef::execute`). On top of that:
//!
//! - one call per skill at a time (a per-skill lock), so two callers never
//!   interleave writes to the skill's files;
//! - the tool's `tier` must be within what the skill's `permission.paths`
//!   grants its working folder;
//! - a skill that keeps a cloud refuses while signed out (`AUTH_REQUIRED:`),
//!   like its turns, and a tool that may write pushes the save afterwards.
//!
//! The companion only reaches tools the skill offers her — `pet: true` to read,
//! `reach: anywhere` by its class ([`run_for_pet`]). Another device's agent reaches the tools
//! marked `remote: true` through the page's door ([`remote_tools`]).

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::engine::permission::{effective_mode_for_path, ActionClass, PathMode, PermissionMode};
use crate::engine::skill::judge::{judge, not_offered, Action, Asker};
use crate::engine::skill::reach::{Offer, Reach};
use crate::engine::skill::params::SessionParams;
use crate::engine::skill::Skill;
use crate::engine::skill_tool::{SkillToolDef, SkillToolKind};
use crate::engine::tools::ToolResult;

/// What a skill tool call answers when the skill keeps a cloud and nobody is
/// signed in — and what a skill turn answers in the same case.
pub const AUTH_REQUIRED: &str = "AUTH_REQUIRED: Sign in to linggen.dev to play.";

/// Why a declared tool did not run.
#[derive(Debug)]
pub enum Refusal {
    NoSkill(String),
    NoTool(String),
    NotRunnable(String),
    SignedOut,
    BeyondGrant(String),
    /// The tool's tier is above the level its caller stands at.
    BeyondLevel {
        level: PermissionMode,
        needs: PermissionMode,
    },
    NotForPet(String),
    /// The app is not one the asking session's person may use, or the
    /// session keeps it out.
    NotYours(String),
    /// A model's call the session's own gate refused (`judge`'s model
    /// asker), in the gate's words.
    ByGate(String),
    /// The action must be asked of the person, and the way in cannot ask.
    NeedsPerson(String),
    BadCall(String),
    Crashed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::SignedOut => f.write_str(AUTH_REQUIRED),
            Refusal::BeyondLevel { level, needs } => {
                write!(
                    f,
                    "beyond_level: the caller stands at {level}; the tool needs {needs}"
                )
            }
            Refusal::NoSkill(m)
            | Refusal::NoTool(m)
            | Refusal::NotRunnable(m)
            | Refusal::BeyondGrant(m)
            | Refusal::NotForPet(m)
            | Refusal::NotYours(m)
            | Refusal::ByGate(m)
            | Refusal::NeedsPerson(m)
            | Refusal::BadCall(m)
            | Refusal::Crashed(m) => f.write_str(m),
        }
    }
}

impl Refusal {
    pub fn no_skill(name: &str) -> Self {
        Refusal::NoSkill(format!("no skill '{name}'"))
    }
}

type SkillLock = Arc<tokio::sync::Mutex<()>>;

static SKILL_LOCKS: Mutex<Option<HashMap<String, SkillLock>>> = Mutex::new(None);

fn skill_lock(skill: &str) -> SkillLock {
    let mut guard = SKILL_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .entry(skill.to_string())
        .or_default()
        .clone()
}

/// Run `tool_name` of `skill` with `args` (an object), for a session bound
/// to `params` (the skill's session parameters; empty: none), as this
/// machine asks it.
pub async fn run(
    skill: &Skill,
    tool_name: &str,
    args: &Value,
    params: &SessionParams,
) -> Result<ToolResult, Refusal> {
    run_at(skill, tool_name, args, params, PermissionMode::Admin).await
}

/// [`run`] for a caller standing at `caller` (a device from off this
/// machine, through the page's door): the tool's tier must be within the
/// skill's grant and within `caller` ([`Refusal::BeyondLevel`]).
pub async fn run_at(
    skill: &Skill,
    tool_name: &str,
    args: &Value,
    params: &SessionParams,
    caller: PermissionMode,
) -> Result<ToolResult, Refusal> {
    let tool = shell_tool(skill, tool_name)?;
    if skill.cloud.is_some() && crate::account::resolve_token().is_none() {
        return Err(Refusal::SignedOut);
    }
    let cwd = working_folder(skill);
    let action = action_of(skill, &tool, &cwd);
    let person = Asker::Person {
        level: caller,
        granted: granted_on(skill, &cwd),
    };
    judge(person, &action).allowed_or_refusal(&action)?;

    let lock = skill_lock(&skill.name);
    let _one = lock.lock().await;
    let run_args = args.clone();
    let run_tool = tool.clone();
    let params = params.clone();
    let ran =
        tokio::task::spawn_blocking(move || run_tool.execute_bound(&run_args, &cwd, &[], &params))
            .await;
    let result = match ran {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(Refusal::BadCall(format!("{e:#}"))),
        Err(e) => return Err(Refusal::Crashed(format!("{e}"))),
    };
    if may_write(&tool) {
        push_save(skill).await;
    }
    Ok(result)
}

/// Stamp one of the skill's quests done at `at` through its declared writer
/// (`quests.stamp`) — the fact door. Runs like a page's tool: the skill's
/// working folder, its lock, an edit tier its grant must cover. `id` and
/// `at` are checked here and ride as quoted words. Ok only on exit 0.
pub async fn stamp_quest(skill: &Skill, id: &str, at: &str) -> Result<(), Refusal> {
    let tool = stamp_tool(skill, id, at)?;
    let cwd = working_folder(skill);
    within_grant(skill, &tool, &cwd)?;
    let lock = skill_lock(&skill.name);
    let _one = lock.lock().await;
    let args = serde_json::json!({ "id": id, "at": at });
    let ran = tokio::task::spawn_blocking(move || tool.execute(&args, &cwd, &[])).await;
    match ran {
        Ok(Ok(ToolResult::CommandOutput {
            exit_code: Some(0), ..
        })) => Ok(()),
        Ok(Ok(other)) => Err(Refusal::BadCall(format!(
            "'{}' quest stamp failed: {}",
            skill.name,
            output(other)
        ))),
        Ok(Err(e)) => Err(Refusal::BadCall(format!("{e:#}"))),
        Err(e) => Err(Refusal::Crashed(format!("{e}"))),
    }
}

/// Run a skill's `notices[].facts` command in its working folder and hand
/// back what it printed: the notice the line says, or nothing. A read: it
/// may look at the skill's data, never change it.
pub async fn notice_facts(skill: &Skill, cmd: &str) -> Result<String, Refusal> {
    let tool = SkillToolDef {
        name: "notices.facts".into(),
        description: "What a declared line would say.".into(),
        cmd: cmd.to_string(),
        endpoint: None,
        class: ActionClass::Read,
        args: Default::default(),
        returns: None,
        timeout_ms: STAMP_TIMEOUT_MS,
        max_output_bytes: 16 * 1024,
        page_only: true,
        others_read: Vec::new(),
        pet: false,
        remote: false,
        reach: Default::default(),
        skill_name: Some(skill.name.clone()),
        skill_dir: skill.skill_dir.clone(),
        senses: skill.senses.clone(),
        session_params: Default::default(),
    };
    let cwd = working_folder(skill);
    let mut env: Vec<(String, String)> = crate::reach::tool_env().into_iter().collect();
    if let Some(dir) = &skill.skill_dir {
        env.push(("SKILL_DIR".into(), dir.to_string_lossy().into_owned()));
    }
    let ran =
        tokio::task::spawn_blocking(move || tool.execute(&serde_json::json!({}), &cwd, &env)).await;
    match ran {
        Ok(Ok(ToolResult::CommandOutput {
            exit_code: Some(0),
            stdout,
            ..
        })) => Ok(stdout),
        Ok(Ok(other)) => Err(Refusal::BadCall(format!(
            "'{}' notice facts failed: {}",
            skill.name,
            output(other)
        ))),
        Ok(Err(e)) => Err(Refusal::BadCall(format!("{e:#}"))),
        Err(e) => Err(Refusal::Crashed(format!("{e}"))),
    }
}

const STAMP_TIMEOUT_MS: u64 = 10_000;

/// How long a `sync.on-list` run may take before its process group is killed.
const ON_LIST_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// Run the skill's `sync.on-list` command in its working folder: an edit
/// tier its grant must cover, `$SKILL_DIR` set. Not under the skill lock — a
/// run may take minutes, and pages and quest stamps must not wait on it; the
/// command keeps its own lock if it writes. Ok only on exit 0.
pub async fn run_on_list(skill: &Skill) -> Result<(), Refusal> {
    let Some(cmd) = skill.sync.as_ref().and_then(|s| s.on_list.clone()) else {
        return Err(Refusal::NoTool(format!(
            "'{}' declares no sync.on-list",
            skill.name
        )));
    };
    let tool = on_list_tool(skill, cmd);
    let cwd = working_folder(skill);
    within_grant(skill, &tool, &cwd)?;
    let mut env: Vec<(String, String)> = crate::reach::tool_env().into_iter().collect();
    if let Some(dir) = &skill.skill_dir {
        env.push(("SKILL_DIR".into(), dir.to_string_lossy().into_owned()));
    }
    let ran =
        tokio::task::spawn_blocking(move || tool.execute(&serde_json::json!({}), &cwd, &env)).await;
    match ran {
        Ok(Ok(ToolResult::CommandOutput {
            exit_code: Some(0), ..
        })) => Ok(()),
        Ok(Ok(other)) => Err(Refusal::BadCall(format!(
            "'{}' sync.on-list failed: {}",
            skill.name,
            output(other)
        ))),
        Ok(Err(e)) => Err(Refusal::BadCall(format!("{e:#}"))),
        Err(e) => Err(Refusal::Crashed(format!("{e}"))),
    }
}

/// The `sync.on-list` command as an argument-less shell tool.
fn on_list_tool(skill: &Skill, cmd: String) -> SkillToolDef {
    SkillToolDef {
        name: "sync.on-list".into(),
        description: "Bring the synced folder up to date.".into(),
        cmd,
        endpoint: None,
        class: ActionClass::Edit,
        args: Default::default(),
        returns: None,
        timeout_ms: ON_LIST_TIMEOUT_MS,
        max_output_bytes: 16 * 1024,
        page_only: true,
        others_read: Vec::new(),
        pet: false,
        remote: false,
        reach: Default::default(),
        skill_name: Some(skill.name.clone()),
        skill_dir: skill.skill_dir.clone(),
        senses: skill.senses.clone(),
        session_params: Default::default(),
    }
}

/// The skill's stamp command as a shell tool with two string arguments.
fn stamp_tool(skill: &Skill, id: &str, at: &str) -> Result<SkillToolDef, Refusal> {
    let Some(q) = skill.quests.as_ref() else {
        return Err(Refusal::NoTool(format!(
            "'{}' declares no quests",
            skill.name
        )));
    };
    if !q.stamp.contains("{id}") || !q.stamp.contains("{at}") {
        return Err(Refusal::NotRunnable(format!(
            "'{}' quests.stamp needs {{id}} and {{at}}",
            skill.name
        )));
    }
    if !valid_quest_id(id) || !valid_stamp_time(at) {
        return Err(Refusal::BadCall(format!("bad quest stamp {id:?} {at:?}")));
    }
    let arg = |name: &str| {
        let p = crate::engine::skill_tool::SkillParamDef {
            param_type: "string".into(),
            required: true,
            default: None,
            description: String::new(),
            items: None,
        };
        (name.to_string(), p)
    };
    Ok(SkillToolDef {
        name: "quests.stamp".into(),
        description: "Stamp a quest done.".into(),
        cmd: q.stamp.replace("{id}", "{{id}}").replace("{at}", "{{at}}"),
        endpoint: None,
        class: ActionClass::Edit,
        args: [arg("id"), arg("at")].into_iter().collect(),
        returns: None,
        timeout_ms: STAMP_TIMEOUT_MS,
        max_output_bytes: 4096,
        page_only: true,
        others_read: Vec::new(),
        pet: false,
        remote: false,
        reach: Default::default(),
        skill_name: Some(skill.name.clone()),
        skill_dir: skill.skill_dir.clone(),
        senses: skill.senses.clone(),
        session_params: Default::default(),
    })
}

/// A quest id: `[a-z0-9-]`, 1–64, no leading dash (never read as a flag).
pub fn valid_quest_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Exactly `YYYY-MM-DDTHH:MM:SSZ`, a real UTC time.
pub fn valid_stamp_time(at: &str) -> bool {
    at.len() == 20 && chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%SZ").is_ok()
}

/// The companion's door: what the skill offers her, judged by what it is
/// (`judge`'s companion), checked here, not left to the prompt.
/// `person_asked`: the turn is one the person started by asking her.
pub async fn run_for_pet(
    skill: &Skill,
    tool_name: &str,
    args: &Value,
    params: &SessionParams,
    person_asked: bool,
) -> Result<ToolResult, Refusal> {
    pet_gate(skill, tool_name, person_asked)?;
    run(skill, tool_name, args, params).await
}

fn pet_gate(skill: &Skill, tool_name: &str, person_asked: bool) -> Result<(), Refusal> {
    let tool = declared(skill, tool_name)?;
    let cwd = working_folder(skill);
    let action = action_of(skill, tool, &cwd);
    judge(companion_of(tool, person_asked), &action).allowed_or_refusal(&action)
}

/// The companion as asker of `tool`: what it is offered as, and whether the
/// person asked in this turn.
fn companion_of(tool: &SkillToolDef, person_asked: bool) -> Asker<'static> {
    Asker::Companion { offer: Offer::of(tool), person_asked }
}

/// The tools a skill offers the companion that she may actually call in a
/// turn the person did (`person_asked`) or did not start by asking her.
pub fn pet_tools(skill: &Skill, person_asked: bool) -> impl Iterator<Item = &SkillToolDef> {
    let cwd = working_folder(skill);
    skill.tool_defs.iter().filter(move |t| {
        t.kind() == SkillToolKind::Shell
            && judge(companion_of(t, person_asked), &action_of(skill, t, &cwd)).is_allow()
    })
}

/// The tools a skill offers any session's model outside its own
/// (`reach: anywhere`): shell tools of a skill a model may start.
pub fn anywhere_tools(skill: &Skill) -> impl Iterator<Item = &SkillToolDef> {
    skill.tool_defs.iter().filter(|t| {
        !skill.disable_model_invocation
            && t.reach == Reach::Anywhere
            && t.kind() == SkillToolKind::Shell
    })
}

/// An action of an app as a caller who asked for the app's listing reads
/// it: the same row wherever an app's actions are listed (`AppTool`, the
/// MCP door's `app`).
pub fn action_row(skill: &Skill, tool: &SkillToolDef) -> Value {
    let entry = tool.to_schema_json();
    serde_json::json!({
        "app": skill.name,
        "tool": tool.name,
        "class": tool.class.word(),
        "description": tool.description,
        "args": entry["args"],
    })
}

/// How much of an app's description its index line keeps.
const ABOUT_CHARS: usize = 90;

/// An app as an index lists it: its name, what it is for in one short
/// line, and the names of the actions it is shown with. Made here for the
/// prompt's index of apps (`prompt::app_index`) and the MCP door's `app`
/// listing alike.
pub struct AppLine {
    pub name: String,
    pub about: String,
    pub actions: Vec<String>,
}

/// `skill`'s index line, naming the actions in `shown`; none when it
/// shows none.
pub fn app_line<'a>(skill: &Skill, shown: impl Iterator<Item = &'a SkillToolDef>) -> Option<AppLine> {
    let actions: Vec<String> = shown.map(|t| t.name.clone()).collect();
    (!actions.is_empty()).then(|| AppLine {
        name: skill.name.clone(),
        about: one_line(&skill.description),
        actions,
    })
}

/// `text`'s first line, whitespace squeezed, cut to [`ABOUT_CHARS`].
fn one_line(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let squeezed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    match squeezed.chars().count() > ABOUT_CHARS {
        true => format!("{}…", squeezed.chars().take(ABOUT_CHARS).collect::<String>().trim_end()),
        false => squeezed,
    }
}

/// Whether a model may start `skill` at all: not an app only the person
/// starts (`disable-model-invocation: true`).
pub fn model_may_start(skill: &Skill) -> Result<(), Refusal> {
    match skill.disable_model_invocation {
        true => Err(Refusal::NotYours(format!(
            "'{}' is user-only: only the person starts it",
            skill.name
        ))),
        false => Ok(()),
    }
}

/// `skill`'s action `tool`, as a model outside the app's own session is
/// offered it, or why it is not: the one place such a call is resolved,
/// for a session's `AppTool` and the MCP door's `app` alike. `reachable`
/// is what the asker's own way in adds — a session's policy, what a day
/// keeps out, what the owner opened — asked after the action is found and
/// before it is judged offered.
pub fn resolve_offered<'a>(
    skill: &'a Skill,
    tool: &str,
    companion: bool,
    reachable: impl FnOnce(Offer) -> Result<(), Refusal>,
) -> Result<(&'a SkillToolDef, Offer), Refusal> {
    let def = declared(skill, tool)?;
    let offer = Offer::to_caller(def, companion);
    reachable(offer)?;
    if offer == Offer::Nobody {
        return Err(not_offered(&skill.name, tool));
    }
    if def.kind() != SkillToolKind::Shell {
        return Err(Refusal::NotRunnable(format!("'{tool}' runs no command")));
    }
    Ok((def, offer))
}

/// The tools a skill offers another device's agent (`remote: true`) that
/// would actually run when called: a shell tool (the door runs nothing
/// else), never `page_only` (the caller is a model), and within the skill's
/// own grant. Any tier — the caller gates writes as its own tiers say.
pub fn remote_tools(skill: &Skill) -> impl Iterator<Item = &SkillToolDef> {
    let cwd = working_folder(skill);
    skill.tool_defs.iter().filter(move |t| {
        t.remote
            && !t.page_only
            && t.kind() == SkillToolKind::Shell
            && within_grant(skill, t, &cwd).is_ok()
    })
}

pub(crate) fn declared<'a>(skill: &'a Skill, name: &str) -> Result<&'a SkillToolDef, Refusal> {
    skill
        .tool_defs
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| Refusal::NoTool(format!("'{}' declares no tool '{name}'", skill.name)))
}

/// The declared shell tool, or why there is none.
fn shell_tool(skill: &Skill, name: &str) -> Result<SkillToolDef, Refusal> {
    let tool = declared(skill, name)?;
    if tool.kind() != SkillToolKind::Shell {
        return Err(Refusal::NotRunnable(format!("'{name}' runs no command")));
    }
    let mut tool = tool.clone();
    tool.skill_name = Some(skill.name.clone());
    if tool.skill_dir.is_none() {
        tool.skill_dir = skill.skill_dir.clone();
    }
    Ok(tool)
}

/// Where the tool runs: the skill's declared `cwd`, else its own folder.
pub(crate) fn working_folder(skill: &Skill) -> PathBuf {
    let declared = skill
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match (declared, skill.skill_dir.as_ref()) {
        (Some(cwd), _) => crate::util::resolve_path(Path::new(cwd)),
        (None, Some(dir)) => dir.clone(),
        (None, None) => std::env::temp_dir(),
    }
}

fn may_write(tool: &SkillToolDef) -> bool {
    tool.class.changes_things()
}

/// This machine's own run of `tool` (a quest stamp, the sync command, the
/// list of what a device may call): the tool's class must be covered by
/// what the skill's own permission block grants the folder it runs in —
/// the scope its sessions get.
fn within_grant(skill: &Skill, tool: &SkillToolDef, cwd: &Path) -> Result<(), Refusal> {
    let action = action_of(skill, tool, cwd);
    let own = Asker::Person {
        level: PermissionMode::Admin,
        granted: granted_on(skill, cwd),
    };
    judge(own, &action).allowed_or_refusal(&action)
}

fn action_of<'a>(skill: &'a Skill, tool: &'a SkillToolDef, cwd: &'a Path) -> Action<'a> {
    Action {
        app: &skill.name,
        tool: &tool.name,
        class: tool.class,
        cwd,
    }
}

/// What the skill's own permission block grants the folder its tools run in.
fn granted_on(skill: &Skill, cwd: &Path) -> PermissionMode {
    let grants: Vec<PathMode> = skill
        .permission
        .iter()
        .flat_map(|p| p.iter_grants())
        .map(|(path, mode)| PathMode {
            path: crate::util::resolve_path(Path::new(path))
                .to_string_lossy()
                .to_string(),
            mode,
        })
        .collect();
    effective_mode_for_path(&grants, cwd).unwrap_or(PermissionMode::Chat)
}

/// A call changed the save: push it now, never pull (a pull happens at a
/// turn's edges and on the page's own sync).
async fn push_save(skill: &Skill) {
    let Some(save) = crate::account::cloud_save::target(skill) else {
        return;
    };
    tokio::spawn(async move {
        if let Err(e) = crate::account::cloud_save::push(&save).await {
            tracing::warn!("save '{}' not pushed after a page tool: {e:#}", save.skill);
        }
    });
}

/// A tool's result as the page door's JSON: `{exit_code, stdout, stderr}`.
pub fn output(result: ToolResult) -> Value {
    match result {
        ToolResult::CommandOutput {
            exit_code,
            stdout,
            stderr,
        } => serde_json::json!({ "exit_code": exit_code, "stdout": stdout, "stderr": stderr }),
        other => {
            serde_json::json!({ "exit_code": 0, "stdout": format!("{other:?}"), "stderr": "" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::skill::SkillSource;

    #[test]
    fn a_description_is_one_short_line() {
        assert_eq!(one_line("  Plays   music\non this Mac. "), "Plays music");
        assert_eq!(one_line(""), "");
        let long = "x".repeat(200);
        let cut = one_line(&long);
        assert_eq!(cut.chars().count(), ABOUT_CHARS + 1, "{cut}");
        assert!(cut.ends_with('…'));
        assert_eq!(one_line("日本語".repeat(40).as_str()).chars().count(), ABOUT_CHARS + 1);
    }

    const SKILL: &str = r#"---
name: zz-pet-test
description: test
permission:
  paths:
    - { path: "{DIR}", mode: edit }
tools:
  - name: Progress
    description: Where things stand.
    cmd: "echo progress {{who}}"
    tier: read
    pet: true
    args:
      who: { type: string }
  - name: Hidden
    description: Not for the pet.
    cmd: "echo hidden"
    tier: read
  - name: Write
    description: Changes things.
    cmd: "echo wrote"
    tier: edit
    pet: true
  - name: Untiered
    description: No tier, so admin.
    cmd: "echo untiered"
    pet: true
  - name: ListThings
    description: Remote read.
    cmd: "echo things"
    tier: read
    remote: true
  - name: RenameThing
    description: Remote edit.
    cmd: "echo renamed"
    tier: edit
    remote: true
  - name: FetchThing
    description: The user's tap only.
    cmd: "echo fetched"
    tier: edit
    page_only: true
    remote: true
  - name: ShowThing
    description: Data tool, runs nothing.
    tier: read
    remote: true
  - name: WipeAll
    description: Beyond the edit grant.
    cmd: "echo wiped"
    tier: admin
    remote: true
---
body
"#;

    fn skill(dir: &Path) -> Skill {
        let text = SKILL.replace("{DIR}", &dir.to_string_lossy());
        let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project)
            .expect("parses");
        s.skill_dir = Some(dir.to_path_buf());
        s
    }

    fn args() -> Value {
        serde_json::json!({ "who": "alex" })
    }

    /// A page's call from a device is judged by the lower of the skill's
    /// grant and the caller's level: a read tool runs at read; an edit tool
    /// is refused at read, with the level and what it needs, and runs at
    /// edit; this machine's own call (`run`) is unchanged.
    #[tokio::test]
    async fn a_tool_is_judged_by_the_callers_level_too() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let p = SessionParams::new();
        assert!(run_at(&s, "Hidden", &args(), &p, PermissionMode::Read)
            .await
            .is_ok());
        let err = run_at(&s, "Write", &args(), &p, PermissionMode::Read)
            .await
            .unwrap_err();
        match err {
            Refusal::BeyondLevel { level, needs } => {
                assert_eq!((level, needs), (PermissionMode::Read, PermissionMode::Edit));
            }
            other => panic!("not refused by level: {other}"),
        }
        assert!(run_at(&s, "Write", &args(), &p, PermissionMode::Edit)
            .await
            .is_ok());
        assert!(run_at(&s, "Hidden", &args(), &p, PermissionMode::Chat)
            .await
            .is_err());
        assert!(
            run(&s, "Write", &args(), &p).await.is_ok(),
            "this machine's call"
        );
        let beyond = run_at(&s, "WipeAll", &args(), &p, PermissionMode::Admin)
            .await
            .unwrap_err();
        assert!(
            matches!(beyond, Refusal::BeyondGrant(_)),
            "the skill's own grant still holds"
        );
    }

    #[tokio::test]
    async fn a_pet_tool_runs() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let result = run_for_pet(&s, "Progress", &args(), &SessionParams::new(), false)
            .await
            .expect("runs");
        let out = output(result);
        assert_eq!(out["exit_code"], 0);
        assert_eq!(out["stdout"], "progress alex\n");
    }

    #[tokio::test]
    async fn a_tool_not_offered_to_the_pet_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let err = run_for_pet(&s, "Hidden", &args(), &SessionParams::new(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, Refusal::NotForPet(_)), "{err}");
        // The page door still runs it.
        assert!(run(&s, "Hidden", &args(), &SessionParams::new())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_pet_tool_above_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        for name in ["Write", "Untiered"] {
            let err = run_for_pet(&s, name, &args(), &SessionParams::new(), false)
                .await
                .unwrap_err();
            assert!(matches!(err, Refusal::NotForPet(_)), "{name}: {err}");
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let err = run_for_pet(&s, "Nope", &args(), &SessionParams::new(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, Refusal::NoTool(_)), "{err}");
        assert_eq!(err.to_string(), "'zz-pet-test' declares no tool 'Nope'");
    }

    fn stamper(dir: &Path, stamp: &str) -> Skill {
        let text = format!(
            "---\nname: zz-stamp-test\ndescription: t\npermission:\n  paths:\n    - {{ path: \"{}\", mode: edit }}\nquests:\n  stamp: \"{stamp}\"\n  facts:\n    zz-seen: zz-quest\n---\nbody\n",
            dir.to_string_lossy()
        );
        let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project)
            .expect("parses");
        s.skill_dir = Some(dir.to_path_buf());
        s
    }

    #[tokio::test]
    async fn a_quest_stamp_runs_the_declared_writer_with_quoted_words() {
        let dir = tempfile::tempdir().unwrap();
        let s = stamper(dir.path(), "printf '%s|%s' {id} {at} > out.txt");
        stamp_quest(&s, "zz-quest", "2026-09-24T14:03:11Z")
            .await
            .expect("stamped");
        let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(out, "zz-quest|2026-09-24T14:03:11Z");
    }

    #[tokio::test]
    async fn a_quest_stamp_refuses_bad_words_and_reports_a_failed_writer() {
        let dir = tempfile::tempdir().unwrap();
        let s = stamper(dir.path(), "touch ran; exit 1 # {id} {at}");
        for (id, at) in [
            ("zz;rm", "2026-09-24T14:03:11Z"),
            ("-rf", "2026-09-24T14:03:11Z"),
            ("zz-quest", "2026-09-24 14:03:11"),
            ("zz-quest", "2026-09-24T14:03:11.000Z"),
            ("zz-quest", "2026-13-24T14:03:11Z"),
        ] {
            assert!(stamp_quest(&s, id, at).await.is_err(), "{id} {at}");
        }
        assert!(!dir.path().join("ran").exists(), "nothing ran");
        let err = stamp_quest(&s, "zz-quest", "2026-09-24T14:03:11Z").await;
        assert!(err.is_err(), "exit 1 is a failed stamp");
        assert!(dir.path().join("ran").exists());

        let no_at = stamper(dir.path(), "true {id}");
        assert!(matches!(
            stamp_quest(&no_at, "zz-quest", "2026-09-24T14:03:11Z").await,
            Err(Refusal::NotRunnable(_))
        ));
    }

    #[test]
    fn a_quest_stamp_carries_the_skills_senses() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = stamper(dir.path(), "true {id} {at}");
        s.senses = vec!["weather".into()];
        let tool = stamp_tool(&s, "zz-quest", "2026-09-24T14:03:11Z").unwrap();
        assert_eq!(tool.senses, s.senses);
    }

    #[test]
    fn only_runnable_read_pet_tools_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let names: Vec<_> = pet_tools(&s, false).map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Progress"]);
    }

    #[test]
    fn only_runnable_model_facing_remote_tools_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let names: Vec<_> = remote_tools(&s).map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["ListThings", "RenameThing"]);
    }

    #[tokio::test]
    async fn a_remote_tool_runs_through_the_page_door() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let out = output(
            run(&s, "RenameThing", &args(), &SessionParams::new())
                .await
                .expect("runs"),
        );
        assert_eq!(out["stdout"], "renamed\n");
    }
}
