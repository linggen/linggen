use super::types::*;
use tracing::info;

impl AgentEngine {
    /// Called when the model signals plan completion (via ExitPlanMode tool or
    /// fallback: done in plan_mode). Emits a PlanUpdate event so the
    /// PlanBlock renders in the UI, and returns `AgentOutcome::Plan` for the
    /// server to store as pending. The user reviews and approves via PlanBlock
    /// buttons (CC-aligned — no modal AskUser dialog).
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        // Preserve items from any prior UpdatePlan call during plan mode.
        // If no items exist (model didn't call UpdatePlan), auto-extract from
        // numbered headings/steps in the plan text.
        let mut items = self
            .plan
            .as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        if items.is_empty() {
            items = Self::extract_plan_items(&plan_text);
            if !items.is_empty() {
                info!(
                    "[plan] Auto-extracted {} plan items from headings",
                    items.len()
                );
            }
        }
        // If UpdatePlan already set items but the plan_text from ExitPlanMode is
        // very short (e.g. DeepSeek puts the real plan in <think> which gets
        // stripped), rebuild plan_text from the items so the PlanBlock shows
        // meaningful content instead of a stub.
        let plan_text = if !items.is_empty() && plan_text.len() < 100 {
            let existing_plan_text = self
                .plan
                .as_ref()
                .map(|p| p.plan_text.clone())
                .filter(|t| t.len() > plan_text.len());
            existing_plan_text.unwrap_or_else(|| {
                let mut lines = Vec::new();
                if !plan_text.trim().is_empty() {
                    lines.push(format!("# {}", plan_text.trim()));
                    lines.push(String::new());
                }
                for item in &items {
                    lines.push(format!("- [ ] {}", item.title));
                }
                info!(
                    "[plan] Rebuilt plan_text from {} items (original was {} chars)",
                    items.len(),
                    plan_text.len()
                );
                lines.join("\n")
            })
        } else {
            plan_text
        };
        let summary = Self::extract_plan_summary(&plan_text);
        let plan = Plan {
            summary,
            status: PlanStatus::Planned,
            plan_text,
            items,
        };
        info!(
            "[plan] finalize_plan_mode: status={:?} items={}",
            plan.status,
            plan.items.len()
        );
        self.persist_and_emit_plan(plan.clone()).await;
        AgentOutcome::Plan(plan)
    }

    /// The model replied in plan mode without calling ExitPlanMode: the
    /// reply is an answer, not a plan (CC-aligned). Leave plan mode so the
    /// text is shown and saved like any reply, and drop an unsubmitted draft.
    pub(crate) fn leave_plan_mode_for_answer(&mut self) {
        if !self.plan_mode {
            return;
        }
        info!("[plan] reply without ExitPlanMode → leaving plan mode, delivering as answer");
        self.plan_mode = false;
        if self
            .plan
            .as_ref()
            .is_some_and(|p| p.status == PlanStatus::Planned)
        {
            self.plan = None;
        }
    }

    /// Store the plan in memory and emit a PlanUpdate event.
    /// Only the session's own agent shows a plan; a delegated run does not.
    fn owns_session_plan(&self) -> bool {
        self.tools.builtins.delegation_depth() == 0
    }

    pub(crate) async fn persist_and_emit_plan(&mut self, plan: Plan) {
        info!(
            "[plan] persist_and_emit_plan: status={:?} items={}",
            plan.status,
            plan.items.len()
        );
        self.plan = Some(plan);

        // The session's plan block and checklist are the main agent's. A
        // subagent's progress stays in its own run, as a Claude Code
        // subagent's todos do; its UpdatePlan call still shows in its pane.
        if !self.owns_session_plan() {
            return;
        }
        if self.tools.get_manager().is_some() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let plan = self.plan.clone().unwrap();
            self.emit(crate::engine::agent::AgentEvent::PlanUpdate { agent_id, plan })
                .await;
        }
    }

    /// The checklist a submitted plan starts with: its numbered headings
    /// ("### 1. Title", "### Step 2: Title"), or else the top-level list
    /// items under its Steps heading ("## Steps" then "1. Title" / "- Title").
    /// Other sections (Files, Verification) never become items.
    pub(crate) fn extract_plan_items(text: &str) -> Vec<PlanItem> {
        let mut titles = Self::numbered_heading_titles(text);
        if titles.is_empty() {
            titles = Self::steps_section_titles(text);
        }
        titles
            .into_iter()
            .enumerate()
            .map(|(i, title)| PlanItem {
                id: (i + 1).to_string(),
                title,
                status: "pending".to_string(),
                active_form: None,
            })
            .collect()
    }

    fn numbered_heading_titles(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| Self::heading_text(line.trim()))
            .filter_map(Self::extract_step_title)
            .collect()
    }

    fn steps_section_titles(text: &str) -> Vec<String> {
        let mut in_steps = false;
        let mut titles = Vec::new();
        for line in text.lines() {
            if let Some(heading) = Self::heading_text(line.trim()) {
                let heading = heading.to_lowercase();
                in_steps = heading.contains("step") || heading.contains("步骤");
                continue;
            }
            // Top-level items only: nested bullets are a step's details.
            if !in_steps || line.starts_with([' ', '\t']) {
                continue;
            }
            if let Some(title) = Self::list_item_title(line) {
                titles.push(title);
            }
        }
        titles
    }

    /// "## Title" → "Title"; None for a line that is not a heading.
    fn heading_text(line: &str) -> Option<&str> {
        if !line.starts_with('#') {
            return None;
        }
        Some(line.trim_start_matches('#').trim())
    }

    /// "- Title", "* Title", "1. Title", "1) Title" → "Title".
    fn list_item_title(line: &str) -> Option<String> {
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            return Self::clean_title(rest);
        }
        Self::extract_step_title(line)
    }

    /// "1. Foo", "2) Foo", "Step 3: Foo" → "Foo".
    fn extract_step_title(text: &str) -> Option<String> {
        let s = text.strip_prefix("Step").unwrap_or(text).trim();
        let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        let rest = &s[digits..];
        let rest = rest.strip_prefix(['.', ':', ')'])?;
        Self::clean_title(rest)
    }

    /// Drop bold markers and cap the length (by characters: titles may be CJK).
    fn clean_title(text: &str) -> Option<String> {
        let title = text.replace("**", "").trim().to_string();
        if title.is_empty() {
            return None;
        }
        if title.chars().count() > 120 {
            return Some(format!(
                "{}...",
                title.chars().take(117).collect::<String>()
            ));
        }
        Some(title)
    }

    /// Extract a summary from the plan text (first heading or first non-empty line).
    pub(crate) fn extract_plan_summary(text: &str) -> String {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                return trimmed.strip_prefix("# ").unwrap_or(trimmed).to_string();
            }
            if !trimmed.is_empty() {
                return trimmed.chars().take(80).collect();
            }
        }
        "Plan".to_string()
    }
}

