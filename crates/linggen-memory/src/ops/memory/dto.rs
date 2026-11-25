//! The row verbs' request bodies (`add`, `get`, `search`, `list`, …), the
//! lenient deserializers they share (LLM-filled enums and dates), and the
//! public shape of a row on output (no embedding vector).

use crate::memory::Memory as Row;
use crate::memory::{AccountScope, Hit, MemoryType, Origin, Outcome, SortOrder, Tier};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

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
pub(crate) fn fact_public(fact: &Row) -> Value {
    let mut v = serde_json::to_value(fact).unwrap_or_else(|_| Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("vector");
    }
    v
}

pub(crate) fn facts_public(facts: &[Row]) -> Vec<Value> {
    facts.iter().map(fact_public).collect()
}

/// Like [`fact_public`] but adds the relevance fields for search responses:
///
/// - `score` — the raw **cosine** similarity (`[0,1]`), the absolute
///   dense-relevance signal. Kept for the recall hook, CLI, and cross-host
///   comparisons that already read it.
/// - `hybrid_score` — the shown relevance: (cosine + IDF-weighted keyword
///   boost) × the row's scope weight, clamped to `[0,1]` (see
///   [`crate::memory::hybrid`]). The number the console displays: what the
///   rows are ordered by, so it is monotonic with rank, and absolute — an
///   unrelated query does not get a fake 1.0.
/// - `raw_hybrid` — cosine + keyword boost, before the weight and unclamped.
/// - `scope_weight` — the row's scope weight in this session (1.0 with no
///   root), for inspection.
pub(crate) fn scored_facts_public(hits: &[Hit]) -> Vec<Value> {
    hits.iter()
        .map(|h| {
            let mut v = fact_public(&h.memory);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("score".into(), json!(h.cosine));
                obj.insert("hybrid_score".into(), json!(h.shown()));
                obj.insert("raw_hybrid".into(), json!(h.hybrid));
                obj.insert("scope_weight".into(), json!(h.weight));
            }
            v
        })
        .collect()
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
    /// `replaced_failed` but never abort the insert. A write that carries
    /// `account_id` may name only that person's rows; any other id refuses
    /// the call before anything is written.
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
    pub(crate) fn scope(self) -> AccountScope {
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
    /// `Filters::scope_root`). On search a skill's dir sees only its own rows;
    /// any other root weighs every non-app row by its directory's distance
    /// from the root. On list it filters: an owner root sees its subtree, its
    /// parents and person rows; `$HOME`/`~/.linggen`/temp person rows only.
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
    /// A programmatic caller that must never hold a turn (the automatic
    /// recall) sets this: while memory warms, the search is refused at once
    /// instead of waiting for the warm-up. Not in the advertised MCP schema,
    /// like `min_score`.
    #[serde(default)]
    pub skip_if_warming: bool,
    /// The automatic recall of a host that can detect use (Claude Code's
    /// hooks) sets this: rows heat has silenced are left out. Not in the
    /// advertised schema; an agent's own search sees every row.
    #[serde(default)]
    pub push: bool,
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
    /// Heat bookkeeping (`memory::heat`), from the hooks: add to the row's
    /// `recalled` / `used` counters. Alone it changes nothing else — not
    /// even `updated_at`. Not in the advertised schema.
    #[serde(default)]
    pub bump_recalled: u32,
    #[serde(default)]
    pub bump_used: u32,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
