//! The fetcher against a local server: no network, no engine.

use super::*;
use axum::{extract::State, http::StatusCode, routing::get, Router};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A stand-in object server: serves what it was given at
/// `/a/<first two hex>/<object>` and remembers what was asked, in order.
struct Server {
    base: String,
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct Shelf {
    objects: Arc<HashMap<String, Vec<u8>>>,
    log: Arc<Mutex<Vec<String>>>,
    failures_left: Arc<AtomicUsize>,
}

impl Server {
    /// `objects`: the path after `/a/` and the bytes served there.
    /// The first `fail_first` requests are answered 500.
    async fn start(objects: Vec<(String, Vec<u8>)>, fail_first: usize) -> Self {
        let log = Arc::new(Mutex::new(Vec::new()));
        let shelf = Shelf {
            objects: Arc::new(objects.into_iter().collect()),
            log: log.clone(),
            failures_left: Arc::new(AtomicUsize::new(fail_first)),
        };
        let app = Router::new().fallback(get(serve)).with_state(shelf);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self {
            base: format!("http://127.0.0.1:{port}/a/"),
            log,
        }
    }

    fn asked(&self) -> Vec<String> {
        self.log.lock_ok().clone()
    }
}

async fn serve(State(shelf): State<Shelf>, uri: axum::http::Uri) -> (StatusCode, Vec<u8>) {
    let path = uri.path().trim_start_matches("/a/").to_string();
    shelf.log.lock_ok().push(path.clone());
    let failing = shelf
        .failures_left
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    match shelf.objects.get(&path) {
        _ if failing => (StatusCode::INTERNAL_SERVER_ERROR, vec![]),
        Some(bytes) => (StatusCode::OK, bytes.clone()),
        None => (StatusCode::NOT_FOUND, vec![]),
    }
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn entry(path: &str, bytes: &[u8], pack: &str, order: u32) -> FileEntry {
    FileEntry {
        path: path.into(),
        sha256: sha(bytes),
        size: bytes.len() as u64,
        pack: pack.into(),
        order,
    }
}

/// Where the server keeps `file`'s bytes.
fn shelf_path(file: &FileEntry) -> String {
    format!("{}/{}", &file.sha256[..2], file.object())
}

fn served(files: &[(&FileEntry, &[u8])]) -> Vec<(String, Vec<u8>)> {
    files
        .iter()
        .map(|(file, bytes)| (shelf_path(file), bytes.to_vec()))
        .collect()
}

fn pack(dir: &std::path::Path, base: &str, files: Vec<FileEntry>) -> Pack {
    Pack {
        skill: "gallery".into(),
        dir: dir.to_path_buf(),
        manifest: Manifest {
            version: 1,
            base: base.into(),
            files,
        },
    }
}

fn fast(concurrency: usize) -> Tuning {
    Tuning {
        concurrency,
        first_delay: Duration::from_millis(20),
        max_delay: Duration::from_millis(80),
    }
}

struct Rig {
    skill: tempfile::TempDir,
    store: tempfile::TempDir,
    assets: Arc<Assets>,
}

impl Rig {
    fn new(concurrency: usize) -> Self {
        Self::with(fast(concurrency))
    }

    fn with(tuning: Tuning) -> Self {
        let (skill, store) = (
            tempfile::tempdir().expect("d"),
            tempfile::tempdir().expect("d"),
        );
        let assets = Arc::new(Assets::new(Store::new(store.path().to_path_buf()), tuning));
        Self {
            skill,
            store,
            assets,
        }
    }

    fn pack(&self, base: &str, files: Vec<FileEntry>) -> Pack {
        pack(self.skill.path(), base, files)
    }

    fn stored(&self, file: &FileEntry) -> bool {
        Store::new(self.store.path().to_path_buf()).holds(file)
    }

    async fn until(&self, what: &str, mut done: impl FnMut(&Rig) -> bool) {
        for _ in 0..200 {
            if done(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("gave up waiting for {what}");
    }
}

#[tokio::test]
async fn a_listed_file_that_is_missing_is_fetched_verified_and_stored() {
    let bytes = b"picture one";
    let file = entry("art/one.png", bytes, "ch1", 1);
    let server = Server::start(served(&[(&file, bytes)]), 0).await;
    let rig = Rig::new(2);
    rig.assets
        .set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    rig.assets.spawn_worker();

    rig.until("the file", |r| r.stored(&file)).await;
    let kept = std::fs::read(Store::new(rig.store.path().into()).path_of(&file)).expect("read");
    assert_eq!(kept, bytes);
    assert!(file.object().ends_with(".png"), "the extension stays");
    assert_eq!(server.asked(), [shelf_path(&file)]);
}

#[tokio::test]
async fn a_second_run_fetches_nothing() {
    let bytes = b"picture one";
    let file = entry("art/one.png", bytes, "ch1", 1);
    let server = Server::start(served(&[(&file, bytes)]), 0).await;
    let rig = Rig::new(2);
    rig.assets
        .set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    rig.assets.spawn_worker();
    rig.until("the file", |r| r.stored(&file)).await;

    // A reload, then a restart: the same manifest, the same store.
    rig.assets
        .set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    let second = Arc::new(Assets::new(Store::new(rig.store.path().into()), fast(2)));
    second.set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    second.spawn_worker();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.asked().len(), 1);
}

#[tokio::test]
async fn a_file_the_skill_folder_holds_is_not_fetched() {
    let bytes = b"already here";
    let file = entry("art/here.png", bytes, "ch1", 1);
    let server = Server::start(served(&[(&file, bytes)]), 0).await;
    let rig = Rig::new(2);
    std::fs::create_dir_all(rig.skill.path().join("art")).expect("dir");
    std::fs::write(rig.skill.path().join("art/here.png"), bytes).expect("write");
    rig.assets
        .set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    rig.assets.spawn_worker();

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(server.asked().is_empty());
    assert!(!rig.stored(&file));
}

#[tokio::test]
async fn wrong_bytes_are_rejected_and_never_stored() {
    let file = entry("art/one.png", b"the real bytes", "ch1", 1);
    let wrong_hash = b"the fake bytes";
    let wrong_size = b"short";
    let other = entry("art/two.png", b"the other bytes", "ch1", 2);
    let objects = vec![
        (shelf_path(&file), wrong_hash.to_vec()),
        (shelf_path(&other), wrong_size.to_vec()),
    ];
    let server = Server::start(objects, 0).await;
    // A long first pause: each file is tried exactly once while we look.
    let rig = Rig::with(Tuning {
        first_delay: Duration::from_secs(60),
        ..fast(2)
    });
    rig.assets.set_packs(vec![
        rig.pack(&server.base, vec![file.clone(), other.clone()])
    ]);
    rig.assets.spawn_worker();

    rig.until("both tried", |_| server.asked().len() >= 2).await;
    rig.until("both failed", |r| {
        r.assets
            .report("gallery")
            .is_some_and(|rep| rep.packs[0].failing == 2)
    })
    .await;
    assert_eq!(server.asked().len(), 2);
    assert!(!rig.stored(&file) && !rig.stored(&other));
    let leftovers: Vec<_> = std::fs::read_dir(rig.store.path()).expect("dir").collect();
    assert!(
        leftovers.is_empty(),
        "a rejected download left {leftovers:?}"
    );
    let report = rig.assets.report("gallery").expect("report");
    assert_eq!(report.packs[0].failing, 2);
}

#[tokio::test]
async fn files_come_in_order_when_one_goes_at_a_time() {
    let blobs: Vec<(&str, &[u8], u32)> = vec![
        ("art/c.png", b"third", 30),
        ("art/a.png", b"first", 10),
        ("art/b.png", b"second", 20),
    ];
    let files: Vec<FileEntry> = blobs
        .iter()
        .map(|(p, b, o)| entry(p, b, "ch1", *o))
        .collect();
    let pairs: Vec<(&FileEntry, &[u8])> = files.iter().zip(&blobs).map(|(f, b)| (f, b.1)).collect();
    let server = Server::start(served(&pairs), 0).await;
    let rig = Rig::new(1);
    rig.assets
        .set_packs(vec![rig.pack(&server.base, files.clone())]);
    rig.assets.spawn_worker();

    rig.until("all three", |r| files.iter().all(|f| r.stored(f)))
        .await;
    let want: Vec<String> = [&files[1], &files[2], &files[0]]
        .iter()
        .map(|f| shelf_path(f))
        .collect();
    assert_eq!(server.asked(), want);
}

#[tokio::test]
async fn a_failing_service_is_retried_until_it_answers() {
    let bytes = b"patience";
    let file = entry("art/one.png", bytes, "ch1", 1);
    let server = Server::start(served(&[(&file, bytes)]), 3).await;
    let rig = Rig::new(1);
    rig.assets
        .set_packs(vec![rig.pack(&server.base, vec![file.clone()])]);
    rig.assets.spawn_worker();

    rig.until("the file after retries", |r| r.stored(&file))
        .await;
    assert_eq!(server.asked().len(), 4);
}

#[tokio::test]
async fn a_request_for_a_listed_file_goes_to_the_front() {
    let blobs: Vec<(&str, &[u8], u32)> = vec![
        ("a.png", b"aa", 1),
        ("b.png", b"bb", 2),
        ("c.png", b"cc", 3),
    ];
    let files: Vec<FileEntry> = blobs
        .iter()
        .map(|(p, b, o)| entry(p, b, "ch1", *o))
        .collect();
    let pairs: Vec<(&FileEntry, &[u8])> = files.iter().zip(&blobs).map(|(f, b)| (f, b.1)).collect();
    let server = Server::start(served(&pairs), 0).await;
    let rig = Rig::new(1);
    rig.assets
        .set_packs(vec![rig.pack(&server.base, files.clone())]);

    assert_eq!(rig.assets.resolve("gallery", "c.png"), Resolved::Pending);
    assert_eq!(
        rig.assets.resolve("gallery", "nowhere.png"),
        Resolved::Unlisted
    );
    assert_eq!(rig.assets.resolve("other", "c.png"), Resolved::Unlisted);
    rig.assets.spawn_worker();

    rig.until("all three", |r| files.iter().all(|f| r.stored(f)))
        .await;
    assert_eq!(server.asked()[0], shelf_path(&files[2]));
    let stored = rig.assets.resolve("gallery", "c.png");
    assert_eq!(
        stored,
        Resolved::Stored(Store::new(rig.store.path().into()).path_of(&files[2]))
    );
}

#[tokio::test]
async fn the_report_counts_wanted_held_and_failing_per_pack() {
    let (held, missing, late) = (
        entry("a.png", b"held!", "ch1", 5),
        entry("b.png", b"missing", "ch1", 6),
        entry("c.png", b"later", "ch2", 9),
    );
    let rig = Rig::new(1);
    let store = Store::new(rig.store.path().into());
    std::fs::write(store.path_of(&held), b"held!").expect("write");
    // Nothing listens here: every fetch fails.
    rig.assets.set_packs(vec![rig.pack(
        "http://127.0.0.1:1/a/",
        vec![late.clone(), missing.clone(), held.clone()],
    )]);
    rig.assets.spawn_worker();
    rig.until("a failure", |r| {
        r.assets
            .report("gallery")
            .is_some_and(|rep| rep.total.held_files == 1 && rep.packs.iter().any(|p| p.failing > 0))
    })
    .await;

    let report = rig.assets.report("gallery").expect("report");
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["files"], 3);
    assert_eq!(json["bytes"], 5 + 7 + 5);
    assert_eq!(json["held_files"], 1);
    assert_eq!(json["held_bytes"], 5);
    assert_eq!(json["packs"][0]["pack"], "ch1");
    assert_eq!(json["packs"][0]["files"], 2);
    assert_eq!(json["packs"][0]["held_files"], 1);
    assert_eq!(json["packs"][0]["failing"], 1);
    assert_eq!(json["packs"][1]["pack"], "ch2");
    assert_eq!(json["packs"][1]["held_bytes"], 0);
    assert!(rig.assets.report("nobody").is_none());
}

fn declare(rig: &Rig, manifest: &str) -> Vec<Declared> {
    std::fs::write(rig.skill.path().join("assets.json"), manifest).expect("write");
    vec![Declared {
        skill: "gallery".into(),
        dir: rig.skill.path().to_path_buf(),
        file: "assets.json".into(),
    }]
}

fn manifest_of(file: &FileEntry) -> String {
    serde_json::json!({ "version": 1, "base": "https://127.0.0.1:1/a/", "files": [{
        "path": file.path, "sha256": file.sha256, "size": file.size,
        "pack": file.pack, "order": file.order }] })
    .to_string()
}

#[tokio::test]
async fn the_first_refresh_sweeps_unlisted_objects_and_later_ones_do_not() {
    let (listed, other) = (
        entry("a.png", b"listed", "p", 1),
        entry("b.png", b"other", "p", 1),
    );
    let rig = Rig::new(1);
    let store = Store::new(rig.store.path().into());
    std::fs::write(store.path_of(&listed), b"listed").expect("write");
    std::fs::write(store.path_of(&other), b"other").expect("write");

    rig.assets.refresh(declare(&rig, &manifest_of(&listed)));
    assert!(store.holds(&listed) && !store.holds(&other));

    std::fs::write(store.path_of(&other), b"other").expect("write");
    rig.assets.refresh(declare(&rig, &manifest_of(&listed)));
    assert!(store.holds(&other), "a reload does not sweep");
}

#[tokio::test]
async fn a_bad_manifest_is_set_aside_and_nothing_is_swept() {
    let held = entry("a.png", b"held", "p", 1);
    let rig = Rig::new(1);
    let store = Store::new(rig.store.path().into());
    std::fs::write(store.path_of(&held), b"held").expect("write");

    rig.assets.refresh(declare(&rig, "{ not a manifest"));
    assert!(store.holds(&held), "an unreadable manifest may list it");
    assert!(rig.assets.report("gallery").is_none());
    assert_eq!(rig.assets.resolve("gallery", "a.png"), Resolved::Unlisted);

    // Fixed, and read again: the skill has its pack.
    rig.assets.refresh(declare(&rig, &manifest_of(&held)));
    assert!(rig.assets.report("gallery").is_some());
}
