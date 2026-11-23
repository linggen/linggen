//! How a model comes into the shared Hugging Face cache — the one place that
//! decides it, for every model the engine fetches (the voice, the hearing and
//! the picture model; `runtime::HUB_MODELS`).
//!
//! The fetch itself is the `huggingface_hub` library's, run in the model's
//! venv. This module is what stands around it:
//!
//! - **Patience.** A failure that is transient by nature (the Hub answered
//!   oddly, the connection dropped, 429/5xx) is tried again after a growing
//!   pause ([`Pace`]). One that is not (no disk, 401/403/404, the user's own
//!   `HF_ENDPOINT` saying no) stops at once ([`Cause::transient`]).
//! - **What is kept.** Nothing is ever deleted from the cache here. Files the
//!   library finished stay in `blobs/` and `snapshots/` and are not asked for
//!   again. (That the library skips finished files and resumes or restarts
//!   only the one it was on is its documented behaviour, not read in its
//!   source here; the engine's own GET, which does resume, is the cover if
//!   it does not.) Either way the next call starts from the finished files.
//! - **The engine's own GET, [`complete_from_hub`].** On 2026-11-22 the
//!   library fetched a hearing model's 1.5 GB weight file and then failed,
//!   twice in half a minute, on its thirteen small files
//!   (`FileMetadataError: Distant resource does not seem to be on
//!   huggingface.co` — the Hub had answered those files' metadata request with
//!   an answer the library's check refused), while the same library called a
//!   little later by hand fetched them at once. The Hub was answering the
//!   engine's own GET all along. So after [`FALLBACK_AFTER`] failures of the
//!   library, the engine lists the model's files itself and GETs the missing
//!   ones into the layout the library's offline lookup reads. This is a
//!   workaround for that library's metadata step: it can go once a first fetch
//!   on a real Mac no longer needs it (the log line `[hf] fetched … by
//!   the engine's own GET` tells when it did).
//! - **What the person is told.** [`Failure::tell`] and [`Note`]: whether
//!   trying again can help and what was the matter, in a few plain words;
//!   a fetch waiting to try again says so. Kept per model ([`note_for`]) for
//!   the page, the picture tool and the voice status to read.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The revision every fetch of a model asks for, the library's own default.
/// Named here so the library's call ([`library_code`]) and the engine's own
/// listing ([`complete_from_hub`]) cannot ask for different ones.
pub const REVISION: &str = "main";

/// After this many failures of the library on one fetch, the engine's own GET
/// completes the model before the next try. Three: the library gets its two
/// pauses first, so a hiccup that passes is never met with a second way in.
const FALLBACK_AFTER: usize = 3;

// ---------------------------------------------------------------------------
// Why it failed
// ---------------------------------------------------------------------------

/// What was the matter, as far as the library's words and the engine's own
/// GET tell. The closed set that decides both "try again?" and the words
/// shown to the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The Hub answered, but not as the library needs (`FileMetadataError`,
    /// `LocalEntryNotFoundError`).
    Metadata,
    /// Nothing reached the Hub, or the line dropped mid-way.
    Connection,
    /// 429, or a 5xx.
    Busy,
    /// Anything else the library stopped on — unknown, so tried again.
    Interrupted,
    /// No room on the disk.
    Disk,
    /// 401 / 403: the Hub will not give it to this Mac.
    Refused,
    /// 404: no such model or file at this revision.
    Missing,
    /// The user's own `HF_ENDPOINT` refused: its owner's word, not weather.
    UsersHub,
}

impl Cause {
    /// Can the same thing, tried again later, come out differently?
    pub fn transient(self) -> bool {
        matches!(self, Cause::Metadata | Cause::Connection | Cause::Busy | Cause::Interrupted)
    }

    /// Plain words, at most five, lower case: they sit mid-sentence.
    pub fn words(self) -> &'static str {
        match self {
            Cause::Metadata => "the Hub answered oddly",
            Cause::Connection => "the connection dropped",
            Cause::Busy => "the Hub is busy",
            Cause::Interrupted => "the download broke off",
            Cause::Disk => "the disk is full",
            Cause::Refused => "the Hub refused access",
            Cause::Missing => "the Hub has no such model",
            Cause::UsersHub => "your Hub address said no",
        }
    }

    /// Every cause, for the tests that hold a rule over all of them.
    #[cfg(test)]
    const ALL: [Cause; 8] = [
        Cause::Metadata,
        Cause::Connection,
        Cause::Busy,
        Cause::Interrupted,
        Cause::Disk,
        Cause::Refused,
        Cause::Missing,
        Cause::UsersHub,
    ];
}

