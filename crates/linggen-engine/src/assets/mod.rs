//! Asset packs: a skill's heavy files, fetched after install (doc/asset-pack-spec.md).
//!
//! A skill lists its heavy files in a manifest (`app.assets`). A listed file
//! the skill's folder does not hold is fetched in the background by its hash,
//! checked, and kept in a store shared by every skill; the skill's page asks
//! for it at the same address as ever. The module knows no skill and no file
//! kind: `pack` is a label, `order` a number.
//!
//! - `manifest` — read and validate `assets.json`
//! - `store`    — fetched objects by content, and the sweep
//! - `queue`    — what is wanted, in what order, and the back-off
//! - `fetch`    — the background downloader
//! - `report`   — what is held and what is failing, per pack
//! - `refresh`  — reads the declared manifests again when skills reload

mod fetch;
mod manifest;
mod queue;
mod refresh;
mod report;
mod store;

pub use manifest::{FileEntry, Manifest};
pub use queue::Tuning;
pub use refresh::Declared;
pub use store::Store;

use crate::util::LockExt;
use fetch::Tally;
use queue::{Queue, Want};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::Notify;

/// One installed skill's manifest, and the folder its files are asked from.
pub struct Pack {
    pub skill: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// What a page's request for `path` finds in a skill's manifest.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    /// The manifest does not list it: nothing to do with asset packs.
    Unlisted,
    /// Listed, fetched, here.
    Stored(PathBuf),
    /// Listed and not here yet. It is now first in line to be fetched.
    Pending,
}

pub struct Assets {
    store: Store,
    tuning: Tuning,
    client: reqwest::Client,
    state: Mutex<State>,
    /// Rung when the queue changes or a download ends.
    wake: Notify,
}

#[derive(Default)]
struct State {
    packs: HashMap<String, Pack>,
    queue: Queue,
    tally: Tally,
    /// The last burst of downloads had a failure (so the next is not news).
    failing: bool,
    refresh: refresh::Memory,
}

impl Assets {
    pub fn new(store: Store, tuning: Tuning) -> Self {
        Self {
            store,
            tuning,
            client: fetch::client(),
            state: Mutex::new(State::default()),
            wake: Notify::new(),
        }
    }

    /// The installed skills' manifests, replacing the last set: everything
    /// listed that neither the skill's folder nor the store holds is wanted.
    pub fn set_packs(&self, packs: Vec<Pack>) {
        let wants = packs.iter().flat_map(|pack| self.missing(pack)).collect();
        let mut state = self.state.lock_ok();
        state.packs = packs.into_iter().map(|p| (p.skill.clone(), p)).collect();
        state.queue.replace(wants);
        drop(state);
        self.wake.notify_one();
    }

    fn missing(&self, pack: &Pack) -> Vec<Want> {
        let held = |file: &FileEntry| pack.dir.join(&file.path).is_file() || self.store.holds(file);
        pack.manifest
            .files
            .iter()
            .filter(|file| !held(file))
            .map(|file| want_of(pack, file))
            .collect()
    }

    /// A page asked for `path` of `skill` and the skill's folder lacks it.
    pub fn resolve(&self, skill: &str, path: &str) -> Resolved {
        let mut state = self.state.lock_ok();
        let Some(pack) = state.packs.get(skill) else {
            return Resolved::Unlisted;
        };
        let Some(file) = pack.manifest.files.iter().find(|f| f.path == path) else {
            return Resolved::Unlisted;
        };
        if self.store.holds(file) {
            return Resolved::Stored(self.store.path_of(file));
        }
        let want = want_of(pack, file);
        state.queue.bump(want);
        drop(state);
        self.wake.notify_one();
        Resolved::Pending
    }

    /// Remove stored objects none of `packs` lists (see [`Store::sweep`]).
    pub fn sweep(&self, packs: &[Pack]) -> usize {
        self.store.sweep(packs)
    }
}

fn want_of(pack: &Pack, file: &FileEntry) -> Want {
    Want {
        url: file.url(&pack.manifest.base),
        file: file.clone(),
    }
}

#[cfg(test)]
mod tests;
