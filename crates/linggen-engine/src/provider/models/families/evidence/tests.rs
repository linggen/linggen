use super::*;
use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
use serde_json::json;

#[test]
fn a_key_has_one_revision_and_the_key_is_not_in_it() {
    assert_eq!(revision_of("test-only-marker"), revision_of("test-only-marker"));
    assert_ne!(revision_of("test-only-marker"), revision_of("another-test-marker"));
}

#[test]
fn pages_read_ids_and_the_next_cursor() {
    let compatible = json!({"data": [{"id": "models/gemini-3.8-flash"}, {"id": "gpt-6.2-sol"}, {"object": "x"}]});
    assert_eq!(read_page(&compatible, false), Some((vec!["gemini-3.8-flash".into(), "gpt-6.2-sol".into()], None)));
    let paged = json!({"data": [{"id": "a"}], "has_more": true, "last_id": "a"});
    assert_eq!(read_page(&paged, true), Some((vec!["a".into()], Some("a".into()))));
    let last = json!({"data": [{"id": "b"}], "has_more": false, "last_id": "b"});
    assert_eq!(read_page(&last, true), Some((vec!["b".into()], None)));
    assert_eq!(read_page(&json!({"error": "no"}), false), None);
}

/// A provider on this machine: the lister asks whatever address it is given.
async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn the_native_anthropic_list_is_read_page_by_page_with_the_key_in_its_header() {
    let app = Router::new().route("/v1/models", get(|headers: HeaderMap, Query(q): Query<std::collections::HashMap<String, String>>| async move {
        assert_eq!(headers["x-api-key"], "test-only-marker");
        assert!(headers.get("authorization").is_none());
        match q.get("after_id").map(String::as_str) {
            None => Json(json!({"data": [{"id": "claude-opus-5"}], "has_more": true, "last_id": "claude-opus-5"})),
            Some("claude-opus-5") => Json(json!({"data": [{"id": "claude-opus-5-5"}], "has_more": false})),
            Some(other) => panic!("unexpected cursor {other}"),
        }
    }));
    let url = serve(app).await;
    let ids = HttpLister::default().list(&Target { url: &url, native_anthropic: true, key: "test-only-marker" }).await.unwrap();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), ["claude-opus-5", "claude-opus-5-5"]);
}

#[tokio::test]
async fn a_compatible_list_uses_a_bearer_key_and_a_refusal_or_a_non_list_is_no_evidence() {
    let app = Router::new()
        .route("/ok/models", get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer test-only-marker");
            Json(json!({"data": [{"id": "deepseek-flash"}]}))
        }))
        .route("/refused/models", get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "no") }))
        .route("/odd/models", get(|| async { Json(json!({"models": []})) }))
        .route("/moved/models", get(|| async { axum::response::Redirect::temporary("/ok/models") }));
    let base = serve(app).await;
    let ask = |path: &str| {
        let url = format!("{base}/{path}");
        async move { HttpLister::default().list(&Target { url: &url, native_anthropic: false, key: "test-only-marker" }).await }
    };
    assert_eq!(ask("ok").await.unwrap().len(), 1);
    for path in ["refused", "odd", "moved", "absent"] {
        assert!(ask(path).await.is_err(), "{path}");
    }
}

#[tokio::test]
async fn a_held_list_answers_until_the_key_changes_or_it_is_stale() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counting(AtomicUsize);
    #[async_trait]
    impl ModelLister for Arc<Counting> {
        async fn list(&self, _: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(BTreeSet::from(["m".to_string()]))
        }
    }
    let preset = crate::config::providers::PRESETS.iter().find(|p| p.key == "anthropic").unwrap();
    let counting = Arc::new(Counting(AtomicUsize::new(0)));
    let evidence = Evidence::new(Arc::new(counting.clone()), Duration::from_secs(3600));
    assert!(evidence.held(preset, "test-only-marker").is_none());
    evidence.gather(preset, "test-only-marker").await.unwrap();
    evidence.gather(preset, "test-only-marker").await.unwrap();
    assert_eq!(counting.0.load(Ordering::SeqCst), 1);
    assert!(evidence.held(preset, "another-test-marker").is_none());
    evidence.gather(preset, "another-test-marker").await.unwrap();
    assert_eq!(counting.0.load(Ordering::SeqCst), 2);
    assert!(evidence.held(preset, "test-only-marker").is_none(), "the old key's list is gone");
    let stale = Evidence::new(Arc::new(counting.clone()), Duration::ZERO);
    stale.gather(preset, "test-only-marker").await.unwrap();
    std::thread::sleep(Duration::from_millis(2));
    assert!(stale.held(preset, "test-only-marker").is_none());
}
