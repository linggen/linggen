//! Tests of `hf_fetch`: a fake Hub on 127.0.0.1 and a scripted library step.
//! No request leaves this machine and no model is fetched.

use super::*;
use sha2::Digest;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// The fake Hub
// ---------------------------------------------------------------------------

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

enum Route {
    Body(Vec<u8>),
    Status(u16),
}

struct FakeHub {
    base: String,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    /// Every request: path (no query) and its Range header.
    hits: Arc<Mutex<Vec<(String, Option<String>)>>>,
}

impl FakeHub {
    fn hub(&self) -> Hub {
        Hub { bases: vec![self.base.clone()], users: false }
    }

    fn asked(&self, path: &str) -> usize {
        self.hits.lock().unwrap().iter().filter(|(p, _)| p == path).count()
    }

    fn ranges(&self, path: &str) -> Vec<String> {
        self.hits.lock().unwrap().iter().filter(|(p, _)| p == path).filter_map(|(_, r)| r.clone()).collect()
    }

    fn set(&self, path: &str, route: Route) {
        self.routes.lock().unwrap().insert(path.to_string(), route);
    }
}

fn serve_hub(routes: Vec<(String, Route)>) -> FakeHub {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::new(Mutex::new(routes.into_iter().collect()));
    let hits: Arc<Mutex<Vec<(String, Option<String>)>>> = Default::default();
    let (served, seen) = (routes.clone(), hits.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (served, seen) = (served.clone(), seen.clone());
            std::thread::spawn(move || answer(stream, &served, &seen));
        }
    });
    FakeHub { base, routes, hits }
}

fn answer(mut s: std::net::TcpStream, routes: &Mutex<HashMap<String, Route>>, hits: &Mutex<Vec<(String, Option<String>)>>) {
    let mut buf = vec![0u8; 8192];
    let n = s.read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    let target = request.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").trim_start_matches('/').to_string();
    let range = request
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("range:").map(|v| v.trim().to_string()));
    hits.lock().unwrap().push((path.clone(), range.clone()));
    let response = match routes.lock().unwrap().get(&path) {
        None => head(404, 0, ""),
        Some(Route::Status(code)) => head(*code, 0, ""),
        Some(Route::Body(body)) => {
            let from = range
                .as_deref()
                .and_then(|r| r.strip_prefix("bytes="))
                .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
            match from {
                Some(from) if from >= body.len() => head(416, 0, ""),
                Some(from) => {
                    let mut out = head(206, body.len() - from, &format!("content-range: bytes {from}-{}/{}\r\n", body.len() - 1, body.len())).into_bytes();
                    out.extend_from_slice(&body[from..]);
                    let _ = s.write_all(&out);
                    return;
                }
                None => {
                    let mut out = head(200, body.len(), "").into_bytes();
                    out.extend_from_slice(body);
                    let _ = s.write_all(&out);
                    return;
                }
            }
        }
    };
    let _ = s.write_all(response.as_bytes());
}

fn head(status: u16, len: usize, extra: &str) -> String {
    format!("HTTP/1.1 {status} X\r\ncontent-length: {len}\r\n{extra}connection: close\r\n\r\n")
}

