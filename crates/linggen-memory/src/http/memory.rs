//! `/api/memory/<method>` — RPC-style endpoints, 1:1 with the `Memory.*`
//! tools in Linggen. Each endpoint POSTs JSON args and returns
//! `{ok, data}` or `{ok:false, error, code}` via `envelope::ApiError`.
//!
//! Semantics mirror the CLI handlers in `crate::cli` — this module is
//! the network-facing path to the same `MemoryStore` operations. Once
//! Phase 4 lands in Linggen, the CLI data-ops wrappers are removed and
//! HTTP becomes the only dispatch path.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::{
    AccountScope, Filters, InsertOutcome, Memory, MemoryPatch, MemoryStore, MemoryType, Origin,
    Outcome, SortOrder, Tier,
};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, SubsecRound, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::sync::Arc;

/// Deserialize an `Option<T>` where empty strings, `null`, and missing
/// keys all collapse to `None`. Wraps any string-or-enum field that LLMs
/// commonly fill with `""` instead of omitting. Without this, a payload
/// like `{"type": "", "from": ""}` hits serde's enum parser and surfaces
/// as `422: premature end of input` — opaque, blocks the call.
fn deserialize_optional_lenient<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = serde_json::Value::deserialize(de)?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(ref s) if s.trim().is_empty() => Ok(None),
        other => serde_json::from_value(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

/// Deserialize `Option<DateTime<Utc>>` while tolerating the shapes LLMs
/// commonly produce. Without this, chat-generated date strings hit
/// chrono's strict parser and surface as an opaque `422: premature end
/// of input`.
///
/// Accepts:
/// - Field omitted / `null` / `""` → `None`
/// - Full RFC-3339 (`"2026-04-27T16:00:00Z"`) → parsed
/// - Date-only (`"2026-04-27"`) → midnight UTC of that date
/// - Date + time without TZ (`"2026-04-27T16:00:00"`) → assumed UTC
fn deserialize_optional_datetime<'de, D>(de: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    let s: Option<String> = Option::deserialize(de)?;
    let raw = match s.as_deref() {
        None | Some("") => return Ok(None),
        Some(s) => s.trim(),
    };
    if raw.is_empty() {
        return Ok(None);
    }

    // 1. Full RFC-3339 with timezone — the canonical form.
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(dt.with_timezone(&Utc)));
    }

    // 2. Date-only "YYYY-MM-DD" → midnight UTC.
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        if let Some(naive) = date.and_hms_opt(0, 0, 0) {
            return Ok(Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)));
        }
    }

    // 3. Date + time without timezone "YYYY-MM-DDTHH:MM:SS" → assume UTC.
    //    Some LLMs drop the trailing 'Z'. Accept it to avoid the 422.
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        return Ok(Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)));
    }

    Err(D::Error::custom(format!(
        "invalid timestamp {raw:?}: expected RFC-3339 (e.g. \"2026-04-27T16:00:00Z\") or date-only (\"2026-04-27\")"
    )))
}

/// Serialize a fact for HTTP response, stripping the 1024-dim embedding
/// vector. Callers never need the raw vector over the wire, and including
/// it bloats every response by ~13 KB / row (noisy for the model, for logs,
/// and for the data UI). The CLI's NDJSON output keeps vectors — they
/// matter for `add --stdin` round-trips.
fn fact_public(fact: &Memory) -> Value {
    let mut v = serde_json::to_value(fact).unwrap_or_else(|_| Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("vector");
    }
    v
}

fn facts_public(facts: &[Memory]) -> Vec<Value> {
    facts.iter().map(fact_public).collect()
}

/// Like [`fact_public`] but adds the relevance fields for search responses.
/// Each input hit is `(memory, cosine, hybrid)`:
///
/// - `score` — the raw **cosine** similarity (`[0,1]`), the absolute
///   dense-relevance signal. Kept for the recall hook, CLI, and cross-host
///   comparisons that already read it.
/// - `hybrid_score` — the blended relevance (`cosine` + IDF-weighted keyword
///   boost, clamped to `[0,1]`; see [`crate::memory::hybrid`]). This is the
///   number the console displays: it is what the rows are ordered by, so it
///   is monotonic with rank, and it is absolute — an unrelated query does
///   not get a fake 1.0.
fn scored_facts_public(scored: &[(Memory, f32, f32)]) -> Vec<Value> {
    scored
        .iter()
        .map(|(f, cosine, hybrid)| {
            let mut v = fact_public(f);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("score".into(), json!(cosine));
                obj.insert("hybrid_score".into(), json!(hybrid));
            }
            v
        })
        .collect()
}

/// Memory subrouter. Mounted at `/api/memory/` by the parent router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/memory/add", post(add))
        .route("/api/memory/add_batch", post(add_batch))
        .route("/api/memory/get", post(get))
        .route("/api/memory/search", post(search))
        .route("/api/memory/list", post(list))
        .route("/api/memory/count", post(count))
        .route("/api/memory/update", post(update))
        .route("/api/memory/delete", post(delete))
        .route("/api/memory/forget", post(forget))
        .route("/api/memory/restamp", post(restamp))
        .route("/api/memory/accounts", post(accounts))
}

// ── Request DTOs ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddRequest {
    pub content: String,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub r#type: Option<MemoryType>,
    /// Origin. Canonical name is `from` (matches the `Memory` field);
    /// accept `origin` as an alias for callers that avoid reserved words.
    #[serde(
        default,
        alias = "origin",
        deserialize_with = "deserialize_optional_lenient"
    )]
    pub from: Option<Origin>,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub outcome: Option<Outcome>,
    /// HOST-FILLED, request only: the session's cwd — the row's stored
    /// `scope` when the model names none, and the base the named one is
    /// checked against. Never stored under this name.
    pub cwd: Option<String>,
    /// The model's choice of scope: one of the candidates the host showed
    /// (`skills/lingjing`, `~/workspace`, …), resolved against `root`. Must
    /// be an existing directory inside root or a parent of root below
    /// `$HOME`; anything else falls back to `cwd`. In `add_batch` (import)
    /// it is the stored, absolute `scope` and is taken as given.
    #[serde(default)]
    pub scope: Option<String>,
    /// HOST-FILLED: the session's root (git root, or where it started).
    #[serde(default)]
    pub root: Option<String>,
    /// One line (≤ 80 chars) saying what the row is for — what the index
    /// shows for an indexed row.
    #[serde(default)]
    pub summary: Option<String>,
    /// Put the row in its directory's index.
    #[serde(default)]
    pub indexed: bool,
    /// The row is about the person, not the project: store it with no
    /// `scope`, whatever a host stamped. Wins over `cwd` — the stamp hooks
    /// fill `cwd` mechanically, and this is the model saying the stamp is
    /// wrong for this row.
    #[serde(default)]
    pub global: bool,
    #[serde(default, deserialize_with = "deserialize_optional_datetime")]
    pub occurred_at: Option<DateTime<Utc>>,
    pub source_session: Option<String>,
    /// Writing host identifier (`linggen`, `claude-code`, `codex`, …).
    /// Distinct from `from` (which captures whether the content came
    /// from `user` / `agent` / `derived`); this records which tool
    /// runtime committed the row. Optional — older callers omit it.
    pub host: Option<String>,
    /// Route the write to the **episodic** staging store instead of the
    /// default curated semantic table. Used by the per-session encoder
    /// subagent so episodic writes can flow through HTTP (no more direct
    /// LanceDB-via-CLI roundtrip from inside the engine).
    #[serde(default)]
    pub episodic: bool,
    /// Destination tier within the semantic table: `core` (always-on
    /// identity set) or `semantic` (default). `episodic` here is a
    /// dispatch-boundary alias for the `episodic` flag above (hosts pass
    /// `tier=episodic`; the engine converts, but direct callers may not).
    /// Before this field existed, `tier` in the body was silently
    /// dropped — `tier=core` over HTTP/MCP never worked.
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub tier: Option<Tier>,
    /// Bypass dedup: insert as a new row even if a near-duplicate exists.
    /// Accepts `skip_dedup` (canonical) or `force` (alias).
    #[serde(default, alias = "force")]
    pub skip_dedup: bool,
    /// Atomic contradiction-resolution helper. When set, the daemon
    /// inserts the new row AND deletes every id in this list in the same
    /// call — used by the agent after an `AskUser`-confirmed conflict
    /// resolution. Lookup spans both tables, so the caller doesn't need
    /// to know which tier each loser lives in. Empty / omitted = a plain
    /// add. Failed deletes are reported in the response under
    /// `replaced_failed` but never abort the insert.
    #[serde(default)]
    pub replace_ids: Vec<String>,
    /// Assert that the user directed this change — their current message
    /// states it as settled (a command, declaration, or commitment), or
    /// the host resolved the conflict with them via an ask. Required when
    /// `replace_ids` targets rows in the user's voice (`from=user`); the
    /// daemon refuses such writes otherwise (the merge law's floor,
    /// enforced at the store so no frontend can silently rewrite the
    /// user's voice). Derived-row replaces never need it.
    #[serde(default)]
    pub user_directed: bool,
    /// Whose memory this is. Absent = the store owner's. Set by the
    /// engine for a row arriving from another person's paired phone —
    /// Mac-minted from the pairing record, never taken from the phone's
    /// own claim — as the account id, or `device:<id>` while that phone
    /// is signed out. See `Memory::account_id`.
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub account_name: Option<String>,
}

