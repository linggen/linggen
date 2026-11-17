//! Grants per place: what the user granted in one app's turn holds in that
//! app's turns of the session, and in no other app's.

use crate::engine::permission::{effective_mode_for_path, PermissionMode, SessionPermissions};
use crate::engine::skill_activation::ActivationMode;
use crate::engine::turn_place::TurnPlace;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use std::path::Path;

const FOLDER: &str = "/zz-places/x-app/work";

fn engine(sid: &str) -> AgentEngine {
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        std::env::temp_dir(),
        InterfaceMode::Web,
    );
    let models = std::sync::Arc::new(crate::provider::models::ModelManager::new_with_credentials(
        Vec::new(),
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap();
    engine.tools.builtins.set_session_id(Some(sid.to_string()));
    engine
}

fn app(name: &str, own: bool) -> TurnPlace {
    TurnPlace {
        skill: Some(name.to_string()),
        own,
        ..TurnPlace::default()
    }
}

/// A turn in `place`, as the loop starts one: its grants read from the file.
async fn turn_in(engine: &mut AgentEngine, sid: &str, place: TurnPlace) -> Option<PermissionMode> {
    engine.leave_place();
    engine.enter_place(place, None, ActivationMode::SessionBound).await;
    engine.load_session_permissions(sid);
    effective_mode_for_path(&engine.session_permissions.path_modes, Path::new(FOLDER))
}

/// What the loop does when the user allows a folder on a prompt.
fn user_grants(engine: &mut AgentEngine, mode: PermissionMode) {
    let mut updated = engine.session_permissions.clone();
    updated.set_path_mode(FOLDER, mode);
    updated.try_save(engine.session_dir.as_deref().unwrap()).unwrap();
    engine.session_permissions = updated;
}

fn session() -> (String, std::path::PathBuf) {
    let sid = format!("sess-place-grants-{}", uuid::Uuid::new_v4());
    let sdir = crate::paths::global_sessions_dir().join(&sid);
    (sid, sdir)
}

#[tokio::test]
async fn a_grant_in_one_app_is_not_honoured_in_another() {
    let (sid, sdir) = session();
    let mut engine = engine(&sid);

    turn_in(&mut engine, &sid, app("zz-x", true)).await;
    user_grants(&mut engine, PermissionMode::Edit);
    engine.leave_place();

    let in_y = turn_in(&mut engine, &sid, app("zz-y", false)).await;
    assert_ne!(in_y, Some(PermissionMode::Edit), "x's grant honoured in y");
    let at_home = turn_in(&mut engine, &sid, TurnPlace { own: false, ..TurnPlace::home() }).await;
    assert_ne!(at_home, Some(PermissionMode::Edit), "x's grant honoured at home");

    let back = turn_in(&mut engine, &sid, app("zz-x", true)).await;
    assert_eq!(back, Some(PermissionMode::Edit), "x's grant lost after a turn in y");
    let _ = std::fs::remove_dir_all(&sdir);
}

/// A grant given in another app's turn is that app's: the session's own
/// place, and the file's top level every older reader reads, never see it.
#[tokio::test]
async fn a_grant_in_another_apps_turn_stays_with_that_app() {
    let (sid, sdir) = session();
    let mut engine = engine(&sid);

    turn_in(&mut engine, &sid, app("zz-y", false)).await;
    user_grants(&mut engine, PermissionMode::Admin);
    engine.leave_place();

    let own = SessionPermissions::load(&sdir);
    assert_ne!(
        effective_mode_for_path(&own.path_modes, Path::new(FOLDER)),
        Some(PermissionMode::Admin)
    );
    assert_eq!(
        turn_in(&mut engine, &sid, app("zz-y", false)).await,
        Some(PermissionMode::Admin)
    );
    // Leaving puts the engine back on the session's own grants.
    engine.leave_place();
    assert!(engine.session_permissions.place.is_none());
    let _ = std::fs::remove_dir_all(&sdir);
}

/// A permission.json written before places reads as the grants of the
/// session's own place — whichever that is — and of no other.
#[tokio::test]
async fn an_old_file_is_the_sessions_own_places_grants() {
    let (sid, sdir) = session();
    std::fs::create_dir_all(&sdir).unwrap();
    let old = format!(
        r#"{{"path_modes":[{{"path":"{FOLDER}","mode":"edit"}}],"interactive":true,"policy":"old"}}"#
    );
    std::fs::write(sdir.join("permission.json"), old).unwrap();
    let mut engine = engine(&sid);

    let own = turn_in(&mut engine, &sid, app("zz-x", true)).await;
    assert_eq!(own, Some(PermissionMode::Edit));
    let other = turn_in(&mut engine, &sid, app("zz-y", false)).await;
    assert_ne!(other, Some(PermissionMode::Edit));
    let home = turn_in(&mut engine, &sid, TurnPlace::home()).await;
    assert_eq!(home, Some(PermissionMode::Edit), "a plain chat's own place is home");
    // Another place's grants never move the top level.
    let file = std::fs::read_to_string(sdir.join("permission.json")).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(raw["path_modes"][0]["path"], FOLDER);
    let _ = std::fs::remove_dir_all(&sdir);
}

/// A subagent of a turn in another app's place uses that place's grants.
#[tokio::test]
async fn a_subagent_uses_its_parent_places_grants() {
    let (sid, sdir) = session();
    let mut parent = engine(&sid);
    turn_in(&mut parent, &sid, app("zz-y", false)).await;
    user_grants(&mut parent, PermissionMode::Edit);

    let mut child = engine(&sid);
    child.tools.builtins.turn_place.set(parent.turn_place().cloned());
    child.join_session_permissions(&sid);
    assert_eq!(
        effective_mode_for_path(&child.session_permissions.path_modes, Path::new(FOLDER)),
        Some(PermissionMode::Edit)
    );
    let mut own_child = engine(&sid);
    own_child.join_session_permissions(&sid);
    assert_ne!(
        effective_mode_for_path(&own_child.session_permissions.path_modes, Path::new(FOLDER)),
        Some(PermissionMode::Edit)
    );
    let _ = std::fs::remove_dir_all(&sdir);
}
