//! What moves a following row, and what happens when the move is refused.
//!
//! A row moves only when `family_current` says `Current` with a newer model,
//! the provider's list for this endpoint and key holds that model, and the
//! catalog is fresh (the selector holds a stale catalog back itself). Rows
//! without a binding are never looked at. Every decision here is made under
//! the settings write lock, from the config and catalog in force then, so a
//! row, an answer or a catalog that changed since the list was fetched is
//! simply seen as it is.

use super::{binding, Endpoint, Families};
use crate::config::Config;
use crate::credentials::Credentials;
use crate::provider::error::{kind_of, ProviderErrorKind};
use super::super::family_catalog::{preset_for, FamilyCurrent};

pub(super) fn follow_all(f: &Families, config: &mut Config, creds: &Credentials) -> bool {
    let snapshot = f.catalog();
    let today = f.today();
    let mut moves: Vec<(String, String, String)> = Vec::new();
    for row in binding::following(config) {
        let Some(held) = row.family_binding.as_ref() else { continue };
        let Some(endpoint) = Endpoint::of_row(row, creds) else { continue };
        // A row moved to another address is no longer this preset's family.
        if endpoint.preset.key != held.preset() {
            continue;
        }
        // Without a key nothing is promoted; the row keeps running as it is.
        let Some(listing) = endpoint.key.as_ref().and_then(|_| endpoint.listing(f)) else { continue };
        if let FamilyCurrent::Current { next, .. } = endpoint.ask(&snapshot, held.family(), Some(&row.model), Some(&listing), today) {
            if next.id() != row.model {
                moves.push((row.id.clone(), row.model.clone(), next.id().to_string()));
            }
        }
    }
    let mut changed = false;
    for (id, from, to) in moves {
        let preset = config.models.iter().find(|r| r.id == id).and_then(preset_for);
        let facts = preset.and_then(|p| snapshot.entry(p, &to));
        if facts.is_some_and(|facts| binding::follow_to(config, &id, &from, facts, today)) {
            f.arm(&id);
            changed = true;
        }
    }
    changed
}

/// An endpoint's first key: tick its starter families once, then record it.
/// An endpoint that already has rows is not a new connection (a save by the
/// released binary drops the record), so it only records. Without a list for
/// the key the key is not proven yet and nothing is recorded: the next key
/// save tries again.
pub(super) fn start(f: &Families, config: &mut Config, creds: &Credentials, (name, url): (&str, &str)) -> bool {
    let Some(endpoint) = Endpoint::of(name, url, creds) else { return false };
    let preset = endpoint.preset;
    if config.family_starts.contains(preset.key) {
        return false;
    }
    let Some(listing) = endpoint.key.as_ref().and_then(|_| endpoint.listing(f)) else { return false };
    let known = config.models.iter().any(|row| preset_for(row).is_some_and(|p| p.key == preset.key));
    let snapshot = f.catalog();
    let starters = preset.catalog.map_or(&[][..], |b| b.starter_families);
    for family in starters.iter().filter(|_| !known) {
        if let FamilyCurrent::Current { next, .. } = endpoint.ask(&snapshot, family, None, Some(&listing), f.today()) {
            if binding::row_of(config, preset.key, family).is_none() {
                binding::tick(config, preset, family, next, binding::Mode::Follow);
            }
        }
    }
    binding::mark_started(config, preset.key)
}

/// Whether this refusal on `model_id` should send a followed row back: the
/// row follows, its outstanding notice is a move onto the model it runs, and
/// the provider said the model is not there or not allowed.
pub(super) fn wants_go_back(config: &Config, model_id: &str, err: &anyhow::Error) -> bool {
    let Some(row) = config.models.iter().find(|row| row.id == model_id) else { return false };
    let Some(held) = row.family_binding.as_ref().filter(|b| b.follows()) else { return false };
    let Some(moved) = held.notice() else { return false };
    moved.kind() == binding::MoveKind::Followed && moved.to() == row.model && is_model_refusal(err)
}

/// "Model not found" and "not allowed", read from the refusal's own words:
/// a bad request or a refused account that names the model.
fn is_model_refusal(err: &anyhow::Error) -> bool {
    if !matches!(kind_of(err), ProviderErrorKind::BadRequest | ProviderErrorKind::Auth | ProviderErrorKind::Other) {
        return false;
    }
    let said = format!("{err:#}").to_lowercase();
    const REFUSED: &[&str] = &[
        "not_found", "not found", "does not exist", "no access", "not allowed", "not have access",
        "not available", "unknown model", "invalid model", "permission",
    ];
    said.contains("model") && REFUSED.iter().any(|w| said.contains(w))
}

/// Answer the queued refusals: each row still on the refused model goes back
/// to the one before, pinned, with its notice.
pub(super) fn go_back(f: &Families, config: &mut Config, queued: std::collections::BTreeSet<(String, String)>) -> bool {
    let snapshot = f.catalog();
    let today = f.today();
    let mut changed = false;
    for (id, refused) in queued {
        let Some(row) = config.models.iter().find(|row| row.id == id && row.model == refused) else { continue };
        let Some(moved) = row.family_binding.as_ref().and_then(|b| b.notice()).filter(|m| m.kind() == binding::MoveKind::Followed) else { continue };
        let (seq, from) = (moved.seq(), moved.from().to_string());
        let previous = preset_for(row).and_then(|p| snapshot.entry(p, &from));
        if binding::went_back(config, &id, seq, previous, today) {
            f.disarm(&id);
            changed = true;
        }
    }
    changed
}