/// Bulk insert. Each element is a plain [`AddRequest`]; the whole batch is
/// embedded in one serialized forward-pass sequence and written with a
/// single LanceDB commit per table. This is the `ling-mem add --stdin`
/// path: one HTTP call for the entire import instead of one POST (and one
/// commit, one version) per row — which is what made large imports degrade
/// super-linearly and eventually trip the client timeout.
///
/// Bulk semantics match the direct-store `cmd_add` stdin path: always plain
/// insert (no dedup), `replace_ids` is not honored here (it's a single-row
/// conflict-resolution helper).
#[derive(Debug, Deserialize)]
pub struct AddBatchRequest {
    pub facts: Vec<AddRequest>,
}

#[derive(Debug, Deserialize)]
pub struct GetRequest {
    pub id: String,
    /// `Some(true)` = episodic only, `Some(false)` = semantic only,
    /// **`None` (default) = span both tables**. Returns the first match.
    /// Callers used to need a 404-then-retry dance; the daemon now does
    /// that internally.
    #[serde(default)]
    pub episodic: Option<bool>,
    #[serde(flatten)]
    pub scope: AccountScopeDTO,
}

/// Whose rows a by-id verb may touch. Absent = the store owner's, so a
/// row that belongs to another person answers "not found" rather than
/// being read, edited or deleted across the line. The engine fills it
/// for a phone; local callers never need to.
#[derive(Debug, Default, Deserialize)]
pub struct AccountScopeDTO {
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub all_accounts: bool,
}

impl AccountScopeDTO {
    fn scope(self) -> AccountScope {
        AccountScope::from_args(self.account, self.all_accounts)
    }
}

/// Filter block shared by `search`, `list`, and `forget`. All fields
/// optional; an empty block matches every row. Every enum-typed field
/// accepts the lowercase variant name (`"fact"`, `"positive"`, …).
#[derive(Debug, Default, Deserialize)]
pub struct FilterDTO {
    /// Rows of any of these skills (`~/.linggen/skills/<name>` and below).
    /// The phone's pull: "rows of an app I run".
    #[serde(default)]
    pub apps: Vec<String>,
    /// Deprecated spelling of `apps` (phones built before 2026-10-04 send
    /// it). Read as app names, so an old phone is never answered with the
    /// whole store.
    #[serde(default)]
    pub contexts_any: Vec<String>,
    /// Only rows with this `indexed` flag.
    #[serde(default)]
    pub indexed: Option<bool>,
    /// Whose rows. Absent = the store owner's; an account id = that
    /// person's; `all_accounts: true` = everyone's (maintenance only).
    #[serde(flatten)]
    pub scope: AccountScopeDTO,
    /// Narrow to one `MemoryType`. Linggen's tool schema is singular;
    /// internally we convert to `Filters.types: Vec<MemoryType>`.
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub r#type: Option<MemoryType>,
    /// Narrow to any of these types (OR), alongside the singular `type`.
    #[serde(default)]
    pub types: Vec<MemoryType>,
    /// Drop rows of these types.
    #[serde(default)]
    pub exclude_types: Vec<MemoryType>,
    /// Narrow to one tier (`core` or `semantic`). Within the semantic
    /// table both tiers coexist; this filter lets callers ask for "just
    /// the always-on identity set" (`tier=core`) or "everything else"
    /// (`tier=semantic`). Ignored for episodic queries — episodic rows
    /// don't carry a meaningful tier.
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub tier: Option<Tier>,
    #[serde(
        default,
        alias = "origin",
        deserialize_with = "deserialize_optional_lenient"
    )]
    pub from: Option<Origin>,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub outcome: Option<Outcome>,
    #[serde(default, deserialize_with = "deserialize_optional_datetime")]
    pub since: Option<DateTime<Utc>>,
    /// Upper bound on `COALESCE(occurred_at, created_at)`. `older_than`
    /// is accepted as an alias (legacy shape from the v0.1 translate_args
    /// table in Linggen core).
    #[serde(
        default,
        alias = "older_than",
        deserialize_with = "deserialize_optional_datetime"
    )]
    pub until: Option<DateTime<Utc>>,
    /// Narrow to one `source_session` id. Powers dashboard deep-links
    /// like `?session=<sid>` so the user can drill into rows the agent
    /// wrote during one engine session.
    #[serde(default)]
    pub source_session: Option<String>,
    /// The session's recall scope, given as its root (see
    /// `Filters::scope_root`): an owner root sees its subtree, its parents and
    /// person rows; a skill's dir only its own rows; `$HOME`/`~/.linggen`/temp
    /// person rows plus at most two strong non-preference project rows.
    ///
    /// Deliberately NOT accepted by `forget` as a standalone filter: it matches
    /// every unscoped row by design, so a delete carrying only this would take
    /// most of the store. The empty-filter guard below does not list it.
    /// `cwd_scope` is its pre-v2 name, accepted for one release.
    #[serde(default, alias = "cwd_scope")]
    pub scope_root: Option<String>,
    /// `true` = apply the daemon's configured `episodic_ttl_days` as an
    /// upper bound on `occurred_at` (i.e. "rows that are past their
    /// TTL"). Resolved at handler entry and folded into `until`. The
    /// dream consolidator sets this so the mission body never has to
    /// know the live TTL value. Opt-in: default `false` keeps every
    /// existing caller (dashboard, CLI list, etc.) unchanged.
    #[serde(default)]
    pub past_ttl: bool,
    /// One **local calendar day**, `YYYY-MM-DD` — sugar over
    /// `since`/`until` covering exactly that day. The dream pipeline's
    /// remember stage lists one day's worklist with this. Explicit
    /// `since`/`until` win; `day` only fills what's unset.
    #[serde(default)]
    pub day: Option<String>,
    /// Include archived rows (`expired_at IS NOT NULL`) — losers a
    /// `replace_ids` merge or digest expired out of live memory. Default
    /// `false`: the archive serves provenance and unpack, never recall.
    #[serde(default)]
    pub include_expired: bool,
    /// Unpack query: only the archived rows this survivor id replaced.
    /// Implies `include_expired`.
    #[serde(default)]
    pub superseded_by: Option<String>,
}

