//! Widget transport only. The SDK decides when permission is needed; no
//! native tool executor, permission tiers or path grants are consulted.
use crate::engine::tools::{AskUserAnswer, AskUserOption, AskUserQuestion, PendingAskUser};
use crate::server::ServerEvent;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) struct Interaction {
    pub agent_id: String,
    pub state: Arc<crate::server::ServerState>,
    pub meta: crate::state_fs::sessions::SessionMeta,
    pub run_id: String,
    /// Unguessable widget id; the host maps the SDK's request id onto it.
    pub question_id: String,
}

/// A fresh widget id for one SDK request.
pub(super) fn new_question_id() -> String {
    format!("sdk-{}", uuid::Uuid::new_v4())
}

impl Interaction {
    pub async fn request(&self, event: &Value) -> Result<Value> {
        let input = &event["input"];
        let question_id = self.question_id.clone();
        match event["kind"].as_str() {
            Some("permission_updates") => {
                let updates = input["updates"]
                    .as_array()
                    .context("Missing SDK permission updates")?;
                self.state.claude.add_session_rules(&self.meta.id, updates);
                Ok(json!({"saved":true}))
            }
            Some("permission") => {
                let tool = input["tool"]
                    .as_str()
                    .context("Permission request has no tool")?;
                let args = &input["args"];
                let target = args["command"]
                    .as_str()
                    .or(args["file_path"].as_str())
                    .or(args["notebook_path"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| args.to_string());
                let mut options = vec![option("Allow once")];
                if input["suggestions"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["type"] != "setMode"))
                {
                    options.push(option("Allow for this session"));
                }
                if self.auto_available().await {
                    options.push(option("Allow and switch to auto"));
                }
                for suggestion in input["suggestions"].as_array().into_iter().flatten() {
                    if suggestion["type"] == "setMode" {
                        if let Some(mode) =
                            suggestion["mode"].as_str().filter(|mode| *mode != "auto")
                        {
                            if serde_json::from_value::<
                                crate::state_fs::sessions::ClaudePermissionMode,
                            >(json!(mode))
                            .is_ok()
                            {
                                options.push(option(&format!("Allow and switch to {mode}")));
                            }
                        }
                    }
                }
                options.push(option("Deny"));
                let question = AskUserQuestion {
                    question: input["title"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{tool} {target}")),
                    header: "Permission".into(),
                    multi_select: false,
                    options,
                };
                let answers = self.ask(vec![question], question_id).await?;
                let answer = answers.first();
                let selected = answer
                    .and_then(|a| a.selected.first())
                    .map(String::as_str)
                    .unwrap_or_default();
                let chosen_mode = selected
                    .strip_prefix("Allow and switch to ")
                    .filter(|mode| {
                        serde_json::from_value::<crate::state_fs::sessions::ClaudePermissionMode>(
                            json!(mode),
                        )
                        .is_ok()
                    });
                let allow = answer.is_some_and(|a| {
                    (matches!(selected, "Allow once" | "Allow for this session")
                        || chosen_mode.is_some())
                        && a.custom_text.as_deref().unwrap_or_default().is_empty()
                });
                let reason = answer
                    .and_then(|a| a.custom_text.as_deref())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("User declined this tool call.");
                Ok(json!({"allow":allow, "reason":reason,
                    "remember": allow && selected == "Allow for this session",
                    "mode": if allow { chosen_mode } else { None }}))
            }
            Some("ask_user") => {
                let mut input = input.clone();
                if let Some(questions) = input["questions"].as_array_mut() {
                    for question in questions {
                        question["multi_select"] =
                            json!(question["multiSelect"].as_bool().unwrap_or(false));
                    }
                }
                let questions: Vec<AskUserQuestion> =
                    serde_json::from_value(input["questions"].clone())?;
                let answers = self.ask(questions, question_id).await?;
                let mut mapped = serde_json::Map::new();
                for answer in answers {
                    if let Some(question) =
                        input["questions"][answer.question_index]["question"].as_str()
                    {
                        let text = answer
                            .custom_text
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| answer.selected.join(", "));
                        mapped.insert(question.into(), json!(text));
                    }
                }
                Ok(json!({"answers":mapped}))
            }
            Some("image") => super::media::keep(
                &self.state.manager.global_sessions,
                &self.meta.id,
                input["base64"].as_str().context("Missing image data")?,
                input["caption"].as_str().unwrap_or("Image"),
            ),
            Some("display_image") => {
                super::media::display(
                    &self.state.manager.global_sessions,
                    &self.meta,
                    input["source"].as_str().context("Missing image source")?,
                    input["caption"].as_str().unwrap_or("Image"),
                )
                .await
            }
            _ => bail!("Unknown SDK interaction request"),
        }
    }

    /// Auto is offered only where the SDK accepts it for this session.
    async fn auto_available(&self) -> bool {
        let auto = crate::state_fs::sessions::ClaudePermissionMode::Auto;
        self.meta.claude_permission_mode != auto
            && super::probe::probe(&self.state, &self.meta, auto)
                .await
                .is_ok()
    }

    pub(super) async fn ask(
        &self,
        questions: Vec<AskUserQuestion>,
        id: String,
    ) -> Result<Vec<AskUserAnswer>> {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        // Register before publication, so a quick widget reply cannot race it.
        self.state.pending_ask_user.lock().await.insert(
            id.clone(),
            PendingAskUser {
                agent_id: self.agent_id.clone(),
                questions: questions.clone(),
                sender: tx,
                session_id: Some(self.meta.id.clone()),
            },
        );
        let _ = self.state.events_tx.send(ServerEvent::AskUser {
            agent_id: self.agent_id.clone(),
            question_id: id.clone(),
            questions,
            session_id: Some(self.meta.id.clone()),
        });
        let mut tick = super::runtime::cancel_tick();
        let result = loop {
            tokio::select! {
                answer = &mut rx => break answer.context("Question cancelled"),
                _ = tick.tick() => {
                    if self.state.manager.is_run_cancelled(&self.run_id).await { break Err(anyhow::anyhow!("run cancelled")); }
                }
            }
        };
        self.state.pending_ask_user.lock().await.remove(&id);
        result
    }
}

fn option(label: &str) -> AskUserOption {
    AskUserOption {
        label: label.into(),
        description: None,
        preview: None,
    }
}
