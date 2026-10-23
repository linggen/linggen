//! Widget transport only. The SDK decides when permission is needed; no
//! native tool executor, permission tiers or path grants are consulted.
use super::{runtime::ClaudeSession, AGENT_ID};
use crate::engine::tools::{AskUserAnswer, AskUserOption, AskUserQuestion, PendingAskUser};
use crate::server::ServerEvent;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

impl ClaudeSession {
    pub async fn request(&mut self, event: &Value) -> Result<Value> {
        let input = &event["input"];
        match event["kind"].as_str() {
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
                let question = AskUserQuestion {
                    question: input["title"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{tool} {target}")),
                    header: "Permission".into(),
                    multi_select: false,
                    options: vec![option("Allow once"), option("Deny")],
                };
                let answers = self.ask(vec![question]).await?;
                let answer = answers.first();
                let allow = answer.is_some_and(|a| {
                    a.selected.first().is_some_and(|s| s == "Allow once")
                        && a.custom_text.as_deref().unwrap_or_default().is_empty()
                });
                let reason = answer
                    .and_then(|a| a.custom_text.as_deref())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("User declined this tool call.");
                Ok(json!({"allow":allow, "reason":reason}))
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
                let answers = self.ask(questions).await?;
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
            _ => bail!("Unknown SDK interaction request"),
        }
    }

    async fn ask(&mut self, questions: Vec<AskUserQuestion>) -> Result<Vec<AskUserAnswer>> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        // Register before publication, so a quick widget reply cannot race it.
        self.state.pending_ask_user.lock().await.insert(
            id.clone(),
            PendingAskUser {
                agent_id: AGENT_ID.into(),
                questions: questions.clone(),
                sender: tx,
                session_id: Some(self.meta.id.clone()),
            },
        );
        let _ = self.state.events_tx.send(ServerEvent::AskUser {
            agent_id: AGENT_ID.into(),
            question_id: id.clone(),
            questions,
            session_id: Some(self.meta.id.clone()),
        });
        let result = loop {
            tokio::select! {
                answer = &mut rx => break answer.context("Question cancelled"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if self.cancelled().await { break Err(anyhow::anyhow!("run cancelled")); }
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