impl PlanItem {
    /// One checklist item from a tool call's JSON. A null or blank
    /// `active_form` (OpenAI strict mode sends null) is no active form.
    pub(crate) fn from_json(item: &serde_json::Value) -> Self {
        let text = |key: &str| item.get(key).and_then(|v| v.as_str());
        PlanItem {
            id: text("id").unwrap_or("").to_string(),
            title: text("title").unwrap_or("?").to_string(),
            status: text("status").unwrap_or("pending").to_string(),
            active_form: text("active_form")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }
    }
}

impl Plan {
    /// The person edited the plan before approving it: its summary and
    /// checklist follow the new text (a text with no steps keeps the old list).
    pub(crate) fn set_plan_text(&mut self, text: &str) {
        self.plan_text = text.to_string();
        self.summary = AgentEngine::extract_plan_summary(text);
        let items = AgentEngine::extract_plan_items(text);
        if !items.is_empty() {
            self.items = items;
        }
    }

    /// The checklist as the execution prompt shows it, one item per line:
    /// `- id 2 [in_progress] Run tests`.
    pub(crate) fn checklist_text(&self) -> String {
        if self.items.is_empty() {
            return "(none yet — create it with UpdatePlan from the plan's steps)".to_string();
        }
        self.items
            .iter()
            .map(|i| format!("- id {} [{}] {}", i.id, i.status, i.title))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_session_agent_owns_the_plan_block() {
        let root = tempfile::tempdir().unwrap();
        let cfg = crate::engine::EngineConfig::from_app_config(
            &crate::config::Config::default(),
            root.path().into(),
            crate::engine::InterfaceMode::Web,
        );
        let models =
            std::sync::Arc::new(crate::provider::models::ModelManager::new_with_credentials(
                vec![],
                &crate::credentials::Credentials::default(),
            ));
        let mut engine =
            AgentEngine::new(cfg, models, "m".into(), crate::engine::AgentRole::Lead).unwrap();
        assert!(engine.owns_session_plan());
        engine.set_delegation_depth(1, 2);
        assert!(!engine.owns_session_plan());
    }

    #[test]
    fn extract_plan_summary_examples() {
        assert_eq!(
            AgentEngine::extract_plan_summary("# My Plan\n\nSome details"),
            "My Plan"
        );
        assert_eq!(
            AgentEngine::extract_plan_summary("First line without heading"),
            "First line without heading"
        );
        assert_eq!(AgentEngine::extract_plan_summary(""), "Plan");
        assert_eq!(AgentEngine::extract_plan_summary("   \n  "), "Plan");
    }

    #[test]
    fn extract_plan_items_numbered_headings() {
        let text = r#"# Improve the UI

## Summary
Make it look better.

### 1. Update the header
Change colors and layout.

### 2. Fix the sidebar
Adjust widths.

### 3. Polish the footer
Add links.
"#;
        let items = AgentEngine::extract_plan_items(text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "Update the header");
        assert_eq!(items[1].title, "Fix the sidebar");
        assert_eq!(items[2].title, "Polish the footer");
        assert!(items.iter().all(|i| i.status == "pending"));
    }

    #[test]
    fn extract_plan_items_step_prefix() {
        let text = "### Step 1: Read the file\n### Step 2: Edit the code\n";
        let items = AgentEngine::extract_plan_items(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Read the file");
        assert_eq!(items[1].title, "Edit the code");
    }

    #[test]
    fn extract_plan_items_no_steps() {
        let text = "# Plan\n\nJust some text without numbered steps.\n\n## Risks\nNone.";
        let items = AgentEngine::extract_plan_items(text);
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn extract_step_title_variants() {
        assert_eq!(
            AgentEngine::extract_step_title("1. Foo bar"),
            Some("Foo bar".into())
        );
        assert_eq!(
            AgentEngine::extract_step_title("Step 3: Baz"),
            Some("Baz".into())
        );
        assert_eq!(
            AgentEngine::extract_step_title("12. Long step"),
            Some("Long step".into())
        );
        assert_eq!(AgentEngine::extract_step_title("No number"), None);
        assert_eq!(AgentEngine::extract_step_title("1."), None);
    }

    /// The exact shape plan-mode.toml asks for: only the Steps become items.
    #[test]
    fn extract_plan_items_from_the_steps_section() {
        let text = r#"# Add active form to todos

The spinner should say what runs.

## Files
- `src/engine/types.rs` — add the field
- `ui/src/lib/runStatus.mts` — reuse `spinnerLabel`

## Steps
1. Add `active_form` to **PlanItem**
   - keep old plans loading
2) Show it in the status line
- Update the docs

## Verification
- Run `cargo test --bin ling plan`
1. Open a session and watch the spinner
"#;
        let items = AgentEngine::extract_plan_items(text);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Add `active_form` to PlanItem",
                "Show it in the status line",
                "Update the docs"
            ]
        );
        assert_eq!(items[2].id, "3");
        assert!(items
            .iter()
            .all(|i| i.status == "pending" && i.active_form.is_none()));
    }