/// A fetch that ended without the model. `detail` is the library's last words
/// for the log; the person is told [`Failure::tell`] only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub cause: Cause,
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.cause.words(), self.detail)
    }
}

impl std::error::Error for Failure {}

impl Failure {
    /// Read the cause off what the library (or the engine's own GET, or the
    /// disk check) said. `users_hub`: the user named the endpoint.
    pub fn classify(text: &str, users_hub: bool) -> Failure {
        Failure { cause: cause_of(text, users_hub), detail: tail(text) }
    }

    /// The line a person reads after the fetch gave up: `what` is what was
    /// being fetched ("the hearing model (1.6 GB)"), `again` what to do to
    /// try again ("Hold again to try again."), said only where trying again
    /// can help. Whole sentence, at most 20 words for the `what`s in use.
    pub fn tell(&self, what: &str, again: &str) -> String {
        let advice = match self.cause {
            c if c.transient() => again,
            Cause::Disk => "Free some space, then try again.",
            _ => "Trying again won't help.",
        };
        format!("Couldn't fetch {what}: {}. {advice}", self.cause.words())
    }
}

/// The last of a long message (a library's stderr is a screen of progress
/// bars before its words).
fn tail(text: &str) -> String {
    let text = text.trim();
    let from = text.char_indices().rev().nth(399).map_or(0, |(i, _)| i);
    text[from..].to_string()
}

fn cause_of(text: &str, users_hub: bool) -> Cause {
    static REFUSAL: OnceLock<regex::Regex> = OnceLock::new();
    // `HTTP Error 403`, `Client error '404 Not Found'`, `403 Client Error`,
    // `status code 401` — a status the library or the OS printed.
    let refusal = REFUSAL.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:client error|http error|status code|status)\D{0,6}(40[134])\b|\b(40[134])\b\W{0,3}(?:client error|forbidden|unauthorized|not found)",
        )
        .expect("refusal pattern")
    });
    static SERVER: OnceLock<regex::Regex> = OnceLock::new();
    let server = SERVER.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(?:429|50[0-4])\b[^\n]{0,40}(?:error|too many|unavailable|gateway|timeout|server)|(?:error|status|code|http)\D{0,8}\b(?:429|50[0-4])\b",
        )
        .expect("server pattern")
    });
    let lower = text.to_ascii_lowercase();
    if ["no space left", "low disk", "not enough free disk", "disk quota"].iter().any(|w| lower.contains(w)) {
        return Cause::Disk;
    }
    let status = refusal
        .captures(text)
        .and_then(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| m.as_str().to_string());
    let named_missing = ["RepositoryNotFoundError", "RevisionNotFoundError", "RemoteEntryNotFoundError"]
        .iter()
        .any(|w| text.contains(w));
    let named_refused = text.contains("GatedRepoError");
    if status.is_some() || named_missing || named_refused {
        if users_hub {
            return Cause::UsersHub;
        }
        return match status.as_deref() {
            Some("404") => Cause::Missing,
            _ if named_missing => Cause::Missing,
            _ => Cause::Refused,
        };
    }
    if server.is_match(text) {
        return Cause::Busy;
    }
    if text.contains("FileMetadataError") || text.contains("LocalEntryNotFoundError") || lower.contains("distant resource") {
        return Cause::Metadata;
    }
    const DROPPED: [&str; 12] = [
        "connecterror", "timeout", "timed out", "connection reset", "connection refused", "connection aborted",
        "name or service not known", "nodename nor servname", "temporary failure in name resolution",
        "remoteprotocolerror", "readerror", "ssl",
    ];
    if DROPPED.iter().any(|w| lower.contains(w)) {
        return Cause::Connection;
    }
    Cause::Interrupted
}

// ---------------------------------------------------------------------------
// What is told while it runs and after
// ---------------------------------------------------------------------------

/// What a model's fetch last said about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Failed this time and waiting to try again: the `try_no`th of `of`.
    Retrying { cause: Cause, try_no: usize, of: usize },
    /// Gave up.
    Failed(Failure),
}

