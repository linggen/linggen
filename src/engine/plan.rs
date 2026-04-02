use super::types::*;
use tracing::info;

impl AgentEngine {
    /// Called when the model signals plan completion (via ExitPlanMode tool or
    /// fallback: done in plan_mode). Emits a PlanUpdate SSE event so the
    /// PlanBlock renders in the UI, and returns `AgentOutcome::Plan` for the
    /// server to store as pending. The user reviews and approves via PlanBlock
    /// buttons (CC-aligned — no modal AskUser dialog).
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        // Preserve items from any prior UpdatePlan call during plan mode.
        // If no items exist (model didn't call UpdatePlan), auto-extract from
        // numbered headings/steps in the plan text.
        let mut items = self.plan.as_ref()
            .map(|p| p.items.clone())
            .unwrap_or_default();
        if items.is_empty() {
            items = Self::extract_plan_items(&plan_text);
            if !items.is_empty() {
                info!("[plan] Auto-extracted {} plan items from headings", items.len());
            }
        }
        // If UpdatePlan already set items but the plan_text from ExitPlanMode is
        // very short (e.g. DeepSeek puts the real plan in <think> which gets
        // stripped), rebuild plan_text from the items so the PlanBlock shows
        // meaningful content instead of a stub.
        let plan_text = if !items.is_empty() && plan_text.len() < 100 {
            let existing_plan_text = self.plan.as_ref()
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
                info!("[plan] Rebuilt plan_text from {} items (original was {} chars)", items.len(), plan_text.len());
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
        info!("[plan] finalize_plan_mode: status={:?} items={}", plan.status, plan.items.len());
        self.persist_and_emit_plan(plan.clone()).await;
        AgentOutcome::Plan(plan)
    }

    /// Store the plan in memory and emit a PlanUpdate SSE event.
    pub(crate) async fn persist_and_emit_plan(&mut self, plan: Plan) {
        info!("[plan] persist_and_emit_plan: status={:?} items={}", plan.status, plan.items.len());
        self.plan = Some(plan);

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let plan = self.plan.clone().unwrap();
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id,
                    plan,
                }, self.session_id.clone())
                .await;
        }
    }

    /// Extract todo items from plan text by finding numbered headings or steps.
    /// Matches patterns like "### 1. Do something", "### Step 1: Do something",
    /// "## 2. Something else", or numbered list items "1. Do this".
    pub(crate) fn extract_plan_items(text: &str) -> Vec<PlanItem> {
        let mut items = Vec::new();
        let mut counter = 1u32;
        for line in text.lines() {
            let trimmed = line.trim();
            // Match "### 1. Title", "### Step 1: Title", "## 2. Title"
            if let Some(rest) = trimmed.strip_prefix("###").or_else(|| trimmed.strip_prefix("##")) {
                let rest = rest.trim();
                // "Step N: Title" or "Step N. Title" or "N. Title" or "N: Title"
                let title = Self::extract_step_title(rest);
                if let Some(title) = title {
                    items.push(PlanItem {
                        id: counter.to_string(),
                        title,
                        status: "pending".to_string(),
                    });
                    counter += 1;
                }
            }
        }
        items
    }

    /// Try to extract a step title from text like "1. Foo", "Step 2: Foo", etc.
    fn extract_step_title(text: &str) -> Option<String> {
        let s = text.strip_prefix("Step").or(Some(text)).unwrap().trim();
        // Match leading number followed by . or :
        let mut chars = s.chars().peekable();
        if !chars.peek().map_or(false, |c| c.is_ascii_digit()) {
            return None;
        }
        // Skip digits
        while chars.peek().map_or(false, |c| c.is_ascii_digit()) {
            chars.next();
        }
        // Expect . or :
        match chars.peek() {
            Some('.') | Some(':') => { chars.next(); }
            _ => return None,
        }
        let title: String = chars.collect::<String>().trim().to_string();
        if title.is_empty() { return None; }
        // Truncate overly long titles
        if title.len() > 120 {
            Some(format!("{}...", &title[..117]))
        } else {
            Some(title)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(AgentEngine::extract_step_title("1. Foo bar"), Some("Foo bar".into()));
        assert_eq!(AgentEngine::extract_step_title("Step 3: Baz"), Some("Baz".into()));
        assert_eq!(AgentEngine::extract_step_title("12. Long step"), Some("Long step".into()));
        assert_eq!(AgentEngine::extract_step_title("No number"), None);
        assert_eq!(AgentEngine::extract_step_title("1."), None);
    }
}
