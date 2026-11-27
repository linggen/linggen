//! The ChatGPT catalog: the one place that knows which models the ChatGPT
//! built-ins are, what an older id of a family now means, and which Codex
//! client version to present to the backend.
//!
//! It holds one [`Snapshot`], seeded from the compiled values in [`seed`] —
//! an engine that has never been online behaves exactly as the seed says —
//! and kept on disk beside the engine's other state. Everything else that
//! names a built-in ChatGPT id, a retired id or the client version asks here;
//! `scan_tests` fails when a literal of either appears anywhere else.
//!
//! Derivation is pure (methods on `&Snapshot`), so tests build their own
//! snapshot and never touch the process-wide one.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

mod seed;
pub mod listing;
pub mod promote;
pub mod refresh;
pub mod version;

#[cfg(test)]
mod scan_tests;
#[cfg(test)]
pub(crate) mod tests;

/// One model the backend listed for this account. Only visible models are
/// kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub slug: String,
    /// The backend's order, 1 first. Within a family, lower is newer.
    pub priority: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_levels: Vec<String>,
    /// Metadata only: Auto lets the provider apply its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_level: Option<String>,
}

/// One built-in family (Sol, Terra, Luna) and where it stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Family {
    pub name: String,
    /// The family's model: the newest one that has answered.
    pub current: String,
    /// What it was on before its last move — what "stay" goes back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// When it last moved, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_at: Option<u64>,
    /// The person chose to stay on `previous` while the backend lists it.
    #[serde(default)]
    pub stay: bool,
    /// Every model this family has been on, so each keeps meaning the family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
}

/// Everything the catalog knows. Persisted as JSON; every field defaults, so
/// an older or partial file still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// The newest plain released Codex client version last read from the
    /// public listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_read_at: Option<u64>,
    /// The backend's visible models for the signed-in account, as last read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listed: Vec<Listed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_at: Option<u64>,
    /// When the last refresh pass started, whether its reads worked or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_tried_at: Option<u64>,
    #[serde(default)]
    pub families: Vec<Family>,
    /// A newer model that was tried and did not answer, with when.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<String, u64>,
}

impl Snapshot {
    /// What an engine that has read nothing knows.
    pub fn seed() -> Self {
        let families = seed::FAMILIES
            .iter()
            .map(|(name, current)| Family {
                name: name.to_string(),
                current: current.to_string(),
                previous: None,
                moved_at: None,
                stay: false,
                history: Vec::new(),
            })
            .collect();
        Self {
            families,
            ..Self::default()
        }
    }

