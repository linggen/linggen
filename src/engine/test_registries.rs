//! Empty registries for engine tests — an `AgentManager` with no skills,
//! agents or missions installed, built without the daemon's disk loaders.

use crate::engine::agent::record::AgentSpecFile;
use crate::engine::agent::registry::AgentRegistry;
use crate::engine::mission::record::Mission;
use crate::engine::mission::MissionRegistry;
use crate::engine::skill::{Skill, SkillRegistry};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub(crate) struct Empty;

#[async_trait]
impl SkillRegistry for Empty {
    async fn get_skill(&self, _: &str) -> Option<Skill> {
        None
    }
    async fn reload_one(&self, _: &str) -> Option<Skill> {
        None
    }
    async fn list_skills(&self) -> Vec<Skill> {
        Vec::new()
    }
    async fn match_trigger(&self, _: &str) -> Option<(String, String)> {
        None
    }
    async fn list_metadata(&self) -> Vec<(String, String, bool)> {
        Vec::new()
    }
}

#[async_trait]
impl AgentRegistry for Empty {
    async fn list(&self, _: &Path) -> Result<Vec<AgentSpecFile>> {
        Ok(Vec::new())
    }
    async fn find(&self, _: &Path, _: &str) -> Result<Option<AgentSpecFile>> {
        Ok(None)
    }
}

impl MissionRegistry for Empty {
    fn get_mission(&self, _: &str) -> Result<Option<Mission>> {
        Ok(None)
    }
    fn reload_one(&self, _: &str) -> Option<Mission> {
        None
    }
    fn mission_dir(&self, id: &str) -> PathBuf {
        PathBuf::from(id)
    }
}
