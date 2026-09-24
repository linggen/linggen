//! User-level events that are not one conversation's: new sessions,
//! notifications, launched apps, the working folder, rooms, skill saves and
//! device topics.

use super::data::{
    AppLaunchedData, DeviceTopicData, QuestsChangedData, RoomChatData, SessionCreatedData,
    SkillSaveChangedData, WorkingFolderData,
};
use super::{Ui, UI_KIND_DEVICE_TOPIC, UI_KIND_QUESTS_CHANGED, UI_KIND_SKILL_SAVE_CHANGED};
use crate::engine::events::{NotificationPayload, ServerEvent, UiEvent};

pub(super) fn map(event: ServerEvent, ui: Ui) -> Option<UiEvent> {
    let seq = ui.seq;
    match event {
        ServerEvent::SessionCreated {
            session_id,
            title,
            creator,
            project,
            project_name,
            skill,
            mission_id,
        } => Some(
            ui.event(
                format!("session-created-{session_id}-{seq}"),
                "notification",
            )
            .text(format!("Session created: {title}"))
            .global()
            .project_root(project.clone())
            .data(SessionCreatedData {
                kind: "session_created".to_string(),
                session_id,
                title,
                creator,
                project,
                project_name,
                skill,
                mission_id,
            }),
        ),
        ServerEvent::Notification(payload) => notification(payload, ui),
        ServerEvent::AppLaunched {
            skill,
            launcher,
            url,
            title,
            width,
            height,
            session_id,
        } => Some(
            ui.event(format!("app-launched-{skill}-{seq}"), "app_launched")
                .text(format!("Launched app: {}", title))
                .session(Some(
                    session_id.unwrap_or_else(|| super::GLOBAL.to_string()),
                ))
                .data(AppLaunchedData {
                    skill,
                    launcher,
                    url,
                    title,
                    width,
                    height,
                }),
        ),
        ServerEvent::WorkingFolderChanged {
            session_id,
            cwd,
            project,
            project_name,
        } => Some(
            ui.event(format!("wf-{seq}"), "working_folder")
                .session(Some(session_id))
                .data(WorkingFolderData {
                    cwd,
                    project,
                    project_name,
                }),
        ),
        ServerEvent::RoomChat {
            sender_id,
            sender_name,
            avatar_url,
            text,
        } => Some(
            ui.event(format!("room-chat-{seq}"), "room_chat")
                .text(text.clone())
                .global()
                .data(RoomChatData {
                    sender_id,
                    sender_name,
                    avatar_url,
                    text,
                }),
        ),
        // Global → every surface, the skill's page among them.
        ServerEvent::SkillSaveChanged {
            skill,
            version,
            conflicts,
        } => Some(
            ui.event(format!("skill-save-{seq}"), UI_KIND_SKILL_SAVE_CHANGED)
                .data(SkillSaveChangedData {
                    skill,
                    version,
                    conflicts,
                }),
        ),
        // Global → every surface; a skill page showing quests reads them again.
        ServerEvent::QuestsChanged { app } => Some(
            ui.event(format!("quests-{seq}"), UI_KIND_QUESTS_CHANGED)
                .data(QuestsChangedData { app }),
        ),
        // User-level: reaches every surface over the control channel.
        ServerEvent::DeviceTopic {
            topic,
            op,
            payload,
            from_device,
        } => Some(
            ui.event(format!("device-topic-{seq}"), UI_KIND_DEVICE_TOPIC)
                .global()
                .data(DeviceTopicData {
                    topic,
                    op,
                    payload,
                    from_device,
                }),
        ),
        _ => None,
    }
}

fn notification(payload: NotificationPayload, ui: Ui) -> Option<UiEvent> {
    match &payload {
        NotificationPayload::MissionCompleted {
            mission_id,
            mission_name,
            status,
            ..
        } => {
            let mut e = ui
                .event(
                    format!("notif-mission-{mission_id}-{}", ui.seq),
                    "notification",
                )
                .text(format!("Mission '{}' {}", mission_name, status))
                .global();
            e.data = serde_json::to_value(&payload).ok();
            Some(e)
        }
        // Internal signal that drives Yinyue's spoken apology — not a UI
        // banner (the failed turn already renders its own error message).
        NotificationPayload::RunFailed { .. } => None,
        // Internal trigger for Yinyue's herald watch — never a UI banner
        // (the completed turn already renders its own reply).
        NotificationPayload::RunCompleted { .. } => None,
    }
}