    /// A snapshot read from disk, made to agree with this build: every seed
    /// family present and nothing else, and a family whose current model
    /// this build knows to be retired starts again from the seed.
    pub fn normalized(mut self) -> Self {
        let mut families = Vec::new();
        for (name, start) in seed::FAMILIES {
            let mut fam = match self.families.iter().position(|f| f.name == *name) {
                Some(i) => self.families.swap_remove(i),
                None => continue_with_seed(name, start),
            };
            if seed_family_of(&fam.current).is_some() {
                fam.history.push(std::mem::replace(&mut fam.current, start.to_string()));
                fam.previous = None;
                fam.stay = false;
            }
            families.push(fam);
        }
        self.families = families;
        self
    }

    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.iter().find(|f| f.name == name)
    }

    pub fn family_mut(&mut self, name: &str) -> Option<&mut Family> {
        self.families.iter_mut().find(|f| f.name == name)
    }

    /// The model a family runs on now: its previous one while the person
    /// stays there and the backend still lists it, else its current one.
    pub fn effective<'a>(&self, fam: &'a Family) -> &'a str {
        match &fam.previous {
            Some(prev) if fam.stay && self.still_served(prev) => prev,
            _ => &fam.current,
        }
    }

    /// The built-in ids, one per family, in the seed's order.
    pub fn builtin_ids(&self) -> Vec<String> {
        self.families
            .iter()
            .map(|f| self.effective(f).to_string())
            .collect()
    }

    /// The primary built-in: the default routing target and picture model.
    pub fn primary_id(&self) -> String {
        self.family(seed::PRIMARY_FAMILY)
            .map(|f| self.effective(f).to_string())
            .unwrap_or_default()
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.families.iter().any(|f| self.effective(f) == id)
    }

    /// Whether the backend's last list carries [slug].
    pub fn is_listed(&self, slug: &str) -> bool {
        self.listed.iter().any(|m| m.slug == slug)
    }

    /// Whether [id] can still be asked for: listed, or nothing listed yet.
    fn still_served(&self, id: &str) -> bool {
        self.listed.is_empty() || self.is_listed(id)
    }

    /// The built-in family [id] belongs to, if it is a ChatGPT id this
    /// catalog knows: an old id of the seed, one a family has been on, or a
    /// listed model whose slug ends in a family's name.
    pub fn family_of(&self, id: &str) -> Option<&Family> {
        if let Some(name) = seed_family_of(id) {
            return self.family(name);
        }
        if let Some(f) = self.families.iter().find(|f| f.has_been_on(id)) {
            return Some(f);
        }
        if !self.is_listed(id) {
            return None;
        }
        self.family(slug_family(id)?)
    }

    /// What a reference that follows its family (a default, the companion's
    /// model, a device's or a room's shared models) should say instead of
    /// [id]: the family's model now. None when [id] is a built-in already or
    /// not a ChatGPT id this catalog knows.
    pub fn follow(&self, id: &str) -> Option<String> {
        if self.is_builtin(id) {
            return None;
        }
        self.family_of(id).map(|f| self.effective(f).to_string())
    }

    /// Whether [id] is a model a built-in family has been on and left (or an
    /// old id of the seed) — what an older release may have written as a
    /// copy of a built-in. A listed model no family has been on is not.
    pub fn is_former(&self, id: &str) -> bool {
        if self.is_builtin(id) {
            return false;
        }
        seed_family_of(id).is_some() || self.families.iter().any(|f| f.has_been_on(id))
    }

    /// What an exact pin (a session's or a mission's model) should run on
    /// instead of [id]: nothing while the backend still lists [id], else
    /// what [`follow`](Self::follow) says.
    pub fn pinned(&self, id: &str) -> Option<String> {
        if self.is_kept(id) {
            return None;
        }
        self.follow(id)
    }

    /// A listed model of a built-in family that is not the family's model:
    /// still runnable for a pin that names it, never offered.
    fn is_kept(&self, id: &str) -> bool {
        self.is_listed(id) && !self.is_builtin(id) && self.family_of(id).is_some()
    }

    /// Every [`is_kept`](Self::is_kept) model.
    pub fn kept_ids(&self) -> Vec<String> {
        self.listed
            .iter()
            .filter(|m| self.is_kept(&m.slug))
            .map(|m| m.slug.clone())
            .collect()
    }

    /// Every old id this catalog knows, with what it now means for a
    /// reference that follows its family.
    pub fn successors(&self) -> BTreeMap<String, String> {
        let known = seed::RETIRED
            .iter()
            .map(|(id, _)| id.to_string())
            .chain(self.families.iter().flat_map(Family::ids))
            .chain(self.listed.iter().map(|m| m.slug.clone()));
        known
            .filter_map(|id| self.follow(&id).map(|next| (id, next)))
            .collect()
    }

    /// The Codex client version to present: the newer of the compiled one and
    /// the last one read from the public listing.
    pub fn client_version(&self) -> String {
        let floor = seed::CLIENT_VERSION;
        let Some(read) = self.published_version.as_deref() else {
            return floor.to_string();
        };
        match (parse_version(read), parse_version(floor)) {
            (Some(r), Some(f)) if r > f => read.to_string(),
            _ => floor.to_string(),
        }
    }

    /// For the models page: when [id] is a built-in whose family moved,
    /// which model the family is on now, what it was on, since when, and
    /// whether the person stays on (or may go back to) the previous one.
    pub fn moved_note(&self, id: &str) -> Option<serde_json::Value> {
        let fam = self.families.iter().find(|f| self.effective(f) == id)?;
        let previous = fam.previous.as_deref()?;
        Some(serde_json::json!({
            "family": fam.name,
            "current": fam.current,
            "previous": previous,
            "since": fam.moved_at,
            "staying": self.effective(fam) == previous,
            "can_stay": self.still_served(previous),
        }))
    }

    /// Stay on [family]'s previous model, or follow it again. Staying needs
    /// a previous model the backend still lists.
    pub fn set_stay(&mut self, family: &str, stay: bool) -> Result<(), &'static str> {
        let served = |id: &str| self.still_served(id);
        let fam = self.families.iter().find(|f| f.name == family).ok_or("no such family")?;
        if stay && !fam.previous.as_deref().is_some_and(served) {
            return Err("nothing to go back to");
        }
        if let Some(fam) = self.family_mut(family) {
            fam.stay = stay;
        }
        Ok(())
    }

    /// What a paired phone takes from this Mac instead of keeping copies:
    /// the version to present when it talks to ChatGPT itself, the
    /// built-ins, and what every older id now means.
    pub fn for_phone(&self) -> serde_json::Value {
        serde_json::json!({
            "client_version": self.client_version(),
            "builtins": self.builtin_ids(),
            "successors": self.successors(),
        })
    }

    /// The supported effort values the backend listed for this exact slug.
    pub(super) fn reasoning_levels(&self, slug: &str) -> &[String] {
        self.listed.iter().find(|m| m.slug == slug)
            .map_or(&[], |m| m.reasoning_levels.as_slice())
    }

    /// The effort the backend lists as this exact slug's own default.
    pub(super) fn default_reasoning_level(&self, slug: &str) -> Option<&str> {
        self.listed.iter().find(|m| m.slug == slug)
            .and_then(|m| m.default_reasoning_level.as_deref())
    }

    /// The context window the backend listed for [slug].
    pub fn context_window(&self, slug: &str) -> Option<u64> {
        self.listed
            .iter()
            .find(|m| m.slug == slug)
            .and_then(|m| m.context_window)
    }
}

