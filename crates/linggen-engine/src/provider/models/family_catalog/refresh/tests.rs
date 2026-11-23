use super::*;
use super::super::SHIPPED;

struct Fake(Option<reqwest::StatusCode>);
impl Transport for Fake {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<Reply> {
        assert_eq!(url, URL);
        assert_eq!(etag, Some("\"rev1\""));
        assert_eq!(limit, MAX_BYTES);
        match self.0 {
            Some(reqwest::StatusCode::NOT_MODIFIED) => Ok(Reply::Unchanged),
            Some(status) => anyhow::bail!("{status}"),
            None => anyhow::bail!("timeout"),
        }
    }
}

#[tokio::test]
async fn cold_start_304_and_failure_preserve_observation_and_last_good_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog");
    let now: DateTime<Utc> = "2026-12-01T10:00:00Z".parse().unwrap();
    std::fs::write(&path, &SHIPPED[..500]).unwrap();
    let mut cache = Cache::open(path.clone(), now);
    assert!(cache.snapshot.is_none());
    assert!(cache.due(now));
    assert!(cache.apply(now, Reply::Unchanged).is_err());
    let good = cache.apply(now, Reply::Body { bytes: SHIPPED.to_vec(), etag: Some("\"rev1\"".into()) }).unwrap().unwrap();
    assert!(good.is_stale(now.date_naive()));
    assert!(!cache.due(now));
    let mut cache = Cache::open(path.clone(), now);
    assert_eq!(cache.body, SHIPPED);
    let tomorrow = now + Duration::days(1);
    assert!(cache.refresh(tomorrow, &Fake(Some(reqwest::StatusCode::NOT_MODIFIED))).await.is_none());
    assert!(!Cache::open(path.clone(), tomorrow).due(tomorrow));
    assert_eq!(cache.snapshot.as_ref().unwrap().age_days(tomorrow.date_naive()), 9);
    let stored = std::fs::read(&path).unwrap();
    let due = tomorrow + Duration::days(1);
    assert!(cache.refresh(due, &Fake(Some(reqwest::StatusCode::NOT_FOUND))).await.is_none());
    assert!(!cache.due(due + Duration::minutes(14)));
    assert!(cache.refresh(due + Duration::minutes(15), &Fake(None)).await.is_none());
    assert!(!cache.due(due + Duration::minutes(44)));
    assert!(cache.due(due + Duration::minutes(45)));
    assert!(cache.apply(due, Reply::Body { bytes: SHIPPED[..500].to_vec(), etag: None }).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), stored);
    assert_eq!(cache.body, SHIPPED);
}

#[test]
fn a_failed_atomic_replace_does_not_publish() {
    let dir = tempfile::tempdir().unwrap();
    let now = "2026-11-24T12:00:00Z".parse().unwrap();
    let path = dir.path().join("catalog");
    let mut cache = Cache::open(path.clone(), now);
    let good = cache.apply(now, Reply::Body { bytes: SHIPPED.to_vec(), etag: None }).unwrap().unwrap();
    cache.path = dir.path().to_path_buf(); // cannot replace a directory
    let next = String::from_utf8_lossy(SHIPPED).replacen("2026-11-23", "2026-11-24", 1);
    assert!(cache.apply(now, Reply::Body { bytes: next.into_bytes(), etag: Some("\"new\"".into()) }).is_err());
    assert!(Arc::ptr_eq(cache.snapshot.as_ref().unwrap(), &good));
    assert_eq!(std::fs::read(&path).unwrap(), read(&path).unwrap());
    assert!(cache.etag.is_none());
}