impl Note {
    /// One sentence for the person: the retry as it is, the failure as
    /// [`Failure::tell`]. `what` and `again` as there; at most 20 words for
    /// the `what`s and `again`s in use (`hf_fetch_tests`).
    pub fn told(&self, what: &str, again: &str) -> String {
        match self {
            Note::Retrying { cause, try_no, of } => {
                format!("Still fetching {what}: {}. Trying again ({try_no} of {of}).", cause.words())
            }
            Note::Failed(failure) => failure.tell(what, again),
        }
    }
}

type Notes = HashMap<String, (u64, Note)>;

fn notes() -> &'static Mutex<Notes> {
    static NOTES: OnceLock<Mutex<Notes>> = OnceLock::new();
    NOTES.get_or_init(Default::default)
}

fn set_note(repo: &str, note: Option<Note>) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut map = notes().lock().unwrap_or_else(|p| p.into_inner());
    match note {
        Some(note) => {
            let at = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            map.insert(repo.to_string(), (at, note));
        }
        None => {
            map.remove(repo);
        }
    }
}

/// Forget what `repos` last said: a new try begins, and what an earlier one
/// said is no longer about it.
pub fn forget(repos: &[&str]) {
    for repo in repos {
        set_note(repo, None);
    }
}

/// The newest thing any of `repos` has said — a hearing fetch that builds the
/// voice model under it first asks for both.
pub fn note_for(repos: &[&str]) -> Option<Note> {
    let map = notes().lock().unwrap_or_else(|p| p.into_inner());
    repos
        .iter()
        .filter_map(|r| map.get(*r))
        .max_by_key(|(at, _)| *at)
        .map(|(_, note)| note.clone())
}

/// Record that `repo`'s fetch ended with `failure` (also for a stop before the
/// fetch ran, such as no disk) and hand the failure back.
pub fn failed(repo: &str, failure: Failure) -> Failure {
    set_note(repo, Some(Note::Failed(failure.clone())));
    failure
}

// ---------------------------------------------------------------------------
// The fetch
// ---------------------------------------------------------------------------

/// The pauses between tries; one more try than pauses.
pub struct Pace(pub Vec<Duration>);

impl Pace {
    /// Five tries in about three minutes: the Hub's bad moments seen so far
    /// last seconds to a minute.
    pub fn patient() -> Pace {
        Pace([5, 15, 45, 120].map(Duration::from_secs).to_vec())
    }
}

/// Where the engine itself may GET from now (`hf_mirror::bases`).
pub struct Hub {
    pub bases: Vec<String>,
    /// The user named the endpoint: no mirror behind it.
    pub users: bool,
}

/// The two steps a fetch is made of, so a test can stand in for both.
#[async_trait::async_trait]
pub trait Steps: Send + Sync {
    /// The library's own fetch of the model (`offline`: its look in the cache
    /// alone, which must pass once the cache is complete). `Err` is what it
    /// said.
    async fn library(&self, offline: bool) -> Result<(), String>;
    /// Where the engine may GET from, asked before each try.
    async fn hub(&self) -> Hub;
}

/// Bring `repo` into the HF cache `cache` (the `hub` folder). The only
/// function that fetches a model; `Ok` means the library's own offline lookup
/// finds it whole.
pub async fn fetch(repo: &str, cache: &Path, steps: &dyn Steps, pace: &Pace) -> Result<(), Failure> {
    set_note(repo, None);
    let tries = pace.0.len() + 1;
    let mut library_failures = 0;
    let mut failure = None;
    for try_no in 1..=tries {
        let hub = steps.hub().await;
        match steps.library(false).await {
            Ok(()) => return Ok(done(repo)),
            Err(said) => {
                library_failures += 1;
                failure = Some(Failure::classify(&said, hub.users));
                tracing::warn!("[hf] {repo}: try {try_no} of {tries} failed: {}", failure.as_ref().map_or("", |f| &f.detail));
            }
        }
        let stopped = failure.as_ref().is_some_and(|f| !f.cause.transient());
        if stopped {
            break;
        }
        if library_failures >= FALLBACK_AFTER {
            match own_get_round(repo, cache, steps, &hub).await {
                Ok(()) => return Ok(done(repo)),
                // A refusal of the engine's own GET is final and is what is
                // told; trouble that may pass leaves the library's reading of
                // it standing (the library's words are the better ones).
                Err(f) if !f.cause.transient() => {
                    failure = Some(f);
                    break;
                }
                Err(f) => tracing::warn!("[hf] {repo}: the engine's own GET failed too: {f}"),
            }
        }
        if try_no == tries {
            break;
        }
        let cause = failure.as_ref().map_or(Cause::Interrupted, |f| f.cause);
        set_note(repo, Some(Note::Retrying { cause, try_no: try_no + 1, of: tries }));
        tokio::time::sleep(pace.0[try_no - 1]).await;
    }
    let failure = failure.unwrap_or(Failure { cause: Cause::Interrupted, detail: String::new() });
    tracing::warn!("[hf] {repo}: gave up: {failure}");
    Err(failed(repo, failure))
}

