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
//! The companion only reaches tools the skill marks `pet: true` whose own
//! tier is read ([`run_for_pet`]).

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::engine::permission::{
    effective_mode_for_path, parse_skill_tier, PathMode, PermissionMode,
};
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
    NotForPet(String),
    BadCall(String),
    Crashed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::SignedOut => f.write_str(AUTH_REQUIRED),
            Refusal::NoSkill(m)
            | Refusal::NoTool(m)
            | Refusal::NotRunnable(m)
            | Refusal::BeyondGrant(m)
            | Refusal::NotForPet(m)
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

/// Run `tool_name` of `skill` with `args` (an object).
pub async fn run(skill: &Skill, tool_name: &str, args: &Value) -> Result<ToolResult, Refusal> {
    let tool = shell_tool(skill, tool_name)?;
    if skill.cloud.is_some() && crate::account::resolve_token().is_none() {
        return Err(Refusal::SignedOut);
    }
    let cwd = working_folder(skill);
    within_grant(skill, &tool, &cwd)?;

    let lock = skill_lock(&skill.name);
    let _one = lock.lock().await;
    let run_args = args.clone();
    let run_tool = tool.clone();
    let ran = tokio::task::spawn_blocking(move || run_tool.execute(&run_args, &cwd, &[])).await;
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

const STAMP_TIMEOUT_MS: u64 = 10_000;

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
        tier: Some("edit".into()),
        args: [arg("id"), arg("at")].into_iter().collect(),
        returns: None,
        timeout_ms: STAMP_TIMEOUT_MS,
        max_output_bytes: 4096,
        page_only: true,
        pet: false,
        skill_name: Some(skill.name.clone()),
        skill_dir: skill.skill_dir.clone(),
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

/// The companion's door: only a tool the skill offers her (`pet: true`)
/// whose own tier is read — checked here, not left to the prompt.
pub async fn run_for_pet(
    skill: &Skill,
    tool_name: &str,
    args: &Value,
) -> Result<ToolResult, Refusal> {
    pet_gate(skill, tool_name)?;
    run(skill, tool_name, args).await
}

fn pet_gate(skill: &Skill, tool_name: &str) -> Result<(), Refusal> {
    let tool = declared(skill, tool_name)?;
    if !tool.pet {
        return Err(Refusal::NotForPet(format!(
            "'{}' does not offer '{tool_name}' to you",
            skill.name
        )));
    }
    if tier_of(tool) > PermissionMode::Read {
        return Err(Refusal::NotForPet(format!(
            "'{tool_name}' may change '{}'; you may only read",
            skill.name
        )));
    }
    Ok(())
}

/// The tools a skill offers the companion that she may actually call.
pub fn pet_tools(skill: &Skill) -> impl Iterator<Item = &SkillToolDef> {
    skill
        .tool_defs
        .iter()
        .filter(|t| t.pet && t.kind() == SkillToolKind::Shell && tier_of(t) <= PermissionMode::Read)
}

fn declared<'a>(skill: &'a Skill, name: &str) -> Result<&'a SkillToolDef, Refusal> {
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
fn working_folder(skill: &Skill) -> PathBuf {
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

fn tier_of(tool: &SkillToolDef) -> PermissionMode {
    tool.tier
        .as_deref()
        .and_then(parse_skill_tier)
        .unwrap_or(PermissionMode::Admin)
}

fn may_write(tool: &SkillToolDef) -> bool {
    tier_of(tool) > PermissionMode::Read
}

/// The tool's tier must be covered by what the skill's own permission block
/// grants the folder it runs in — the scope its sessions get.
fn within_grant(skill: &Skill, tool: &SkillToolDef, cwd: &Path) -> Result<(), Refusal> {
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
    let granted = effective_mode_for_path(&grants, cwd).unwrap_or(PermissionMode::Chat);
    let needed = tier_of(tool);
    if needed > granted {
        return Err(Refusal::BeyondGrant(format!(
            "'{}' needs {needed} on {}; the skill grants {granted}",
            tool.name,
            cwd.display()
        )));
    }
    Ok(())
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

    #[tokio::test]
    async fn a_pet_tool_runs() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let result = run_for_pet(&s, "Progress", &args()).await.expect("runs");
        let out = output(result);
        assert_eq!(out["exit_code"], 0);
        assert_eq!(out["stdout"], "progress alex\n");
    }

    #[tokio::test]
    async fn a_tool_not_offered_to_the_pet_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let err = run_for_pet(&s, "Hidden", &args()).await.unwrap_err();
        assert!(matches!(err, Refusal::NotForPet(_)), "{err}");
        // The page door still runs it.
        assert!(run(&s, "Hidden", &args()).await.is_ok());
    }

    #[tokio::test]
    async fn a_pet_tool_above_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        for name in ["Write", "Untiered"] {
            let err = run_for_pet(&s, name, &args()).await.unwrap_err();
            assert!(matches!(err, Refusal::NotForPet(_)), "{name}: {err}");
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let err = run_for_pet(&s, "Nope", &args()).await.unwrap_err();
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
    fn only_runnable_read_pet_tools_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill(dir.path());
        let names: Vec<_> = pet_tools(&s).map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Progress"]);
    }
}