impl Family {
    fn has_been_on(&self, id: &str) -> bool {
        self.current == id || self.previous.as_deref() == Some(id) || self.history.iter().any(|h| h == id)
    }

    fn ids(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(self.current.clone())
            .chain(self.previous.clone())
            .chain(self.history.iter().cloned())
    }
}

fn continue_with_seed(name: &str, start: &str) -> Family {
    Family {
        name: name.to_string(),
        current: start.to_string(),
        previous: None,
        moved_at: None,
        stay: false,
        history: Vec::new(),
    }
}

/// The family an old id of the seed belongs to.
fn seed_family_of(id: &str) -> Option<&'static str> {
    seed::RETIRED
        .iter()
        .find(|(old, _)| *old == id)
        .map(|(_, fam)| *fam)
}

/// A slug's family: its last word (`gpt-6.2-sol` → `sol`).
pub fn slug_family(slug: &str) -> Option<&str> {
    let (_, last) = slug.rsplit_once('-')?;
    (!last.is_empty() && last.chars().all(|c| c.is_ascii_alphabetic())).then_some(last)
}

/// A plain released version: exactly three numbers, nothing after them.
pub fn parse_version(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        let p = parts.next()?;
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The generation in an OpenAI-style model name and what follows its
/// number: `gpt-4o` → (4, "o"), `openai/gpt-6.2-sol` → (6, ".2-sol"). Name
/// heuristics read this rather than spelling each generation out.
pub fn openai_generation(name: &str) -> Option<(u32, &str)> {
    let start = name.find("gpt-")? + 4;
    let rest = &name[start..];
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    Some((rest[..digits].parse().ok()?, &rest[digits..]))
}

// ---------------------------------------------------------------------------
// The process-wide snapshot
// ---------------------------------------------------------------------------

static CURRENT: RwLock<Option<Arc<Snapshot>>> = RwLock::new(None);

/// The snapshot in force: loaded from disk on first use, else the seed.
pub fn current() -> Arc<Snapshot> {
    if let Some(s) = CURRENT.read().ok().and_then(|g| g.clone()) {
        return s;
    }
    let loaded = Arc::new(store_path().map(|p| load_from(&p)).unwrap_or_else(Snapshot::seed));
    if let Ok(mut g) = CURRENT.write() {
        return g.get_or_insert(loaded).clone();
    }
    loaded
}

/// Put [snapshot] in force and keep it on disk. A failed write is logged; the
/// snapshot still holds for this run.
pub fn publish(snapshot: Snapshot) -> Arc<Snapshot> {
    if let Some(path) = store_path() {
        if let Err(e) = save_to(&path, &snapshot) {
            tracing::warn!("chatgpt catalog: could not save {}: {e}", path.display());
        }
    }
    let snapshot = Arc::new(snapshot);
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(snapshot.clone());
    }
    snapshot
}