fn sha256_of(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// A model of four files: a big weight file (LFS, with a checksum) and three
/// small ones, one of them in a folder.
struct Model {
    repo: String,
    files: Vec<(&'static str, Vec<u8>)>,
}

impl Model {
    fn new(repo: &str) -> Model {
        Model {
            repo: repo.to_string(),
            files: vec![
                ("weights.safetensors", (0..4000u32).map(|i| (i % 251) as u8).collect()),
                ("config.json", br#"{"model_type":"fake"}"#.to_vec()),
                ("tokenizer/vocab.json", b"{\"a\":1}".to_vec()),
                ("README.md", b"# fake\n".to_vec()),
            ],
        }
    }

    fn blob_id(&self, name: &str, body: &[u8]) -> String {
        if name.ends_with(".safetensors") {
            sha256_of(body)
        } else {
            sha256_of(name.as_bytes())[..40].to_string()
        }
    }

    fn listing(&self) -> Vec<u8> {
        let siblings: Vec<serde_json::Value> = self
            .files
            .iter()
            .map(|(name, body)| {
                let mut s = serde_json::json!({"rfilename": name, "size": body.len(), "blobId": self.blob_id(name, body)});
                if name.ends_with(".safetensors") {
                    s["lfs"] = serde_json::json!({"sha256": sha256_of(body), "size": body.len(), "pointerSize": 134});
                }
                s
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({"sha": COMMIT, "siblings": siblings})).unwrap()
    }

    fn hub(&self) -> FakeHub {
        let mut routes = vec![(format!("api/models/{}/revision/main", self.repo), Route::Body(self.listing()))];
        for (name, body) in &self.files {
            routes.push((format!("{}/resolve/{COMMIT}/{name}", self.repo), Route::Body(body.clone())));
        }
        serve_hub(routes)
    }

    fn resolve_path(&self, name: &str) -> String {
        format!("{}/resolve/{COMMIT}/{name}", self.repo)
    }

    /// What the library leaves after it finished `name` alone: its blob, the
    /// snapshot's link to it and the ref.
    fn seed_finished(&self, cache: &Path, name: &str) {
        let body = &self.files.iter().find(|(n, _)| *n == name).unwrap().1;
        let folder = repo_folder(cache, &self.repo);
        let blob = folder.join("blobs").join(self.blob_id(name, body));
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, body).unwrap();
        link(&blob, &folder.join("snapshots").join(COMMIT).join(name), name).unwrap();
        std::fs::create_dir_all(folder.join("refs")).unwrap();
        std::fs::write(folder.join("refs/main"), COMMIT).unwrap();
    }

    /// The library's own offline lookup, as far as it matters here: the ref
    /// names a commit, and every file of the model is readable through the
    /// snapshot folder with the bytes the Hub holds.
    fn offline_lookup(&self, cache: &Path) -> Result<(), String> {
        let folder = repo_folder(cache, &self.repo);
        let commit = std::fs::read_to_string(folder.join("refs/main")).map_err(|e| format!("LocalEntryNotFoundError: no ref ({e})"))?;
        for (name, body) in &self.files {
            let path = folder.join("snapshots").join(commit.trim()).join(name);
            let got = std::fs::read(&path).map_err(|e| format!("LocalEntryNotFoundError: {name} ({e})"))?;
            if &got != body {
                return Err(format!("{name} differs"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The scripted library
// ---------------------------------------------------------------------------

/// A library that says what the script says, one answer per online call, and
/// then `then` for every call after; its offline call is [`Model::offline_lookup`].
struct Scripted {
    script: Mutex<VecDeque<Result<(), String>>>,
    then: Result<(), String>,
    online: AtomicUsize,
    offline: AtomicUsize,
    hub: Hub,
    model: Model,
    cache: PathBuf,
    /// What `note_for` said at each online call.
    notes_seen: Mutex<Vec<Option<Note>>>,
}

impl Scripted {
    fn new(model: Model, cache: &Path, hub: Hub, script: Vec<Result<(), String>>, then: Result<(), String>) -> Scripted {
        Scripted {
            script: Mutex::new(script.into()),
            then,
            online: AtomicUsize::new(0),
            offline: AtomicUsize::new(0),
            hub,
            model,
            cache: cache.to_path_buf(),
            notes_seen: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl Steps for Scripted {
    async fn library(&self, offline: bool) -> Result<(), String> {
        if offline {
            self.offline.fetch_add(1, Ordering::SeqCst);
            return self.model.offline_lookup(&self.cache);
        }
        self.online.fetch_add(1, Ordering::SeqCst);
        self.notes_seen.lock().unwrap().push(note_for(&[&self.model.repo]));
        self.script.lock().unwrap().pop_front().unwrap_or_else(|| self.then.clone())
    }

    async fn hub(&self) -> Hub {
        Hub { bases: self.hub.bases.clone(), users: self.hub.users }
    }
}

const METADATA: &str = "huggingface_hub.errors.FileMetadataError: Distant resource does not seem to be on huggingface.co. It is possible that a configuration issue prevents you from downloading resources from https://huggingface.co. Please check your firewall and proxy settings and make sure your SSL certificates are updated.";

fn quick() -> Pace {
    Pace(vec![Duration::from_millis(1); 4])
}

fn dead_hub() -> Hub {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    Hub { bases: vec![format!("http://{addr}")], users: false }
}

fn cache() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

// ---------------------------------------------------------------------------
// Patience
// ---------------------------------------------------------------------------

/// The failure of 2026-11-22: the library's metadata check fails twice, then
/// the Hub is well. Nobody does anything; the fetch ends well, by the library
/// alone (the engine's own GET is not reached: its Hub here is dead).
#[tokio::test(flavor = "multi_thread")]
async fn metadata_failure_twice_then_success_needs_nobody() {
    let dir = cache();
    let lib = Scripted::new(Model::new("t/twice"), dir.path(), dead_hub(), vec![Err(METADATA.into()), Err(METADATA.into())], Ok(()));
    fetch("t/twice", dir.path(), &lib, &quick()).await.expect("the third try works");
    assert_eq!(lib.online.load(Ordering::SeqCst), 3);
    assert_eq!(lib.offline.load(Ordering::SeqCst), 0, "the engine's own GET is not needed for a hiccup");
    assert_eq!(note_for(&["t/twice"]), None, "a fetch that worked says nothing");
    // While it waited, it said it was trying again, and why.
    let seen = lib.notes_seen.lock().unwrap().clone();
    assert_eq!(seen[0], None);
    assert_eq!(seen[1], Some(Note::Retrying { cause: Cause::Metadata, try_no: 2, of: 5 }));
    assert_eq!(seen[2], Some(Note::Retrying { cause: Cause::Metadata, try_no: 3, of: 5 }));
}

/// What a person is told while a retry waits: trying again, and why.
#[test]
fn a_retry_says_it_is_one() {
    let said = Note::Retrying { cause: Cause::Metadata, try_no: 2, of: 5 }.told("the hearing model (1.6 GB)", "Hold again to try again.");
    assert_eq!(said, "Still fetching the hearing model (1.6 GB): the Hub answered oddly. Trying again (2 of 5).");
}

#[test]
fn the_pauses_grow_and_last_minutes() {
    let Pace(pauses) = Pace::patient();
    assert!(pauses.windows(2).all(|w| w[0] < w[1]), "{pauses:?}");
    let total: u64 = pauses.iter().map(|p| p.as_secs()).sum();
    assert!((120..=600).contains(&total), "{total}s of waiting: patient, not endless");
}

/// A failure that is not transient stops at once — no pause, no second try —
/// and says so in its reason. A pause that would be slept here is an hour.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_that_is_not_transient_stops_at_once_with_its_reason() {
    let hour = Pace(vec![Duration::from_secs(3600); 4]);
    let cases = [
        ("OSError: [Errno 28] No space left on device", false, Cause::Disk),
        ("low disk: 2GB free, need 3GB", false, Cause::Disk),
        ("huggingface_hub.errors.HfHubHTTPError: 403 Client Error: Forbidden for url: https://huggingface.co/api/models/x/y", false, Cause::Refused),
        ("huggingface_hub.errors.GatedRepoError: 401 Client Error.", false, Cause::Refused),
        ("huggingface_hub.errors.RepositoryNotFoundError: 404 Client Error", false, Cause::Missing),
        ("Client error '404 Not Found' for url 'https://hf.example/x'", true, Cause::UsersHub),
    ];
    for (n, (said, users, cause)) in cases.into_iter().enumerate() {
        let repo = format!("t/stops-{n}");
        let dir = cache();
        let hub = Hub { bases: dead_hub().bases, users };
        let lib = Scripted::new(Model::new(&repo), dir.path(), hub, vec![], Err(said.to_string()));
        let failure = tokio::time::timeout(Duration::from_secs(5), fetch(&repo, dir.path(), &lib, &hour))
            .await
            .expect("it did not wait")
            .expect_err("it did not work");
        assert_eq!(failure.cause, cause, "{said}");
        assert!(!cause.transient());
        assert_eq!(lib.online.load(Ordering::SeqCst), 1, "{said}: one try");
        assert_eq!(lib.offline.load(Ordering::SeqCst), 0);
        assert_eq!(note_for(&[&repo]), Some(Note::Failed(failure.clone())), "what the page reads");
        assert!(failure.tell("the hearing model (1.6 GB)", "Hold again to try again.").contains(cause.words()));
    }
}

/// The trouble that passes is tried again, to the end: five tries, then it
/// gives up and says it may pass.
#[tokio::test(flavor = "multi_thread")]
async fn a_transient_failure_that_never_passes_gives_up_after_every_try() {
    let dir = cache();
    let lib = Scripted::new(Model::new("t/never"), dir.path(), dead_hub(), vec![], Err(METADATA.into()));
    let failure = fetch("t/never", dir.path(), &lib, &quick()).await.unwrap_err();
    assert_eq!(failure.cause, Cause::Metadata);
    assert_eq!(lib.online.load(Ordering::SeqCst), 5);
    assert!(failure.tell("the hearing model (1.6 GB)", "Hold again to try again.").ends_with("Hold again to try again."));
}

// ---------------------------------------------------------------------------
// The engine's own GET
// ---------------------------------------------------------------------------

/// The library brought the big weight file and then fails on the small ones
/// for good, while the Hub answers the engine's own GET. The engine fetches
/// the rest, into the cache layout, so the offline lookup finds all four —
/// and the big file, already whole, is not asked for again.
#[tokio::test(flavor = "multi_thread")]
async fn when_the_library_fails_on_small_files_for_good_the_own_get_completes_the_model() {
    let dir = cache();
    let model = Model::new("t/fallback");
    let hub = model.hub();
    model.seed_finished(dir.path(), "weights.safetensors");
    let lib = Scripted::new(model, dir.path(), hub.hub(), vec![], Err(METADATA.into()));
    fetch("t/fallback", dir.path(), &lib, &quick()).await.expect("completed by the engine's own GET");

    assert_eq!(lib.online.load(Ordering::SeqCst), FALLBACK_AFTER, "the library had its tries first");
    assert!(lib.offline.load(Ordering::SeqCst) >= 1);
    lib.model.offline_lookup(dir.path()).expect("`local_files_only` finds every file");
    assert_eq!(hub.asked(&lib.model.resolve_path("weights.safetensors")), 0, "what was fetched is kept, not fetched again");
    for name in ["config.json", "tokenizer/vocab.json", "README.md"] {
        assert_eq!(hub.asked(&lib.model.resolve_path(name)), 1, "{name}");
    }
    // The layout the library reads: relative links into blobs/, the ref, nothing left half-written.
    let folder = repo_folder(dir.path(), "t/fallback");
    let link = std::fs::read_link(folder.join("snapshots").join(COMMIT).join("tokenizer/vocab.json")).unwrap();
    assert!(link.starts_with("../../../blobs"), "{link:?}");
    assert_eq!(std::fs::read_to_string(folder.join("refs/main")).unwrap(), COMMIT);
    let leftovers: Vec<_> = std::fs::read_dir(folder.join("blobs")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".incomplete")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(note_for(&["t/fallback"]), None);
}

/// The list of files is the Hub's own listing at the fetch's revision: a file
/// the engine has never heard of is fetched, one the listing omits is not.
#[tokio::test(flavor = "multi_thread")]
async fn the_files_come_from_the_hubs_listing_not_from_a_list_kept_here() {
    let dir = cache();
    let mut model = Model::new("t/listing");
    model.files = vec![("added-later.json", b"{}".to_vec()), ("config.json", b"{\"x\":2}".to_vec())];
    let hub = model.hub();
    complete_from_hub("t/listing", dir.path(), &hub.hub()).await.unwrap();
    model.offline_lookup(dir.path()).unwrap();
    assert_eq!(hub.asked("api/models/t/listing/revision/main"), 1, "the listing is asked at the fetch's revision");
    assert_eq!(REVISION, "main");
    // The library is asked for that same revision.
    assert!(library_code(LibraryStep::Snapshot, "t/listing", false).contains(&format!("revision={REVISION:?}")));
}

/// A half-written file is resumed from where it stopped, not begun again, and
/// only a whole, checked file is put in place.
#[tokio::test(flavor = "multi_thread")]
async fn a_partial_file_is_resumed_and_checked() {
    let dir = cache();
    let model = Model::new("t/resume");
    let hub = model.hub();
    let weights = &model.files[0].1;
    let blobs = repo_folder(dir.path(), "t/resume").join("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join(format!("{}.incomplete", sha256_of(weights))), &weights[..1500]).unwrap();
    complete_from_hub("t/resume", dir.path(), &hub.hub()).await.unwrap();
    model.offline_lookup(dir.path()).unwrap();
    assert_eq!(hub.ranges(&model.resolve_path("weights.safetensors")), vec!["bytes=1500-".to_string()]);
}

/// A body that is not what the Hub's listing says (wrong checksum, wrong
/// size) is never put in the cache.
#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_does_not_match_the_listing_is_not_kept() {
    let dir = cache();
    let model = Model::new("t/corrupt");
    let hub = model.hub();
    let mut bad = model.files[0].1.clone();
    bad[10] ^= 0xff;
    hub.set(&model.resolve_path("weights.safetensors"), Route::Body(bad));
    let err = complete_from_hub("t/corrupt", dir.path(), &hub.hub()).await.unwrap_err();
    assert!(format!("{err:#}").contains("checksum"), "{err:#}");
    let folder = repo_folder(dir.path(), "t/corrupt");
    assert!(!folder.join("blobs").join(sha256_of(&model.files[0].1)).exists());
    assert!(!folder.join("blobs").join(format!("{}.incomplete", sha256_of(&model.files[0].1))).exists(), "a bad part is not resumed from");
    assert!(!folder.join("refs/main").exists(), "the ref says the model is whole only when it is");
}

/// The Hub names a path; it never leaves the snapshot folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_climbs_out_of_the_snapshot_is_refused() {
    let dir = cache();
    let mut model = Model::new("t/climb");
    model.files = vec![("../../outside.json", b"{}".to_vec())];
    let hub = model.hub();
    let err = complete_from_hub("t/climb", dir.path(), &hub.hub()).await.unwrap_err();
    assert!(format!("{err:#}").contains("unsafe path"), "{err:#}");
    assert_eq!(hub.asked(&model.resolve_path("../../outside.json")), 0);
}

/// When the Hub refuses the engine's own GET too, the fetch stops there and
/// says why: the own GET is not a way around a refusal.
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_of_the_own_get_stops_the_fetch() {
    let dir = cache();
    let model = Model::new("t/refused");
    let hub = model.hub();
    hub.set("api/models/t/refused/revision/main", Route::Status(403));
    let lib = Scripted::new(model, dir.path(), hub.hub(), vec![], Err(METADATA.into()));
    let failure = fetch("t/refused", dir.path(), &lib, &quick()).await.unwrap_err();
    assert_eq!(failure.cause, Cause::Refused, "{failure}");
    assert_eq!(lib.online.load(Ordering::SeqCst), FALLBACK_AFTER, "stopped at the round that met the refusal");
}

// ---------------------------------------------------------------------------
// What is kept
// ---------------------------------------------------------------------------

/// A failed fetch deletes nothing: what the library finished and what is
/// half-written are both still there, byte for byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_fetch_keeps_the_partial_cache() {
    let dir = cache();
    let model = Model::new("t/keeps");
    model.seed_finished(dir.path(), "weights.safetensors");
    let folder = repo_folder(dir.path(), "t/keeps");
    let part = folder.join("blobs/aaaa.incomplete");
    std::fs::write(&part, b"half").unwrap();
    let before = tree(dir.path());
    assert!(before.len() >= 4, "{before:?}");

    // Stops at once (refused).
    let lib = Scripted::new(model, dir.path(), dead_hub(), vec![], Err("403 Client Error: Forbidden".into()));
    fetch("t/keeps", dir.path(), &lib, &quick()).await.unwrap_err();
    assert_eq!(tree(dir.path()), before);
    // Gives up after every try (the engine's own GET cannot reach a dead Hub either).
    let lib = Scripted::new(Model::new("t/keeps"), dir.path(), dead_hub(), vec![], Err(METADATA.into()));
    fetch("t/keeps", dir.path(), &lib, &quick()).await.unwrap_err();
    assert_eq!(tree(dir.path()), before);
}

/// Every file under `dir` with its bytes (links by their target).
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let kind = e.file_type().unwrap();
            let name = p.strip_prefix(dir).unwrap().display().to_string();
            if kind.is_symlink() {
                out.push((name, std::fs::read_link(&p).unwrap().display().to_string().into_bytes()));
            } else if kind.is_dir() {
                stack.push(p);
            } else {
                out.push((name, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Nothing in this module removes a file from the cache except the engine's
/// own half-written part that failed its check: the library's finished files
/// are never touched.
#[test]
fn the_fetch_deletes_nothing_it_did_not_write() {
    let code = include_str!("hf_fetch.rs");
    let production = &code[..code.find("#[cfg(test)]\n#[path").unwrap()];
    let removals: Vec<&str> = production.lines().filter(|l| l.contains("remove_file") || l.contains("remove_dir")).map(str::trim).collect();
    assert_eq!(
        removals,
        vec!["std::fs::remove_file(&part)?;", "let _ = std::fs::remove_file(&part);"],
        "a deletion in the fetch must be of the engine's own `.incomplete` part, in `fetch_blob`"
    );
}

// ---------------------------------------------------------------------------
// What the person is told
// ---------------------------------------------------------------------------

/// What the library's own words (this log is the first real failure, home path
/// masked) and the engine's are taken to mean.
#[test]
fn the_librarys_words_say_what_was_the_matter() {
    let log = format!("Fetching 14 files:   0%|          | 0/14 [01:25<?, ?it/s]\n{METADATA}\n\nThe above exception was the direct cause of the following exception:\n\nTraceback (most recent call last):\n  File \"~/.linggen/runtime/envs/tts/lib/python3.12/site-packages/huggingface_hub/file_download.py\", line 1908, in _raise_on_head_call_error\nhuggingface_hub.errors.LocalEntryNotFoundError: An error happened while trying to locate the file on the Hub and we cannot find the requested files in the local cache. Please check your connection and try again or make sure your Internet connection is on.");
    let table = [
        (log.as_str(), Cause::Metadata),
        ("httpx.ConnectError: [Errno 8] nodename nor servname provided, or not known", Cause::Connection),
        ("httpx.ReadTimeout: The read operation timed out", Cause::Connection),
        ("huggingface_hub.errors.HfHubHTTPError: 503 Server Error: Service Unavailable for url", Cause::Busy),
        ("HTTP Error 429: Too Many Requests", Cause::Busy),
        ("HTTP Error 502 Bad Gateway", Cause::Busy),
        ("Client Error 401: Unauthorized", Cause::Refused),
        ("Traceback ... KeyboardInterrupt", Cause::Interrupted),
        ("warm tts model failed (exit status: 1): Segmentation fault", Cause::Interrupted),
    ];
    for (said, cause) in table {
        assert_eq!(Failure::classify(said, false).cause, cause, "{said}");
    }
    let failure = Failure::classify(&log, false);
    assert!(failure.detail.len() <= 400 + 4, "the log keeps the end of it, not a screen of bars");
}

/// Whatever the matter and whichever model, what is said is at most 20 words,
/// says whether trying again can help, and says what was the matter.
#[test]
fn what_the_person_is_told_is_short_and_says_whether_trying_again_helps() {
    let again = "Hold again to try again.";
    let whats = [
        crate::runtime::what_is_fetched(crate::runtime::STT_MODEL),
        crate::runtime::what_is_fetched(crate::runtime::TTS_MODEL),
        crate::runtime::what_is_fetched(crate::runtime::PICTURE_MODEL),
        "her voice and hearing (about 4 GB)",
    ];
    for what in whats {
        for cause in Cause::ALL {
            let failure = Failure { cause, detail: "x".into() };
            let said = Note::Failed(failure).told(what, again);
            assert!(said.split_whitespace().count() <= 20, "{said}");
            assert!(said.contains(what) && said.contains(cause.words()), "{said}");
            assert_eq!(said.ends_with(again), cause.transient(), "a try again is offered only where it can help: {said}");
            assert_eq!(said.contains("won't help"), !cause.transient() && cause != Cause::Disk, "{said}");
            let retry = Note::Retrying { cause, try_no: 5, of: 5 }.told(what, again);
            assert!(retry.split_whitespace().count() <= 20 && retry.contains("Trying again"), "{retry}");
        }
    }
}

/// A fetch that begins again forgets what the last one said.
#[test]
fn a_new_try_forgets_the_old_reason() {
    failed("t/forget", Failure { cause: Cause::Busy, detail: String::new() });
    assert!(note_for(&["t/forget"]).is_some());
    forget(&["t/forget"]);
    assert_eq!(note_for(&["t/forget"]), None);
    // The newest of several wins (a hearing fetch builds the voice under it too).
    failed("t/older", Failure { cause: Cause::Busy, detail: String::new() });
    failed("t/newer", Failure { cause: Cause::Disk, detail: String::new() });
    assert!(matches!(note_for(&["t/older", "t/newer"]), Some(Note::Failed(f)) if f.cause == Cause::Disk));
}

// ---------------------------------------------------------------------------
// The one place
// ---------------------------------------------------------------------------

/// Every `.rs` file under `dir`, with the part before its test module.
fn production_sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            production_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.to_string_lossy().ends_with("_tests.rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            let end = text.find("#[cfg(test)]").unwrap_or(text.len());
            out.push((path.display().to_string(), text[..end].to_string()));
        }
    }
}

fn all_production_sources() -> Vec<(String, String)> {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src");
    let mut out = Vec::new();
    production_sources(&engine, &mut out);
    production_sources(&server, &mut out);
    out
}

/// The three models are fetched through one function. Fails when a fourth
/// fetch is written beside it: another call of the library from Rust, another
/// run of `fetch`, another `HF_ENDPOINT` handed to a fetch, another
/// `ensure_*` of a model, or a fourth row.
#[test]
fn the_models_are_fetched_through_one_place() {
    let sources = all_production_sources();
    let outside = |needle: &str, allowed: &[&str]| -> Vec<String> {
        sources
            .iter()
            .filter(|(path, text)| text.contains(needle) && !allowed.iter().any(|a| path.ends_with(a)))
            .map(|(path, _)| path.clone())
            .collect()
    };
    // The library is asked in Python only inside `library_code`.
    for needle in ["snapshot_download", "hf_hub_download", "tts.utils import load_model", "from_pretrained"] {
        let found = outside(needle, &["hf_fetch.rs"]);
        assert!(found.is_empty(), "`{needle}` written outside `hf_fetch::library_code`: {found:?}");
    }
    // `fetch` is called only by `ensure_hub_model`.
    let callers: Vec<_> = sources.iter().filter(|(_, t)| t.contains("hf_fetch::fetch(")).map(|(p, _)| p.clone()).collect();
    assert_eq!(callers.len(), 1, "{callers:?}");
    assert!(callers[0].ends_with("runtime.rs"));
    let runtime = &sources.iter().find(|(p, _)| p.ends_with("src/runtime.rs")).unwrap().1;
    assert_eq!(runtime.matches("hf_fetch::fetch(").count(), 1);
    assert_eq!(runtime.matches("async fn ensure_hub_model(").count(), 1);
    assert_eq!(runtime.matches("ensure_hub_model(model)").count(), 1, "one caller of the one fetch");
    // No `ensure_*` that fetches a model but this one, and no spawn of a library beside `LibraryRun`.
    assert!(!runtime.contains("fn ensure_tts_model") && !runtime.contains("fn ensure_stt_model") && !runtime.contains("fn ensure_picture_model"));
    assert_eq!(runtime.matches(".env(\"HF_ENDPOINT\"").count(), 1, "the fetch's Python is spawned once, in `LibraryRun::library`");
    // The three models are three rows; a model's repo is named in no other fetch.
    assert_eq!(crate::runtime::HUB_MODELS.len(), 3);
    let repos: Vec<&str> = crate::runtime::HUB_MODELS.iter().map(|m| m.repo).collect();
    assert_eq!(repos, [crate::runtime::TTS_MODEL, crate::runtime::STT_MODEL, crate::runtime::PICTURE_MODEL]);
    let stages: Vec<&str> = crate::runtime::HUB_MODELS.iter().map(|m| m.stage).collect();
    assert_eq!(stages, ["tts-model", "stt-model", "pictures-model"]);
    assert_eq!(runtime.matches("\n    HubModel {\n").count(), 3, "a fourth model is a fourth row, reviewed here");
}

/// Each model's stage goes to the same function: `run_stage` has no arm of
/// its own for a model.
#[test]
fn a_stage_of_a_model_has_no_fetch_of_its_own() {
    let runtime = include_str!("runtime.rs");
    let run_stage = &runtime[runtime.find("async fn run_stage").unwrap()..];
    let run_stage = &run_stage[..run_stage.find("\n}\n").unwrap()];
    for stage in ["tts-model", "stt-model", "pictures-model"] {
        assert!(!run_stage.contains(&format!("\"{stage}\"")), "`run_stage` names `{stage}`: a model's stage is `HUB_MODELS`'s");
    }
    assert!(run_stage.contains("ensure_hub_model"));
}

#[test]
fn the_library_is_asked_for_the_same_revision_the_engine_lists() {
    for (step, repo) in [(LibraryStep::Snapshot, "o/m")] {
        for offline in [false, true] {
            let code = library_code(step, repo, offline);
            assert!(code.contains("snapshot_download(\"o/m\"") && code.contains("\"main\""), "{code}");
            assert_eq!(code.contains("local_files_only=True"), offline, "{code}");
        }
    }
    // The hearing sidecar loads the model by this same offline call.
    assert!(include_str!("../../../src/runtime_py/mlx_voice.py").contains("snapshot_download(state[\"stt_id\"], local_files_only=True)"));
}