fn done(repo: &str) {
    set_note(repo, None);
}

/// The engine's own GET, then the library's offline lookup to confirm.
async fn own_get_round(repo: &str, cache: &Path, steps: &dyn Steps, hub: &Hub) -> Result<(), Failure> {
    complete_from_hub(repo, cache, hub).await.map_err(|e| Failure::classify(&hub_error_text(&e), hub.users))?;
    steps.library(true).await.map_err(|said| Failure::classify(&said, hub.users))?;
    tracing::info!("[hf] fetched {repo} by the engine's own GET");
    Ok(())
}

/// An error from the engine's own GET as words [`Failure::classify`] reads:
/// a refusal carries its status.
fn hub_error_text(e: &anyhow::Error) -> String {
    let mut text = format!("{e:#}");
    if let Some(status) = e
        .downcast_ref::<crate::mirror::HttpStatus>()
        .map(|h| h.status)
        .or_else(|| e.downcast_ref::<reqwest::Error>().and_then(|r| r.status()))
    {
        text = format!("HTTP Error {} {text}", status.as_u16());
    }
    text
}

// ---------------------------------------------------------------------------
// The engine's own GET
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Listing {
    sha: String,
    siblings: Vec<Sibling>,
}

#[derive(serde::Deserialize)]
struct Sibling {
    rfilename: String,
    size: Option<u64>,
    #[serde(rename = "blobId")]
    blob_id: Option<String>,
    lfs: Option<LfsInfo>,
}

#[derive(serde::Deserialize)]
struct LfsInfo {
    sha256: String,
}

/// The cache folder of one repo, in the library's layout.
fn repo_folder(cache: &Path, repo: &str) -> PathBuf {
    cache.join(format!("models--{}", repo.replace('/', "--")))
}

/// A path the Hub names must stay inside the snapshot folder.
fn safe_relative(name: &str) -> Result<()> {
    let bad = name.is_empty() || name.starts_with('/') || name.contains('\\') || name.split('/').any(|s| s.is_empty() || s == "." || s == "..");
    anyhow::ensure!(!bad, "the Hub's listing names an unsafe path: {name:?}");
    Ok(())
}

/// Put every file of `repo` at [`REVISION`] that the cache lacks into it, in
/// the layout the library reads offline (`blobs/<id>`, `snapshots/<commit>/<path>`
/// a link to it, `refs/<revision>` holding the commit). The list is the Hub's
/// own (`/api/models/<repo>/revision/<revision>`), never a list kept here. A
/// file already in the snapshot is not asked for; a half-written one
/// (`blobs/<id>.incomplete`) is resumed. LFS files are checked against the
/// Hub's sha256, others against its size, before they are put in place.
pub async fn complete_from_hub(repo: &str, cache: &Path, hub: &Hub) -> Result<()> {
    let body = crate::hf_mirror::get_bytes_at(&hub.bases, &format!("api/models/{repo}/revision/{REVISION}?blobs=true")).await?;
    let listing: Listing = serde_json::from_slice(&body).context("read the Hub's file list")?;
    anyhow::ensure!(
        !listing.sha.is_empty() && listing.sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "the Hub's file list has no commit"
    );
    anyhow::ensure!(!listing.siblings.is_empty(), "the Hub's file list is empty");
    let folder = repo_folder(cache, repo);
    let snapshot = folder.join("snapshots").join(&listing.sha);
    for file in &listing.siblings {
        safe_relative(&file.rfilename)?;
        let pointer = snapshot.join(&file.rfilename);
        if pointer.is_file() {
            continue;
        }
        let id = file.lfs.as_ref().map(|l| l.sha256.clone()).or_else(|| file.blob_id.clone());
        let id = id.filter(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_hexdigit()));
        let id = id.with_context(|| format!("the Hub's file list has no id for {}", file.rfilename))?;
        let blob = folder.join("blobs").join(&id);
        if !blob.is_file() {
            fetch_blob(hub, repo, &listing.sha, file, &blob).await?;
        }
        link(&blob, &pointer, &file.rfilename)?;
    }
    let refs = folder.join("refs");
    std::fs::create_dir_all(&refs)?;
    std::fs::write(refs.join(REVISION), &listing.sha)?;
    Ok(())
}