/// Where the snapshot lives: beside the engine's other state, not in the
/// user's config. A test run has none — it never reads or writes a home.
fn store_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    Some(crate::paths::linggen_home().join("chatgpt_models.json"))
}

/// The snapshot in [path], or the seed when it is missing or unreadable.
pub fn load_from(path: &Path) -> Snapshot {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Snapshot>(&t).ok())
        .map(Snapshot::normalized)
        .unwrap_or_else(Snapshot::seed)
}

/// Write [snapshot] to [path] whole (a temporary file renamed over it).
pub fn save_to(path: &Path, snapshot: &Snapshot) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(snapshot)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// What the rest of the engine asks
// ---------------------------------------------------------------------------

/// The built-in ChatGPT ids, one per family.
pub fn builtin_ids() -> Vec<String> {
    current().builtin_ids()
}

/// The primary built-in ChatGPT id.
pub fn primary_id() -> String {
    current().primary_id()
}

pub fn is_builtin(id: &str) -> bool {
    current().is_builtin(id)
}

/// What a family-following reference to [id] should say now.
pub fn successor(id: &str) -> Option<String> {
    current().follow(id)
}

/// What an exact pin to [id] should run on now.
pub fn pin_successor(id: &str) -> Option<String> {
    current().pinned(id)
}

/// Stay on [family]'s previous model or follow it again, in the snapshot in
/// force (kept on disk, so it survives refreshes and restarts).
pub fn set_stay(family: &str, stay: bool) -> Result<Arc<Snapshot>, &'static str> {
    let mut refused = None;
    let kept = InForce.modify(&mut |cur| {
        let mut next = cur.clone();
        match next.set_stay(family, stay) {
            Ok(()) => Some(next),
            Err(why) => {
                refused = Some(why);
                None
            }
        }
    });
    refused.map_or(Ok(kept), Err)
}

/// Where a snapshot is kept: the process-wide one ([`InForce`]) or a
/// test's own. Every change is a read-modify-write under one writer, so a
/// refresh and a person's choice never overwrite each other.
pub trait Store: Send + Sync {
    fn current(&self) -> Arc<Snapshot>;
    /// Give [change] the snapshot kept now; what it returns, if anything,
    /// is kept instead. Returns what is kept after.
    fn modify(&self, change: &mut dyn FnMut(&Snapshot) -> Option<Snapshot>) -> Arc<Snapshot>;
}

/// The process-wide snapshot, kept on disk.
pub struct InForce;

static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Store for InForce {
    fn current(&self) -> Arc<Snapshot> {
        current()
    }

    fn modify(&self, change: &mut dyn FnMut(&Snapshot) -> Option<Snapshot>) -> Arc<Snapshot> {
        let _one = WRITER.lock().unwrap_or_else(|e| e.into_inner());
        let cur = current();
        match change(&cur) {
            Some(next) => publish(next),
            None => cur,
        }
    }
}

/// The Codex client version to present to the ChatGPT backend.
pub fn client_version() -> String {
    current().client_version()
}