    #[test]
    fn extract_plan_items_caps_long_cjk_titles_without_panicking() {
        let long = "修改".repeat(80);
        let items = AgentEngine::extract_plan_items(&format!("## 步骤\n1. {long}\n"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title.chars().count(), 120);
        assert!(items[0].title.ends_with("..."));
    }

    #[test]
    fn active_form_round_trips_and_old_plans_load() {
        let old = r#"{"summary":"S","status":"executing","plan_text":"","items":[{"id":"1","title":"Run tests","status":"pending"}]}"#;
        let plan: Plan = serde_json::from_str(old).unwrap();
        assert_eq!(plan.items[0].active_form, None);
        // An item without an active form saves as it always did.
        assert!(!serde_json::to_string(&plan)
            .unwrap()
            .contains("active_form"));

        let mut plan = plan;
        plan.items[0].active_form = Some("Running tests".into());
        let back: Plan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(back.items[0].active_form.as_deref(), Some("Running tests"));
    }

    #[test]
    fn plan_item_from_json_reads_active_form_and_treats_null_as_none() {
        let item = PlanItem::from_json(&serde_json::json!({
            "id": "2", "title": "Run tests", "status": "in_progress", "active_form": "Running tests"
        }));
        assert_eq!(item.active_form.as_deref(), Some("Running tests"));
        let strict = PlanItem::from_json(&serde_json::json!({
            "id": "2", "title": "Run tests", "status": "pending", "active_form": null
        }));
        assert_eq!(strict.active_form, None);
        assert_eq!(
            PlanItem::from_json(&serde_json::json!({})).status,
            "pending"
        );
    }

    #[test]
    fn editing_a_plan_rebuilds_its_checklist() {
        let mut plan = Plan {
            summary: "Old".into(),
            status: PlanStatus::Planned,
            plan_text: String::new(),
            items: AgentEngine::extract_plan_items("## Steps\n1. Old step\n"),
        };
        plan.set_plan_text("# New\n\n## Steps\n1. First\n2. Second\n");
        assert_eq!(plan.summary, "New");
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[1].title, "Second");
        // A text without steps keeps the list it had.
        plan.set_plan_text("# Just prose");
        assert_eq!(plan.items.len(), 2);
        assert!(plan.checklist_text().contains("- id 2 [pending] Second"));
    }
}