async fn fetch_blob(hub: &Hub, repo: &str, commit: &str, file: &Sibling, blob: &Path) -> Result<()> {
    let part = blob.with_extension("incomplete");
    std::fs::create_dir_all(blob.parent().context("blob parent")?)?;
    let have = part.metadata().map(|m| m.len()).unwrap_or(0);
    // A part that is already whole needs no request (a Range past the end is a 416).
    let whole = file.size == Some(have) && have > 0;
    if !whole {
        if file.size.is_some_and(|s| have > s) {
            std::fs::remove_file(&part)?;
        }
        let path = format!("{repo}/resolve/{commit}/{}", file.rfilename);
        crate::hf_mirror::download_at(&hub.bases, &path, &part).await?;
    }
    if let Err(e) = verify(&part, file) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, blob)?;
    Ok(())
}

fn verify(part: &Path, file: &Sibling) -> Result<()> {
    let len = part.metadata()?.len();
    if let Some(size) = file.size {
        anyhow::ensure!(len == size, "{} came to {len} bytes, the Hub says {size}", file.rfilename);
    }
    if let Some(lfs) = &file.lfs {
        let got = crate::runtime::file_sha256(part)?;
        anyhow::ensure!(got == lfs.sha256, "{} does not match the Hub's checksum", file.rfilename);
    }
    Ok(())
}

/// `pointer` -> `blob`, relative as the library writes it
/// (`snapshots/<commit>/<dirs…>/<file>` → `blobs/<id>`). A file system with no
/// links gets a copy.
fn link(blob: &Path, pointer: &Path, rfilename: &str) -> Result<()> {
    std::fs::create_dir_all(pointer.parent().context("pointer parent")?)?;
    let mut relative = PathBuf::from("../..");
    for _ in 0..rfilename.matches('/').count() {
        relative.push("..");
    }
    relative.push("blobs");
    relative.push(blob.file_name().context("blob name")?);
    #[cfg(unix)]
    if std::os::unix::fs::symlink(&relative, pointer).is_ok() {
        return Ok(());
    }
    std::fs::copy(blob, pointer).map(|_| ()).context("link a file into the snapshot")
}

// ---------------------------------------------------------------------------
// The library's call
// ---------------------------------------------------------------------------

/// How the library is asked for a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryStep {
    /// `snapshot_download`: the files, nothing loaded.
    Snapshot,
    /// mlx-audio's `load_model`: it downloads (every file its patterns name)
    /// and loads, which also proves the voice model loads.
    LoadAudioModel,
}

/// The Python run in a model's venv for `step`. `offline`: the library may
/// look in the cache only — the check that the engine's own GET left the
/// model whole. The caller also sets `HF_HUB_OFFLINE=1` for it (the load of
/// the voice model has no argument for it). For the hearing and picture models
/// it is `snapshot_download` with `local_files_only`, the call the hearing
/// sidecar loads the model by (`mlx_voice.py`).
pub fn library_code(step: LibraryStep, repo: &str, offline: bool) -> String {
    match (step, offline) {
        (LibraryStep::Snapshot, true) => format!(
            "from huggingface_hub import snapshot_download; snapshot_download({repo:?}, revision={REVISION:?}, local_files_only=True)"
        ),
        (LibraryStep::Snapshot, false) => format!(
            "from huggingface_hub import snapshot_download; snapshot_download({repo:?}, revision={REVISION:?})"
        ),
        // `load_model` has no revision argument: it asks for the default, [`REVISION`].
        (LibraryStep::LoadAudioModel, _) => {
            format!("from mlx_audio.tts.utils import load_model; load_model({repo:?})")
        }
    }
}

#[cfg(test)]
#[path = "hf_fetch_tests.rs"]
mod tests;
