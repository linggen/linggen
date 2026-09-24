//! The pet's body and voice: Express and Voice.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::LazyLock;

#[derive(serde::Deserialize)]
struct ExpressArgs {
    #[serde(default)]
    emotion: Option<String>,
    #[serde(default)]
    action: Option<String>,
    /// An ordered list of gestures to play back-to-back as one routine.
    /// Takes precedence over `action` when present.
    #[serde(default)]
    sequence: Option<Vec<String>>,
}

/// Cap on how many gestures one Express call may chain.
const MAX_SEQUENCE: usize = 8;

/// One entry of the pet animation manifest. Only the fields the engine needs
/// to build the `Express` tool schema are deserialized; the renderer-side
/// fields (`render`, `proc`, `clips`, `visible`, `type`, …) are ignored here
/// and consumed by the web UI instead.
#[derive(serde::Deserialize)]
struct PetIntent {
    name: String,
    use_when: String,
}

/// The pet's `Express` vocabulary — the single source of truth shared with the
/// web renderer. Baked in at compile time from the UI's manifest so the tool
/// schema and the avatar can never drift; a malformed manifest fails the build.
static PET_INTENTS: LazyLock<Vec<PetIntent>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Manifest {
        intents: Vec<PetIntent>,
    }
    let raw = include_str!("../../../../ui/public/anim/actions.json");
    serde_json::from_str::<Manifest>(raw)
        .expect("ui/public/anim/actions.json must be valid")
        .intents
});

/// The mascot's body control — she shows a mood and/or a gesture on her avatar.
/// Fire-and-forget: emits a `PetExpress` event to every surface and returns
/// immediately. Carries no speech (her spoken line is just her reply text).
pub struct ExpressTool;
#[async_trait]
impl Tool for ExpressTool {
    fn name(&self) -> &'static str {
        "Express"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["express"]
    }
    fn description(&self) -> &'static str {
        "Show feeling on your avatar body: a sustained mood and/or a one-shot \
         gesture (no speech). Use sparingly and naturally — never narrate it."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        let names: Vec<Value> = PET_INTENTS
            .iter()
            .map(|i| Value::String(i.name.clone()))
            .collect();
        let menu = PET_INTENTS
            .iter()
            .map(|i| format!("• {} — {}", i.name, i.use_when))
            .collect::<Vec<_>>()
            .join("\n");
        json!({
            "type": "object",
            "properties": {
                "emotion": {
                    "type": "string",
                    "enum": ["neutral", "happy", "sad", "angry", "relaxed"],
                    "description": "Sustained mood to hold until you change it."
                },
                "action": {
                    "type": "string",
                    "enum": names.clone(),
                    "description": format!(
                        "A gesture, pose, or movement. Choose by what fits the moment:\n{menu}"
                    )
                },
                "sequence": {
                    "type": "array",
                    "items": { "type": "string", "enum": names },
                    "description": "Several gestures to play back-to-back as one little routine, \
                        in order (e.g. [\"wave\", \"tilt_head\", \"shrug\"]). Use instead of `action` \
                        when one beat isn't enough. Max 8."
                }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        let names = PET_INTENTS
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join("|");
        json!({
            "name": "Express",
            "args": {"emotion": "string?", "action": "string?", "sequence": "string[]?"},
            "returns": "ok",
            "notes": format!(
                "Show feeling on your avatar. emotion (sustained): neutral|happy|sad|angry|relaxed. \
                 action: {names}. sequence: an ordered list of those to chain (max 8). \
                 At least one of emotion/action/sequence. Use sparingly; never narrate it."
            )
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ExpressArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Express: {}", e))?;

        // `sequence` (an ordered routine) takes precedence over a single `action`.
        let intents: Vec<String> = match args.sequence {
            Some(seq) if !seq.is_empty() => seq,
            _ => args.action.into_iter().collect(),
        };
        if args.emotion.is_none() && intents.is_empty() {
            anyhow::bail!("Express needs at least one of: emotion, action, sequence");
        }
        if intents.len() > MAX_SEQUENCE {
            anyhow::bail!("Express sequence too long (max {MAX_SEQUENCE})");
        }
        for name in &intents {
            if !PET_INTENTS.iter().any(|i| &i.name == name) {
                anyhow::bail!("Express: unknown action '{name}' (not in the avatar vocabulary)");
            }
        }
        // Transport the ordered intents as one comma-joined string so the
        // existing PetExpress event + spine stay unchanged; the UI splits + queues.
        let action = (!intents.is_empty()).then(|| intents.join(","));
        if let Some(manager) = tools.get_manager() {
            manager
                .send_event(
                    crate::engine::agent::AgentEvent::PetExpress {
                        emotion: args.emotion,
                        action,
                    },
                    tools.session_id.clone(),
                )
                .await;
        }
        Ok(ToolResult::Success("ok".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Voice — the pet's voice on this machine, off or on
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct VoiceArgs {
    muted: bool,
}

/// "Mute yourself", "be quiet", "you can talk again": the words for what
/// `/mute` and `/unmute` do. Every agent has it, in every session — a person
/// asks whoever they're talking to. Muted, Yinyue still writes every line.
pub struct VoiceTool;
#[async_trait]
impl Tool for VoiceTool {
    fn name(&self) -> &'static str {
        "Voice"
    }
    fn description(&self) -> &'static str {
        "Turn Yinyue's spoken voice on this Mac off or on. Muted, she still \
         writes every line — only the audio stops — until it is turned back on. \
         Use when the person asks to mute her (or you), to be quiet or silent, \
         to stop talking out loud — or to speak again. Not for ending an answer."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "muted": {
                    "type": "boolean",
                    "description": "true = voice off (text only); false = voice back on."
                }
            },
            "required": ["muted"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Voice",
            "args": {"muted": "boolean"},
            "returns": "the new state",
            "notes": "Yinyue's voice on this Mac off (muted: true, text only) or back on (false)."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: VoiceArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Voice: {}", e))?;
        let manager = tools
            .get_manager()
            .ok_or_else(|| anyhow::anyhow!("Voice: no engine to change"))?;
        manager.set_pet_muted(args.muted).await?;
        Ok(ToolResult::Success(if args.muted {
            "Yinyue's voice is off on this Mac; she still writes. Tell the person in a few words."
        } else {
            "Yinyue's voice is back on on this Mac. Tell the person in a few words."
        }
        .to_string()))
    }
}

#[cfg(test)]
mod express_tests {
    use super::*;

    /// The `Express` vocabulary is built from `ui/public/anim/actions.json` at
    /// runtime — this proves the baked-in manifest parses and every intent
    /// reaches the model-facing schema (the engine/renderer contract).
    #[test]
    fn express_vocab_loads_from_manifest() {
        let schema = ExpressTool.args_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum array");
        assert_eq!(actions.len(), 43, "expected 43 intents from actions.json");

        let names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
        for expected in [
            "nod",
            "wave",
            "dance",
            "appear",
            "disappear",
            "walk",
            "run",
            "think",
            "spin",
            "pose",
        ] {
            assert!(names.contains(&expected), "missing intent '{expected}'");
        }

        // `sequence` chains the same vocabulary.
        let seq = schema["properties"]["sequence"]["items"]["enum"]
            .as_array()
            .expect("sequence items enum array");
        assert_eq!(
            seq.len(),
            actions.len(),
            "sequence vocab must match action vocab"
        );
    }
}