impl FilterDTO {
    /// Fold the `day` sugar into `since`/`until`. Called by every
    /// handler before `into_filters` — a bad day string is a 400, not a
    /// silently-ignored filter.
    fn resolve_day(&mut self) -> Result<(), ApiError> {
        let Some(day) = self.day.take() else {
            return Ok(());
        };
        let date = chrono::NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").map_err(|_| {
            ApiError::bad_request(format!("invalid day {day:?}: expected YYYY-MM-DD"))
        })?;
        let (start, end) = super::days::local_day_bounds(date);
        if self.since.is_none() {
            self.since = Some(start);
        }
        if self.until.is_none() {
            self.until = Some(end);
        }
        Ok(())
    }

    fn into_filters(mut self) -> Result<Filters, ApiError> {
        self.resolve_day()?;
        let mut types = self.types;
        if let Some(t) = self.r#type {
            if !types.contains(&t) {
                types.push(t);
            }
        }
        let mut apps = self.apps;
        for a in self.contexts_any {
            if !apps.contains(&a) {
                apps.push(a);
            }
        }
        Ok(Filters {
            apps,
            indexed: self.indexed,
            scope_in: Vec::new(),
            scoped_only: false,
            legacy_contexts: false,
            legacy_cwd: false,
            account: self.scope.scope(),
            types,
            exclude_types: self.exclude_types,
            origin: self.from,
            outcome: self.outcome,
            since: self.since,
            until: self.until,
            created_since: None,
            tier: self.tier,
            source_session: self.source_session,
            scope_root: self.scope_root,
            include_expired: self.include_expired,
            superseded_by: self.superseded_by,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(flatten)]
    pub filters: FilterDTO,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// Drop rows whose cosine similarity to the query falls below this
    /// threshold. Range `[-1.0, 1.0]`; in practice Qwen3-Embedding-0.6B
    /// outputs land in `[0.0, 1.0]`. Omit to disable filtering.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Which table(s) to query. Default is `both` (cross-table recall —
    /// the consolidator subagent relies on this). `semantic` / `episodic`
    /// constrain to a single table so a tab-scoped UI does not double-
    /// fetch and merge the same rows twice. Note: this selects the
    /// **table**, not the `tier` field; `tier=core` filtering inside the
    /// semantic table is a separate parameter.
    #[serde(default)]
    pub table: SearchTable,
    /// Table-scope alias matching every other CRUD DTO: `true` = episodic
    /// only, `false` = semantic only. Overrides `table` when set. This is
    /// the field the MCP dispatch shim produces from `tier=episodic` —
    /// before it existed, an episodic-scoped MCP search silently searched
    /// BOTH tables (the flag landed as an ignored unknown field).
    #[serde(default)]
    pub episodic: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchTable {
    #[default]
    Both,
    Semantic,
    Episodic,
}

fn default_search_limit() -> usize {
    10
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDTO {
    #[default]
    Newest,
    Oldest,
}

impl From<SortDTO> for SortOrder {
    fn from(v: SortDTO) -> Self {
        match v {
            SortDTO::Newest => SortOrder::Newest,
            SortDTO::Oldest => SortOrder::Oldest,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListRequest {
    #[serde(flatten)]
    pub filters: FilterDTO,
    #[serde(default)]
    pub sort: SortDTO,
    #[serde(default = "default_list_limit")]
    pub limit: usize,
    /// Number of rows to skip in the sorted result. `0` = first page.
    #[serde(default)]
    pub offset: usize,
    /// `Some(true)` = episodic only, `Some(false)` = semantic only,
    /// **`None` (default) = span both tables**. Merged + sorted +
    /// limited as one result set.
    #[serde(default)]
    pub episodic: Option<bool>,
    /// With `day`: only the rows no remember pass has judged — created at
    /// or after the day's `remembered_at`, or all of them when the day was
    /// never remembered. One late row must not re-read a judged day.
    #[serde(default)]
    pub unjudged: bool,
}

fn default_list_limit() -> usize {
    50
}

/// Lightweight summary endpoint used by the memory dashboard. Returns
/// the row count + the most recent row's `created_at` per filter set,
/// so the on-open UI can render tier cards (Core / Semantic / Episodic)
/// without paging through actual rows. `count` is metadata-only at the
/// LanceDB layer; `latest_created_at` costs one row fetch.
///
/// Same scope rules as `list`: `episodic` `None` spans both stores,
/// `Some(true)` episodic only, `Some(false)` semantic only.
#[derive(Debug, Deserialize)]
pub struct CountRequest {
    #[serde(flatten)]
    pub filters: FilterDTO,
    #[serde(default)]
    pub episodic: Option<bool>,
}

/// Update semantics mirror the CLI: explicit set-vs-clear via twin
/// fields (`outcome` / `clear_outcome`, `scope` / `clear_scope`). Absent
/// fields mean "leave unchanged." Set wins over clear if both are given.
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub id: String,
    pub content: Option<String>,
    /// New one-line summary; `clear_summary` removes it.
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub clear_summary: bool,
    /// Put the row in (true) or take it out of (false) its dir's index.
    #[serde(default)]
    pub indexed: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub r#type: Option<MemoryType>,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub tier: Option<Tier>,
    #[serde(
        default,
        alias = "origin",
        deserialize_with = "deserialize_optional_lenient"
    )]
    pub from: Option<Origin>,
    #[serde(default, deserialize_with = "deserialize_optional_lenient")]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub clear_outcome: bool,
    /// The stored scope: the absolute directory the row is about. `cwd` is
    /// its pre-v2 name, accepted for one release.
    #[serde(default, rename = "scope", alias = "cwd")]
    pub scope_dir: Option<String>,
    #[serde(default, alias = "clear_cwd")]
    pub clear_scope: bool,
    /// Make the row global: clear its `scope`, so it applies in every
    /// project. Same effect as `clear_scope`; the name the model is given.
    #[serde(default)]
    pub global: bool,
    pub host: Option<String>,
    #[serde(default)]
    pub clear_host: bool,
    /// When the remembered thing happened (recall sorts by it). Was silently
    /// dropped for the endpoint's whole life — the store's patch always
    /// carried it, only this DTO never asked — which is how a backdate
    /// "succeeded" while changing nothing.
    #[serde(default, deserialize_with = "deserialize_optional_datetime")]
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub clear_occurred_at: bool,
    /// `Some(true)` = episodic only, `Some(false)` = semantic only,
    /// **`None` (default) = locate the id in whichever table holds it**
    /// before applying the patch.
    #[serde(default)]
    pub episodic: Option<bool>,
    /// See [`AddRequest::user_directed`]. Required when this update
    /// rewrites `content` on a `from=user` row; metadata-only patches
    /// (tier, scope, summary, indexed) stay unguarded.
    #[serde(default)]
    pub user_directed: bool,
    #[serde(flatten)]
    pub scope: AccountScopeDTO,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub id: String,
    /// `Some(true)` = episodic only, `Some(false)` = semantic only,
    /// **`None` (default) = locate the id in whichever table holds it**
    /// before deleting.
    #[serde(default)]
    pub episodic: Option<bool>,
    #[serde(flatten)]
    pub scope: AccountScopeDTO,
}

/// Move every row stamped `from` to an account — the once-only re-stamp
/// when a phone that wrote under its device id signs in.
#[derive(Debug, Deserialize)]
pub struct RestampRequest {
    /// The stamp to replace, e.g. `device:<id>`.
    pub from: String,
    /// The account to move the rows to. Absent or empty = the store owner
    /// (the rows lose their stamp): the phone that signed in was the
    /// owner's all along.
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub account_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ForgetRequest {
    #[serde(flatten)]
    pub filters: FilterDTO,
    /// Target the episodic table instead of the default semantic table.
    #[serde(default)]
    pub episodic: bool,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Pick the semantic or episodic store based on the request's `episodic`
/// flag. Centralized so every CRUD endpoint routes consistently.
fn pick_store(state: &SharedState, episodic: bool) -> Arc<MemoryStore> {
    if episodic {
        Arc::clone(&state.episodic)
    } else {
        Arc::clone(&state.store)
    }
}

/// Resolve which store(s) a read/edit/delete should hit.
///   - `Some(true)`  → episodic only
///   - `Some(false)` → semantic only
///   - `None`        → both (caller decides how to merge)
///
/// Returned in the order semantic-first so locate-first-hit semantics
/// preserve the higher-tier preference when an id collision exists
/// across tables.
fn stores_for_read(state: &SharedState, episodic: Option<bool>) -> Vec<Arc<MemoryStore>> {
    match episodic {
        Some(true) => vec![Arc::clone(&state.episodic)],
        Some(false) => vec![Arc::clone(&state.store)],
        None => vec![Arc::clone(&state.store), Arc::clone(&state.episodic)],
    }
}

async fn add(
    State(state): State<SharedState>,
    Json(req): Json<AddRequest>,
) -> Result<Response, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    guard_user_voice(&state, &req.replace_ids, req.user_directed).await?;

    let skip_dedup = req.skip_dedup;
    let replace_ids = req.replace_ids.clone();
    // The rows this one replaces: their tier and scope carry over.
    let losers = locate_rows(&state, &replace_ids).await?;
    let tier = resolve_tier(req.tier, req.episodic, &losers, req.indexed);
    let episodic = tier == Tier::Episodic;
    let scope = resolve_scope(&req, &losers, tier);
    let mut fact = Memory::new(
        req.content,
        req.r#type.unwrap_or(MemoryType::Fact),
        req.from.unwrap_or_default(),
    );
    fact.tier = tier;
    fact.outcome = req.outcome;
    fact.scope = scope;
    fact.summary = clean_summary(req.summary);
    fact.indexed = req.indexed && tier != Tier::Core;
    fact.occurred_at = req.occurred_at;
    fact.source_session = req.source_session;
    fact.host = req.host;
    fact.account_id = req.account_id.filter(|a| !a.trim().is_empty());
    fact.account_name = req.account_name.filter(|a| !a.trim().is_empty());

    // Embed the content so the row is immediately searchable. Serialized +
    // off the async workers so concurrent adds can't stack forward passes.
    let vector = state
        .embedder()
        .await?
        .embed_passage(fact.content.clone())
        .await
        .map_err(ApiError::internal)?;
    fact.vector = Some(vector);

    let store = pick_store(&state, episodic);
    if skip_dedup {
        store.insert(std::slice::from_ref(&fact)).await?;
        let body = json!({
            "action": "added",
            "fact": fact_public(&fact),
        });
        return Ok(ok(
            apply_replace_ids(&state, &replace_ids, &fact.id, body).await
        ));
    }

    // Cross-tier dedup. The single-table `insert_with_dedup` below only
    // catches duplicates inside the chosen store; without this step, a
    // semantic copy and an episodic copy of byte-identical (content, type)
    // would coexist. Tier rank: Core > Semantic > Episodic. The higher-
    // tier row wins; on equal rank (e.g. both semantic) the in-table
    // dedup handles it. See linggen `agents/ling-mem.md` tier-discipline.
    let other = if episodic {
        Arc::clone(&state.store)
    } else {
        Arc::clone(&state.episodic)
    };
    if let Some(existing) = other
        .find_exact_content_public(&fact.content, fact.r#type, &replace_ids)
        .await?
    {
        let new_rank = tier_rank(fact.tier);
        let existing_rank = tier_rank(existing.tier);
        if existing_rank >= new_rank {
            // Existing row is at the same or higher tier — keep it. Fill
            // what it lacks from the new write, then return as if dedup'd.
            let merged = merge_with_existing(&existing, &fact);
            other.update_full_public(&existing.id, &merged).await?;
            let body = json!({
                "action": "merged",
                "similarity": 1.0,
                "previous_id": existing.id,
                "fact": fact_public(&merged),
            });
            return Ok(ok(apply_replace_ids(
                &state,
                &replace_ids,
                &existing.id,
                body,
            )
            .await));
        }
        // New row is at a higher tier — promote: delete the lower-tier
        // copy from the other table, then proceed with single-table insert.
        let _ = other.delete(&existing.id).await?;
    }

    // Never into a row this call replaces: that row is about to be retired.
    let outcome = store.insert_with_dedup_except(fact, &replace_ids).await?;
    let survivor = match &outcome {
        crate::memory::InsertOutcome::Added(f) => f.id.clone(),
        crate::memory::InsertOutcome::Merged { fact, .. } => fact.id.clone(),
    };
    let body = apply_replace_ids(&state, &replace_ids, &survivor, outcome_public(&outcome)).await;
    Ok(ok(body))
}

/// The rows `replace_ids` names, wherever they live (missing ids skipped).
async fn locate_rows(state: &SharedState, ids: &[String]) -> Result<Vec<Memory>, ApiError> {
    let mut found = Vec::new();
    for id in ids {
        for store in stores_for_read(state, None) {
            if let Some(row) = store.get(id).await? {
                found.push(row);
                break;
            }
        }
    }
    Ok(found)
}

/// The tier a write lands in. Asked-for wins (`episodic: true` or `tier`);
/// a replacement keeps its losers' highest tier; an indexed row is
/// long-term by nature; anything else is episodic — the protocol's default
/// per-turn capture, which the nightly dream promotes from.
fn resolve_tier(asked: Option<Tier>, episodic: bool, losers: &[Memory], indexed: bool) -> Tier {
    if episodic {
        return Tier::Episodic;
    }
    if let Some(t) = asked {
        return t;
    }
    if let Some(t) = losers.iter().map(|l| l.tier).max_by_key(|t| tier_rank(*t)) {
        return t;
    }
    if indexed {
        return Tier::Semantic;
    }
    Tier::Episodic
}

/// The scope a new row is stored with. Core rows and `global` rows have
/// none. A `scope` the model named wins when it resolves to a valid dir for
/// this session; a replacement with no named scope takes its losers' common
/// scope; otherwise the host's stamped session cwd, if it can be a scope.
fn resolve_scope(req: &AddRequest, losers: &[Memory], tier: Tier) -> Option<String> {
    if req.global || tier == Tier::Core {
        return None;
    }
    let home = crate::memory::scope::home();
    let session = req
        .cwd
        .as_deref()
        .and_then(|c| crate::memory::scope::expand(c, &home))
        .filter(|c| crate::memory::scope::is_scope_dir(c, &home));
    let root = req
        .root
        .as_deref()
        .and_then(|r| crate::memory::scope::expand(r, &home))
        .or_else(|| {
            session
                .as_deref()
                .map(|s| crate::memory::scope::find_root(s, &home))
        });
    if let Some(named) = req.scope.as_deref().filter(|s| !s.trim().is_empty()) {
        let chosen = match root.as_deref() {
            Some(r) => crate::memory::scope::resolve(named, Some(r), &home)
                .filter(|d| crate::memory::scope::is_valid_for(d, r, &home)),
            // No host stamp (Codex cannot rewrite tool input): nothing to
            // check the choice against, and falling back would store the row
            // with no scope — "about the person", recalled everywhere — the
            // opposite of what naming a directory says. An absolute (or ~/)
            // existing directory that can hold rows is taken as named.
            None => crate::memory::scope::resolve(named, None, &home)
                .filter(|d| d.is_dir() && crate::memory::scope::is_scope_dir(d, &home)),
        };
        if let Some(dir) = chosen {
            return Some(dir.to_string_lossy().to_string());
        }
    } else if !losers.is_empty() {
        let scopes: Vec<Option<String>> = losers.iter().map(|l| l.scope.clone()).collect();
        return crate::memory::scope::common(&scopes, &home);
    }
    session.map(|s| s.to_string_lossy().to_string())
}

/// The longest a summary may be, in characters.
const SUMMARY_MAX_CHARS: usize = 80;

/// One line, trimmed, at most [`SUMMARY_MAX_CHARS`] — cut with `…` beyond.
fn clean_summary(raw: Option<String>) -> Option<String> {
    let line = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= SUMMARY_MAX_CHARS {
        return Some(line);
    }
    let cut: String = line.chars().take(SUMMARY_MAX_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// The `scope` a batch-imported row is stored with: `global` wins, an empty
/// string is no scope.
fn written_scope(scope: Option<String>, global: bool) -> Option<String> {
    if global {
        None
    } else {
        scope.filter(|c| !c.trim().is_empty())
    }
}

/// An update's `scope` change. `global` clears it and wins over a new value;
/// otherwise set wins over clear, and nothing given leaves it alone.
fn scope_patch(scope: Option<String>, clear: bool, global: bool) -> Option<Option<String>> {
    match (scope, clear) {
        _ if global => Some(None),
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    }
}

/// Bulk insert N rows in one call: build every row, batch-embed all
/// contents (one gate acquisition), then commit each table's rows with a
/// single `MemoryStore::insert` (one LanceDB version per table, not per
/// row). Empty-content rows are skipped rather than failing the batch.
async fn add_batch(
    State(state): State<SharedState>,
    Json(req): Json<AddBatchRequest>,
) -> Result<Response, ApiError> {
    // Partition by target table up front so each table commits exactly once.
    let mut semantic: Vec<Memory> = Vec::new();
    let mut episodic: Vec<Memory> = Vec::new();
    for r in req.facts {
        if r.content.trim().is_empty() {
            continue; // skip blanks; don't abort the whole import for one
        }
        let tier = resolve_tier(r.tier, r.episodic, &[], r.indexed);
        let mut fact = Memory::new(
            r.content,
            r.r#type.unwrap_or(MemoryType::Fact),
            r.from.unwrap_or_default(),
        );
        fact.tier = tier;
        fact.outcome = r.outcome;
        fact.scope = written_scope(r.scope.or(r.cwd), r.global || tier == Tier::Core);
        fact.summary = clean_summary(r.summary);
        fact.indexed = r.indexed && tier != Tier::Core;
        fact.occurred_at = r.occurred_at;
        fact.source_session = r.source_session;
        fact.host = r.host;
        if tier == Tier::Episodic {
            episodic.push(fact);
        } else {
            semantic.push(fact);
        }
    }

    let mut added = Vec::new();
    for (mut rows, is_episodic) in [(semantic, false), (episodic, true)] {
        if rows.is_empty() {
            continue;
        }
        // One serialized batch embed for the whole group (chunked to the
        // embedder's MAX_EMBED_BATCH internally), then one commit.
        let texts: Vec<String> = rows.iter().map(|f| f.content.clone()).collect();
        let vectors = state
            .embedder()
            .await?
            .embed_passages(texts)
            .await
            .map_err(ApiError::internal)?;
        for (f, v) in rows.iter_mut().zip(vectors) {
            f.vector = Some(v);
        }
        pick_store(&state, is_episodic).insert(&rows).await?;
        added.extend(rows.iter().map(fact_public));
    }

    Ok(ok(json!({
        "action": "added",
        "count": added.len(),
        "facts": added,
    })))
}

/// The merge law's floor, enforced at the store: a write that replaces
/// (`add` + `replace_ids`) or rewrites (`update` + `content`) rows in the
/// USER'S VOICE (`from=user`) is refused unless the caller asserts
/// `user_directed: true` — the user's current message states the change
/// as settled, or the host resolved the conflict with the user via an
/// ask. Hosts justify the flag; the daemon guarantees no silent
/// user-voice rewrite reaches the store regardless of which frontend
/// wrote it (the Linggen engine has its own pre-flight guard and
/// forwards the flag; CLI/MCP hosts pass it directly). Derived rows
/// pass free. A missing id is not the guard's problem — the real call
/// reports it.
async fn guard_user_voice(
    state: &SharedState,
    target_ids: &[String],
    user_directed: bool,
) -> Result<(), ApiError> {
    if user_directed || target_ids.is_empty() {
        return Ok(());
    }
    let mut offending = Vec::new();
    for id in target_ids {
        for store in [Arc::clone(&state.store), Arc::clone(&state.episodic)] {
            if let Some(fact) = store.get(id).await? {
                if fact.origin == crate::memory::Origin::User {
                    let gist: String = fact.content.chars().take(60).collect();
                    offending.push(format!("{id} \"{gist}\""));
                }
                break;
            }
        }
    }
    if offending.is_empty() {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "BLOCKED by the user-voice merge guard: this write replaces/rewrites rows in the USER'S VOICE (from=user): {}. The user's voice changes only with the user (merge law). Recovery — pick ONE: (1) if the user's CURRENT message states this change as SETTLED — a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\") — retry the exact same call with \"user_directed\": true; a hedged reflection (\"X feels about right to me\") does NOT qualify; (2) otherwise ask the user to choose (the host's ask-user primitive, or a plain question in chat), then retry with \"user_directed\": true after their answer. Do NOT work around it by writing a new row without replace_ids — that creates the drift this guard exists to prevent.",
        offending.join(", ")
    )))
}

/// Apply the caller's `replace_ids` deletion sweep across both tables and
/// fold the outcome into `body`.
///
/// `replace_ids` are the losers the user picked against via AskUser; they
/// are deleted regardless of which tier they live in (the caller doesn't
/// track tier). Failures are surfaced (`replaced_failed`) but never abort
/// the add — the insert is the load-bearing operation.
///
/// This runs on **every** add return path (added / merged / promoted). It
/// used to be inline before only the final return, so an AskUser-resolved
/// conflict whose winner happened to dedup-merge into an existing row would
/// silently keep all the losers. Hoisting it into a shared helper closes
/// that gap.
/// Retire the losers of a merge. Semantic-table losers are EXPIRED, not
/// deleted (2026-08-17, Hanli's call): stamped `expired_at` +
/// `superseded_by = successor`, they leave every default read but stay on
/// disk — a merge or digest is reversible via the unpack query
/// (`superseded_by` filter). Episodic losers stay hard-deleted: staging
/// is disposable by design, and an immortal staged row would outlive the
/// sweep that exists to kill it.
async fn apply_replace_ids(
    state: &SharedState,
    replace_ids: &[String],
    successor: &str,
    mut body: serde_json::Value,
) -> serde_json::Value {
    if replace_ids.is_empty() {
        return body;
    }
    let mut replaced = Vec::new();
    let mut replaced_failed = Vec::new();
    for id in replace_ids {
        // The row this write landed in is never its own loser.
        if id == successor {
            continue;
        }
        let retired = state.store.expire(id, successor).await.unwrap_or(false)
            || state.episodic.delete(id).await.unwrap_or(false);
        if retired {
            replaced.push(id.clone());
        } else {
            replaced_failed.push(id.clone());
        }
    }
    if let Some(obj) = body.as_object_mut() {
        obj.insert("replaced".to_string(), json!(replaced));
        if !replaced_failed.is_empty() {
            obj.insert("replaced_failed".to_string(), json!(replaced_failed));
        }
    }
    body
}

/// Tier rank for the "higher wins on cross-tier dedup" rule.
/// Higher number = higher tier. Core (2) > Semantic (1) > Episodic (0).
fn tier_rank(tier: crate::memory::Tier) -> u8 {
    use crate::memory::Tier::*;
    match tier {
        Core => 2,
        Semantic => 1,
        Episodic => 0,
    }
}

/// Same merge logic as `store::merge_fact` but at the HTTP layer so we
/// can compose it with a cross-store update. Takes the longer content and
/// fills missing optional fields from the candidate. The surviving row's
/// `scope` and summary stay unless empty: a fact re-said from elsewhere
/// must not leave the directory it is about.
fn merge_with_existing(
    existing: &crate::memory::Memory,
    candidate: &crate::memory::Memory,
) -> crate::memory::Memory {
    let mut merged = existing.clone();
    if candidate.content.len() > existing.content.len() {
        merged.content = candidate.content.clone();
        merged.vector = candidate.vector.clone();
    }
    if candidate.outcome.is_some() {
        merged.outcome = candidate.outcome;
    }
    if merged.scope.is_none() && merged.tier != crate::memory::Tier::Core {
        merged.scope = candidate.scope.clone();
    }
    if merged.summary.is_none() {
        merged.summary = candidate.summary.clone();
    }
    merged.indexed |= candidate.indexed;
    if candidate.occurred_at.is_some() {
        merged.occurred_at = candidate.occurred_at;
    }
    if candidate.source_session.is_some() {
        merged.source_session = candidate.source_session.clone();
    }
    if candidate.host.is_some() {
        merged.host = candidate.host.clone();
    }
    merged
}

/// Wrap an [`InsertOutcome`] as the JSON payload returned by the add
/// endpoint. Always includes `action` and `fact`; on a merge also
/// includes `similarity` and `previous_id`.
fn outcome_public(outcome: &InsertOutcome) -> Value {
    match outcome {
        InsertOutcome::Added(f) => json!({
            "action": "added",
            "fact": fact_public(f),
        }),
        InsertOutcome::Merged {
            fact,
            similarity,
            previous_id,
        } => json!({
            "action": "merged",
            "similarity": similarity,
            "previous_id": previous_id,
            "fact": fact_public(fact),
        }),
    }
}

async fn get(
    State(state): State<SharedState>,
    Json(req): Json<GetRequest>,
) -> Result<Response, ApiError> {
    let scope = req.scope.scope();
    for store in stores_for_read(&state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            if !scope.admits(&fact) {
                break;
            }
            return Ok(ok(fact_public(&fact)));
        }
    }
    Err(ApiError::not_found(format!("no fact with id {}", req.id)))
}

async fn search(
    State(state): State<SharedState>,
    Json(req): Json<SearchRequest>,
) -> Result<Response, ApiError> {
    if req.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }

    let vector = state
        .embedder()
        .await?
        .embed_query_serialized(req.query.clone())
        .await
        .map_err(ApiError::internal)?;
    let filters = req.filters.into_filters()?;

    // Store-wide recall floor: when the client omits `min_score`, apply the
    // daemon's configured `recall_min_score` so every host shares one recall
    // selectivity. An explicit per-call `min_score` overrides. (Same
    // server-owns-it pattern as `episodic_ttl_days`.)
    let min_score = match req.min_score {
        Some(s) => Some(s),
        None => Some(
            crate::http::config::load(&state.data_dir)
                .await
                .recall_min_score,
        ),
    };

    // Hybrid retrieval: each row scored as cosine + an IDF-weighted keyword
    // boost, so exact-keyword queries rank correctly without inventing
    // relevance for unrelated rows. `min_score` gates the hybrid score,
    // applied inside the fuse step (see crate::memory::hybrid).
    let table = match req.episodic {
        Some(true) => SearchTable::Episodic,
        Some(false) => SearchTable::Semantic,
        None => req.table,
    };
    let mut results = scored(
        &state, &table, &vector, &req.query, &filters, req.limit, min_score,
    )
    .await?;
    if is_no_root(&filters) {
        let extra = no_root_project_rows(&state, &table, &vector, &req.query, &filters).await?;
        merge_extra(&mut results, extra, req.limit);
    }
    Ok(ok(scored_facts_public(&results)))
}

/// At most this many rows filed under a directory join a no-root session's
/// recall.
pub const NO_ROOT_PROJECT_ROWS: usize = 2;

type Hit = (Memory, f32, f32);

async fn scored(
    state: &SharedState,
    table: &SearchTable,
    vector: &[f32],
    query: &str,
    filters: &Filters,
    limit: usize,
    min_score: Option<f32>,
) -> Result<Vec<Hit>, ApiError> {
    Ok(match table {
        SearchTable::Both => {
            state
                .recall
                .query(vector, query, filters, limit, min_score)
                .await?
        }
        SearchTable::Semantic => {
            state
                .store
                .hybrid_scored(vector, query, filters, limit, min_score)
                .await?
        }
        SearchTable::Episodic => {
            state
                .episodic
                .hybrid_scored(vector, query, filters, limit, min_score)
                .await?
        }
    })
}

/// Is this search a no-root session's (`$HOME`, `~/.linggen`, temp)?
fn is_no_root(filters: &Filters) -> bool {
    filters.scope_root.as_deref().is_some_and(|p| {
        crate::memory::RecallScope::of(p, &crate::memory::scope::home())
            == crate::memory::RecallScope::NoRoot
    })
}

/// The project half of a no-root session's recall: rows filed under some
/// directory, never `preference` (dev and project rules must not ride into
/// everyday chat), whose cosine reaches `no_root_project_min_score` — at
/// most [`NO_ROOT_PROJECT_ROWS`] of them.
async fn no_root_project_rows(
    state: &SharedState,
    table: &SearchTable,
    vector: &[f32],
    query: &str,
    filters: &Filters,
) -> Result<Vec<Hit>, ApiError> {
    let floor = crate::http::config::load(&state.data_dir)
        .await
        .no_root_project_min_score;
    let mut wide = filters.clone();
    wide.scope_root = None;
    wide.scoped_only = true;
    if !wide.exclude_types.contains(&MemoryType::Preference) {
        wide.exclude_types.push(MemoryType::Preference);
    }
    let mut hits = scored(state, table, vector, query, &wide, 20, None).await?;
    hits.retain(|(_, cosine, _)| *cosine >= floor);
    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    hits.truncate(NO_ROOT_PROJECT_ROWS);
    Ok(hits)
}

/// Fold the project rows into a no-root answer, best hybrid first, within
/// `limit` — the project rows keep their place even when person rows fill it.
fn merge_extra(results: &mut Vec<Hit>, extra: Vec<Hit>, limit: usize) {
    if extra.is_empty() {
        return;
    }
    let keep = limit.saturating_sub(extra.len());
    results.truncate(keep);
    results.extend(extra);
    results.sort_by(|a, b| b.2.total_cmp(&a.2));
}

async fn list(
    State(state): State<SharedState>,
    Json(mut req): Json<ListRequest>,
) -> Result<Response, ApiError> {
    // Resolve `past_ttl: true` into a concrete `until` cutoff. The mission
    // body sends `past_ttl: true` so it doesn't have to hardcode the TTL
    // number; the daemon owns the live value and applies it here.
    // An explicit `until`/`older_than` from the caller wins (caller knows
    // best); we only fill in when nothing is set.
    if req.filters.past_ttl && req.filters.until.is_none() {
        let cfg = crate::http::config::load(&state.data_dir).await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(cfg.episodic_ttl_days as i64);
        req.filters.until = Some(cutoff);
    }
    // `past_ttl` is an episodic-TTL concept and the tool schema promises
    // "implies tier=episodic" — scope the read accordingly unless the
    // caller explicitly asked for a table, so old semantic rows don't
    // masquerade as evictable staging.
    if req.filters.past_ttl && req.episodic.is_none() {
        req.episodic = Some(true);
    }
    let created_since = unjudged_since(&state, &req).await?;
    let mut filters = req.filters.into_filters()?;
    filters.created_since = created_since;
    let sort = req.sort.into();
    let mut combined = Vec::new();
    // For each in-scope store, pull `limit + offset` rows so the post-
    // merge sort+page still has enough material; bounded by the per-
    // store list cap.
    let take = req.limit.saturating_add(req.offset);
    for store in stores_for_read(&state, req.episodic) {
        let rows = store.list(&filters, sort, take, 0).await?;
        combined.extend(rows);
    }
    sort_combined(&mut combined, sort);
    let paged: Vec<_> = combined
        .into_iter()
        .skip(req.offset)
        .take(req.limit)
        .collect();
    Ok(ok(facts_public(&paged)))
}

/// `unjudged` resolved to its cutoff: the listed day's `remembered_at`.
/// The flag names a day's worklist, so it needs `day`.
async fn unjudged_since(
    state: &SharedState,
    req: &ListRequest,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ApiError> {
    if !req.unjudged {
        return Ok(None);
    }
    let Some(day) = req.filters.day.as_deref() else {
        return Err(ApiError::bad_request("unjudged needs a day"));
    };
    let days = super::days::load(&state.data_dir).await;
    Ok(days.days.get(day.trim()).and_then(|r| r.remembered_at))
}

fn sort_combined(rows: &mut [crate::memory::Memory], sort: crate::memory::SortOrder) {
    use crate::memory::SortOrder::*;
    // Match the store-layer order (and the UI's age badge): sort by
    // activity_timestamp (updated_at ?? created_at), NOT effective_timestamp.
    // effective_timestamp prefers the consolidator's back-dated occurred_at,
    // which buries freshly-written rows below their save date.
    rows.sort_by(|a, b| match sort {
        Newest => b.activity_timestamp().cmp(&a.activity_timestamp()),
        Oldest => a.activity_timestamp().cmp(&b.activity_timestamp()),
    });
}

async fn count(
    State(state): State<SharedState>,
    Json(req): Json<CountRequest>,
) -> Result<Response, ApiError> {
    let filters = req.filters.into_filters()?;
    let stores = stores_for_read(&state, req.episodic);

    // Sum row counts across in-scope stores (metadata-only LanceDB call).
    let mut total: usize = 0;
    for store in &stores {
        total = total.saturating_add(store.count_filtered(&filters).await?);
    }

    // "Latest" = max activity_timestamp (updated_at ?? created_at) across
    // in-scope stores — the same key the list order and UI badge use.
    // `list(Newest, 1)` returns each store's true newest row (list sorts the
    // full filtered set before slicing), so the top row's activity timestamp
    // is the answer. Skipped when count is 0 to avoid an empty fetch.
    let latest = if total == 0 {
        None
    } else {
        let mut newest: Option<chrono::DateTime<chrono::Utc>> = None;
        for store in &stores {
            let rows = store
                .list(&filters, crate::memory::SortOrder::Newest, 1, 0)
                .await?;
            if let Some(ts) = rows.first().map(|r| r.activity_timestamp()) {
                newest = Some(newest.map_or(ts, |cur| cur.max(ts)));
            }
        }
        newest
    };

    Ok(ok(json!({
        // Key kept for the dashboard's existing contract; value is now the
        // activity timestamp (updated_at ?? created_at), so an edited row
        // counts as "latest".
        "count": total,
        "latest_created_at": latest,
    })))
}

async fn update(
    State(state): State<SharedState>,
    Json(req): Json<UpdateRequest>,
) -> Result<Response, ApiError> {
    // A content rewrite of an existing row is the same violation class
    // as an add-with-replace_ids; metadata-only patches stay unguarded.
    if req.content.is_some() {
        guard_user_voice(&state, std::slice::from_ref(&req.id), req.user_directed).await?;
    }
    let outcome_patch = match (req.outcome, req.clear_outcome) {
        (Some(o), _) => Some(Some(o)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    // `~/…` expands; a relative path is no directory and is refused.
    let scope_dir = match req.scope_dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => Some(
            crate::memory::scope::expand(&d, &crate::memory::scope::home())
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "scope {d:?} is not an absolute directory (or ~/…)"
                    ))
                })?
                .to_string_lossy()
                .to_string(),
        ),
        None => None,
    };
    let scope_patch = scope_patch(scope_dir, req.clear_scope, req.global);
    let host_patch = match (req.host, req.clear_host) {
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let occurred_patch = match (req.occurred_at, req.clear_occurred_at) {
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    };

    let summary_patch = match (clean_summary(req.summary), req.clear_summary) {
        (Some(h), _) => Some(Some(h)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let mut patch = MemoryPatch {
        content: req.content,
        summary: summary_patch,
        indexed: req.indexed,
        r#type: req.r#type,
        tier: req.tier,
        origin: req.from,
        outcome: outcome_patch,
        scope: scope_patch,
        host: host_patch,
        occurred_at: occurred_patch,
        ..Default::default()
    };

    // Locate which table currently holds the row, so a tier patch that
    // crosses the semantic/episodic boundary can be honored by moving the
    // row across tables instead of leaving a tier=episodic row stranded
    // in the semantic table (or vice-versa).
    let mut located: Option<(Arc<MemoryStore>, Memory)> = None;
    for store in stores_for_read(&state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            located = Some((store, fact));
            break;
        }
    }
    let Some((current_store, existing)) = located else {
        return Err(ApiError::not_found(format!("no fact with id {}", req.id)));
    };
    if !req.scope.scope().admits(&existing) {
        return Err(ApiError::not_found(format!("no fact with id {}", req.id)));
    }

    let target_tier = patch.tier.unwrap_or(existing.tier);
    // Core is who the person is: no scope, no index — ever.
    if target_tier == Tier::Core {
        patch.scope = Some(None);
        patch.indexed = Some(false);
    }
    let target_is_episodic = matches!(target_tier, Tier::Episodic);
    let current_is_episodic = Arc::ptr_eq(&current_store, &state.episodic);

    if current_is_episodic == target_is_episodic {
        // Same-table update — delete+reinsert inside one store.
        let updated = current_store
            .update(&req.id, &patch)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("no fact with id {}", req.id)))?;
        return Ok(ok(fact_public(&updated)));
    }

    // Cross-table move: apply patch in memory, delete from source, insert
    // into target. Preserves the id and the existing embedding vector.
    let target_store = if target_is_episodic {
        Arc::clone(&state.episodic)
    } else {
        Arc::clone(&state.store)
    };
    let mut moved = existing;
    patch.apply(&mut moved);
    moved.tier = target_tier;
    moved.updated_at = Some(Utc::now().trunc_subsecs(6));
    current_store.delete(&req.id).await?;
    target_store.insert(std::slice::from_ref(&moved)).await?;
    Ok(ok(fact_public(&moved)))
}

