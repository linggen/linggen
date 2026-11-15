//! `GET /api/assets/{skill}`: what a skill wants, holds and fails to get.

use super::manifest::FileEntry;
use super::{Assets, Pack};
use crate::util::LockExt;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub files: usize,
    pub bytes: u64,
    pub held_files: usize,
    pub held_bytes: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PackReport {
    pub pack: String,
    #[serde(flatten)]
    pub counts: Counts,
    /// Wanted files that have failed since they were last tried.
    pub failing: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Report {
    pub packs: Vec<PackReport>,
    #[serde(flatten)]
    pub total: Counts,
}

impl Counts {
    fn add(&mut self, file: &FileEntry, held: bool) {
        self.files += 1;
        self.bytes += file.size;
        if held {
            self.held_files += 1;
            self.held_bytes += file.size;
        }
    }
}

impl Assets {
    /// None when the skill has no manifest.
    pub fn report(&self, skill: &str) -> Option<Report> {
        let state = self.state.lock_ok();
        let pack = state.packs.get(skill)?;
        let mut by_pack: BTreeMap<&str, (u32, PackReport)> = BTreeMap::new();
        let mut total = Counts::default();
        for file in &pack.manifest.files {
            let held = self.holds(pack, file);
            let (first, report) = by_pack
                .entry(&file.pack)
                .or_insert_with(|| (file.order, PackReport::named(&file.pack)));
            *first = (*first).min(file.order);
            report.counts.add(file, held);
            report.failing += usize::from(!held && state.queue.failures(&file.object()) > 0);
            total.add(file, held);
        }
        let mut packs: Vec<_> = by_pack.into_values().collect();
        packs.sort_by(|a, b| (a.0, &a.1.pack).cmp(&(b.0, &b.1.pack)));
        Some(Report {
            packs: packs.into_iter().map(|(_, report)| report).collect(),
            total,
        })
    }

    fn holds(&self, pack: &Pack, file: &FileEntry) -> bool {
        pack.dir.join(&file.path).is_file() || self.store.holds(file)
    }
}

impl PackReport {
    fn named(pack: &str) -> Self {
        Self {
            pack: pack.to_string(),
            counts: Counts::default(),
            failing: 0,
        }
    }
}
