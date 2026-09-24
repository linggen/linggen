//! The pet's cues: speak, express, voice on/off. Global — every surface.

use super::Ui;
use crate::engine::agent::COMPANION_AGENT_ID;
use crate::engine::events::{ServerEvent, UiEvent};
use serde_json::json;

pub(super) fn map(event: ServerEvent, ui: Ui) -> Option<UiEvent> {
    let seq = ui.seq;
    let pet = |id: String, kind: &str| ui.event(id, kind).agent(COMPANION_AGENT_ID);
    match event {
        ServerEvent::PetVoice { muted } => {
            Some(pet(format!("pet-voice-{seq}"), "pet_voice").data(json!({ "muted": muted })))
        }
        ServerEvent::PetSpeak {
            text,
            emotion,
            voice,
        } => Some(
            pet(format!("pet-speak-{seq}"), "pet_speak")
                .text(text.clone())
                .data(json!({ "text": text, "emotion": emotion, "voice": voice })),
        ),
        ServerEvent::PetExpress { emotion, action } => Some(
            pet(format!("pet-express-{seq}"), "pet_express")
                .data(json!({ "emotion": emotion, "action": action })),
        ),
        _ => None,
    }
}