async fn delete(
    State(state): State<SharedState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let scope = req.scope.scope();
    for store in stores_for_read(&state, req.episodic) {
        // Fetch first: a row outside the caller's account is simply not
        // theirs to remove, and it answers exactly as a missing one does.
        let Some(fact) = store.get(&req.id).await? else {
            continue;
        };
        if !scope.admits(&fact) {
            break;
        }
        if store.delete(&req.id).await? {
            return Ok(ok(json!({"id": req.id, "removed": true})));
        }
    }
    Ok(ok(json!({"id": req.id, "removed": false})))
}

/// `POST /api/memory/restamp` — re-own every row stamped `from` (both
/// tables, archived rows included, so provenance follows the person).
async fn restamp(
    State(state): State<SharedState>,
    Json(req): Json<RestampRequest>,
) -> Result<Response, ApiError> {
    let from = req.from.trim().to_string();
    if from.is_empty() {
        return Err(ApiError::bad_request("from must not be empty"));
    }
    let to = req
        .account_id
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let name = req.account_name.filter(|n| !n.trim().is_empty());
    let mut moved = 0usize;
    for store in stores_for_read(&state, None) {
        moved += store
            .restamp_account(&from, to.as_deref(), name.as_deref())
            .await?;
    }
    Ok(ok(json!({"from": from, "account_id": to, "moved": moved})))
}

