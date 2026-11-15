//! Reading the declared manifests again whenever the installed skills are
//! (re)loaded, and the one-time sweep of the store.

use super::{manifest, Assets, Pack};
use crate::util::LockExt;
use std::collections::HashSet;
use std::path::PathBuf;

/// A skill that declares a manifest (`app.assets`).
pub struct Declared {
    pub skill: String,
    pub dir: PathBuf,
    /// The manifest's path inside `dir`.
    pub file: String,
}

/// What `refresh` remembers between calls.
#[derive(Default)]
pub struct Memory {
    swept: bool,
    complained: HashSet<(String, String)>,
}

impl Assets {
    /// Read every declared manifest, sweep the store (the first time only),
    /// and hand the packs to the downloader. A bad manifest is logged once
    /// and its skill carries on without it. Blocking: call off the runtime.
    pub fn refresh(&self, declared: Vec<Declared>) {
        let (packs, unreadable) = self.read_packs(declared);
        self.sweep_first_time(&packs, unreadable);
        self.set_packs(packs);
    }

    fn read_packs(&self, declared: Vec<Declared>) -> (Vec<Pack>, usize) {
        let mut packs = Vec::new();
        let mut unreadable = 0;
        for d in declared {
            match manifest::load(&d.dir, &d.file) {
                Ok(manifest) => packs.push(Pack {
                    skill: d.skill,
                    dir: d.dir,
                    manifest,
                }),
                Err(why) => {
                    unreadable += 1;
                    self.complain_once(&d.skill, why);
                }
            }
        }
        (packs, unreadable)
    }

    fn complain_once(&self, skill: &str, why: String) {
        let key = (skill.to_string(), why.clone());
        if self.state.lock_ok().refresh.complained.insert(key) {
            tracing::warn!("[assets] {skill}: {why}; carrying on without it");
        }
    }

    /// At start-up, once, and only when every declared manifest was read: a
    /// manifest that could not be read may list what the store holds. Never
    /// on a later reload — a download may be under way by then.
    fn sweep_first_time(&self, packs: &[Pack], unreadable: usize) {
        let first = !std::mem::replace(&mut self.state.lock_ok().refresh.swept, true);
        if !first || unreadable > 0 {
            return;
        }
        let removed = self.sweep(packs);
        if removed > 0 {
            tracing::info!("[assets] swept {removed} unlisted file(s)");
        }
    }
}
