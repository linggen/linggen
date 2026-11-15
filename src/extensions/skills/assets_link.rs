//! Hands the installed skills' asset manifests (`app.assets`) to
//! `crate::assets` every time the skills are loaded or reloaded.

use super::SkillLoader;
use crate::assets::{Assets, Declared};
use crate::engine::skill::Skill;
use std::sync::Arc;

/// Start the downloader and follow the skill set. The first read comes at
/// once if the skills are already loaded.
pub fn spawn(assets: Arc<Assets>, skills: Arc<SkillLoader>) {
    assets.spawn_worker();
    tokio::spawn(async move {
        let mut loads = skills.subscribe();
        let mut seen = 0;
        while let Ok(count) = loads.wait_for(|n| *n > seen).await.map(|n| *n) {
            seen = count;
            let declared = declared_by(&skills.list_skills().await);
            let assets = assets.clone();
            if tokio::task::spawn_blocking(move || assets.refresh(declared))
                .await
                .is_err()
            {
                return;
            }
        }
    });
}

fn declared_by(skills: &[Skill]) -> Vec<Declared> {
    skills
        .iter()
        .filter_map(|skill| {
            Some(Declared {
                skill: skill.name.clone(),
                dir: skill.skill_dir.clone()?,
                file: skill.app.as_ref()?.assets.clone()?,
            })
        })
        .collect()
}