/// `POST /api/memory/accounts` — every person the store holds rows for
/// besides the owner: `{account_id, account_name, rows}`. What a
/// per-account maintenance pass iterates.
async fn accounts(State(state): State<SharedState>) -> Result<Response, ApiError> {
    let mut seen: std::collections::BTreeMap<String, (Option<String>, usize)> =
        std::collections::BTreeMap::new();
    for store in stores_for_read(&state, None) {
        for (id, name, n) in store.distinct_accounts().await? {
            let e = seen.entry(id).or_insert((None, 0));
            if e.0.is_none() {
                e.0 = name;
            }
            e.1 += n;
        }
    }
    let list: Vec<Value> = seen
        .into_iter()
        .map(|(id, (name, rows))| json!({"account_id": id, "account_name": name, "rows": rows}))
        .collect();
    Ok(ok(json!(list)))
}

async fn forget(
    State(state): State<SharedState>,
    Json(req): Json<ForgetRequest>,
) -> Result<Response, ApiError> {
    let filters = req.filters.into_filters()?;
    // Refuse empty filters — bulk delete must be intentional. Matches the
    // CLI's refusal when no filter flags are passed.
    if filters.apps.is_empty()
        && filters.types.is_empty()
        && filters.origin.is_none()
        && filters.outcome.is_none()
        && filters.tier.is_none()
        && filters.since.is_none()
        && filters.until.is_none()
        && filters.source_session.is_none()
    {
        return Err(ApiError::bad_request(
            "forget refuses an empty filter — supply at least one of \
             apps, type, tier, from, outcome, since, until, source_session",
        ));
    }
    let store = pick_store(&state, req.episodic);
    let removed = store.forget(&filters).await?;
    Ok(ok(json!({"removed": removed})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_wins_over_a_stamped_scope() {
        assert_eq!(written_scope(Some("/u/w/p".into()), true), None);
        assert_eq!(
            written_scope(Some("/u/w/p".into()), false).as_deref(),
            Some("/u/w/p")
        );
        // An explicit empty cwd is no project, not a project named "".
        assert_eq!(written_scope(Some("".into()), false), None);
    }

    fn row(tier: Tier, cwd: Option<&str>) -> Memory {
        let mut m = Memory::new("x", MemoryType::Fact, Origin::Derived);
        m.tier = tier;
        m.scope = cwd.map(str::to_string);
        m
    }

    #[test]
    fn an_omitted_tier_is_episodic_unless_something_says_otherwise() {
        assert_eq!(resolve_tier(None, false, &[], false), Tier::Episodic);
        assert_eq!(
            resolve_tier(Some(Tier::Core), false, &[], false),
            Tier::Core
        );
        assert_eq!(
            resolve_tier(Some(Tier::Core), true, &[], false),
            Tier::Episodic
        );
        // A replacement keeps its losers' highest tier.
        let losers = [row(Tier::Semantic, None), row(Tier::Core, None)];
        assert_eq!(resolve_tier(None, false, &losers, false), Tier::Core);
        // An indexed row is long-term.
        assert_eq!(resolve_tier(None, false, &[], true), Tier::Semantic);
    }

    fn add_req(v: Value) -> AddRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn scope_resolution_on_add() {
        let home = crate::memory::scope::home();
        // A real dir under home: this crate's checkout is one.
        let here = std::env::current_dir().unwrap();
        if !here.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let root = here.to_string_lossy().to_string();
        let src = here.join("src").to_string_lossy().to_string();
        // The model's scope wins when it is a dir inside root.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": src}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(src.as_str())
        );
        // A scope outside root falls back to the session cwd.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": "/etc"}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(root.as_str())
        );
        // A parent of root is fine.
        let parent = here.parent().unwrap().to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": parent}));
        if here.parent().unwrap() != home {
            assert_eq!(
                resolve_scope(&r, &[], Tier::Semantic).as_deref(),
                Some(parent.as_str())
            );
        }
        // Home itself is never a scope.
        let h = home.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": h}));
        assert_eq!(resolve_scope(&r, &[], Tier::Episodic), None);
        // Core and global rows have none.
        let r = add_req(json!({"content": "x", "cwd": root}));
        assert_eq!(resolve_scope(&r, &[], Tier::Core), None);
        let r = add_req(json!({"content": "x", "cwd": root, "global": true}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        // A merge takes its losers' common scope, not the session's.
        let r = add_req(json!({"content": "x", "cwd": root}));
        let a = home.join("w/a/x").to_string_lossy().to_string();
        let b = home.join("w/a/y").to_string_lossy().to_string();
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, Some(&b))];
        assert_eq!(
            resolve_scope(&r, &losers, Tier::Semantic),
            Some(home.join("w/a").to_string_lossy().to_string())
        );
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, None)];
        assert_eq!(resolve_scope(&r, &losers, Tier::Semantic), None);
    }

    /// The 2026-10-05 session: started in a non-git workspace holding two
    /// repos, shell cd'd into one of them. With the session's root stamped,
    /// a candidate as the line shows it, an absolute dir under root, no scope
    /// and a dir outside root each land where the spec says.
    #[test]
    fn scope_from_a_nested_repo_resolves_against_the_session_root() {
        use std::path::{Path, PathBuf};
        let home = crate::memory::scope::home();
        let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("scope-nested-{}", std::process::id()))
            .join("linggen");
        if !ws.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let engine = ws.join("linggen");
        let cfo = ws.join("skills").join("cfo");
        std::fs::create_dir_all(engine.join(".git")).unwrap();
        std::fs::create_dir_all(ws.join("skills").join(".git")).unwrap();
        std::fs::create_dir_all(&cfo).unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();
        let at = |cwd: &Path, scope: Option<String>| {
            let mut v = json!({"content": "x", "cwd": s(cwd), "root": s(&ws)});
            if let Some(sc) = scope {
                v["scope"] = json!(sc);
            }
            resolve_scope(&add_req(v), &[], Tier::Episodic)
        };
        // Relative candidate (shown relative to root's parent), from skills/cfo.
        assert_eq!(at(&cfo, Some("linggen/linggen".into())), Some(s(&engine)));
        // Absolute dir under root, from the other repo.
        assert_eq!(at(&engine, Some(s(&cfo))), Some(s(&cfo)));
        // No scope: the session cwd.
        assert_eq!(at(&cfo, None), Some(s(&cfo)));
        // Outside root: falls back to the session cwd.
        assert_eq!(at(&engine, Some("/etc".into())), Some(s(&engine)));
        // The pre-fix stamp (root = the shell cwd's git root) dropped the
        // candidate: the hook, not the daemon, must send the session's root.
        let v = json!({"content": "x", "cwd": s(&cfo), "root": s(&ws.join("skills")), "scope": "linggen/linggen"});
        assert_eq!(
            resolve_scope(&add_req(v), &[], Tier::Episodic),
            Some(s(&cfo))
        );
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    /// An unstamped add (Codex) that names an existing directory keeps it;
    /// one naming nothing real stays unscoped.
    #[test]
    fn an_unstamped_add_keeps_a_named_directory() {
        let dir = std::env::current_dir().unwrap();
        let named = dir.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "scope": named}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), Some(named));
        let r = add_req(json!({"content": "x", "scope": "/no/such/dir/anywhere"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        let r = add_req(json!({"content": "x", "scope": "relative/dir"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
    }

    #[test]
    fn a_no_root_answer_keeps_its_project_rows() {
        let hit = |c: &str, h: f32| (Memory::new(c, MemoryType::Fact, Origin::User), h, h);
        let mut results = vec![hit("p1", 0.9), hit("p2", 0.8), hit("p3", 0.7)];
        merge_extra(&mut results, vec![hit("x1", 0.75)], 3);
        let names: Vec<&str> = results.iter().map(|r| r.0.content.as_str()).collect();
        assert_eq!(names, ["p1", "p2", "x1"]);
        let f = Filters {
            scope_root: Some(crate::memory::scope::home().to_string_lossy().to_string()),
            ..Default::default()
        };
        assert!(is_no_root(&f));
        let f = Filters {
            scope_root: Some("/w/repo".into()),
            ..Default::default()
        };
        assert!(!is_no_root(&f));
    }

    #[test]
    fn hooks_are_one_short_line() {
        assert_eq!(
            clean_summary(Some("  a\n b  ".into())).as_deref(),
            Some("a b")
        );
        assert_eq!(clean_summary(Some("   ".into())), None);
        let long = clean_summary(Some("x".repeat(200))).unwrap();
        assert_eq!(long.chars().count(), 80);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn an_old_phone_naming_contexts_any_gets_its_apps() {
        let req: ListRequest =
            serde_json::from_value(json!({"contexts_any": ["dj", "cfo"], "contexts": ["x"]}))
                .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.apps, ["dj", "cfo"]);
    }

    #[test]
    fn global_on_update_clears_the_scope() {
        assert_eq!(scope_patch(None, false, true), Some(None));
        assert_eq!(scope_patch(Some("/x".into()), false, true), Some(None));
        assert_eq!(
            scope_patch(Some("/x".into()), true, false),
            Some(Some("/x".into()))
        );
        assert_eq!(scope_patch(None, true, false), Some(None));
        assert_eq!(scope_patch(None, false, false), None);
    }

    #[test]
    fn list_takes_types_and_exclude_types() {
        let req: ListRequest = serde_json::from_value(json!({
            "types": ["preference", "decision"],
            "type": "fact",
            "exclude_types": ["built"],
            "cwd_scope": "/u/w/p"
        }))
        .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(
            f.types,
            [
                MemoryType::Preference,
                MemoryType::Decision,
                MemoryType::Fact
            ]
        );
        assert_eq!(f.exclude_types, [MemoryType::Built]);
        // `cwd_scope` is the pre-v2 name, still accepted.
        assert_eq!(f.scope_root.as_deref(), Some("/u/w/p"));
        let req: ListRequest = serde_json::from_value(json!({"scope_root": "/u/w/q"})).unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.scope_root.as_deref(), Some("/u/w/q"));
    }

    #[test]
    fn update_takes_scope_and_its_old_name() {
        let r: UpdateRequest =
            serde_json::from_value(json!({"id": "x", "scope": "/w/a", "clear_scope": true}))
                .unwrap();
        assert_eq!(r.scope_dir.as_deref(), Some("/w/a"));
        assert!(r.clear_scope);
        let r: UpdateRequest =
            serde_json::from_value(json!({"id": "x", "cwd": "/w/b", "clear_cwd": true})).unwrap();
        assert_eq!(r.scope_dir.as_deref(), Some("/w/b"));
        assert!(r.clear_scope);
    }

    #[test]
    fn search_takes_exclude_types() {
        let req: SearchRequest = serde_json::from_value(json!({
            "query": "q",
            "exclude_types": ["preference"]
        }))
        .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.exclude_types, [MemoryType::Preference]);
        let sql = f.to_sql_for_test();
        assert!(sql.contains("type NOT IN ('preference')"), "{sql}");
    }

    /// A state over fresh stores, with an embedder that is never loaded.
    async fn scratch_state(dir: &std::path::Path) -> SharedState {
        let semantic = Arc::new(MemoryStore::open_semantic(dir).await.unwrap());
        let episodic = Arc::new(MemoryStore::open_episodic(dir).await.unwrap());
        let recall = Arc::new(crate::memory::Recall::new(
            Arc::clone(&semantic),
            Arc::clone(&episodic),
        ));
        fn no_model() -> anyhow::Result<crate::embed::Embedder> {
            anyhow::bail!("no model in unit tests")
        }
        Arc::new(super::super::state::AppState::new(
            semantic,
            episodic,
            recall,
            Arc::new(crate::embed::LoadOnce::new(no_model)),
            dir.to_path_buf(),
            0,
        ))
    }

    /// The 2026-10-05 loss: `memory_add` merged into the row it was told to
    /// replace, then retired that same row — nothing live was left. The row
    /// a write lands in is never one of its losers; the others still retire.
    #[tokio::test]
    async fn a_replacement_never_retires_the_row_it_wrote_into() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let kept = Memory::new("kept", MemoryType::Fact, Origin::Derived);
        let other = Memory::new("other", MemoryType::Fact, Origin::Derived);
        state
            .store
            .insert(&[kept.clone(), other.clone()])
            .await
            .unwrap();

        let ids = vec![kept.id.clone(), other.id.clone()];
        let body = apply_replace_ids(&state, &ids, &kept.id, json!({})).await;
        assert_eq!(body["replaced"], json!([other.id]));
        assert!(body.get("replaced_failed").is_none(), "{body}");

        let live = state.store.get(&kept.id).await.unwrap().unwrap();
        assert!(live.expired_at.is_none(), "the survivor stays live");
        let gone = state.store.get(&other.id).await.unwrap().unwrap();
        assert!(gone.expired_at.is_some());
        assert_eq!(gone.superseded_by.as_deref(), Some(kept.id.as_str()));
    }

    /// Reading rows never waits on the embedding model: with a model that
    /// cannot load, `session_start` still answers with core.
    #[tokio::test]
    async fn session_start_needs_no_embedder() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let mut core = Memory::new("Alex lives in Lisbon", MemoryType::Fact, Origin::User);
        core.tier = Tier::Core;
        state.store.insert(&[core]).await.unwrap();

        let app = super::super::session::router().with_state(Arc::clone(&state));
        let res = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::post("/api/memory/session_start")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["data"]["core"].as_array().map(Vec::len), Some(1), "{v}");
        assert!(!state.embedder.is_ready(), "nothing asked for the model");
        // A call that embeds reports the load failure instead of hanging.
        assert!(state.embedder().await.is_err());
    }
}
