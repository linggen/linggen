use super::*;

#[test]
fn shipped_contract_covers_bound_presets_and_preserves_exact_rows() {
    let seed = shipped();
    assert!(SHIPPED.len() <= 256 * 1024);
    assert_eq!(seed.providers["openrouter"].len(), 380);
    assert!(seed.providers["anthropic"].contains_key("claude-haiku-5-5"));
    assert!(seed.providers["deepseek"].contains_key("deepseek-v4-flash-vision-exp"));
    for (age, stale) in [(7, false), (8, true)] {
        let now = seed.observed + Days::new(age);
        assert_eq!(seed.age_days(now), age);
        assert_eq!(seed.is_stale(now), stale);
    }
}

#[test]
fn rejects_bad_schema_fields_dates_prices_duplicates_and_lost_coverage() {
    let seed = shipped();
    let base: serde_json::Value = serde_json::from_slice(SHIPPED).unwrap();
    let rejects = |v: serde_json::Value| {
        assert!(accept(&serde_json::to_vec(&v).unwrap(), seed, seed.observed).is_err());
    };
    for (key, value) in [
        ("schema", 2.into()),
        ("observed_at", "2026-02-30".into()),
        ("observed_at", "2026-11-22".into()),
        ("observed_at", "2028-11-22".into()),
        ("providers", serde_json::json!([])),
        ("request_body", serde_json::json!({})),
    ] {
        let mut v = base.clone();
        v[key] = value;
        rejects(v);
    }
    for (key, value) in [
        ("cost", serde_json::json!({"input": -1})),
        ("context", "unknown".into()),
        ("release_date", "2026-13".into()),
        ("modalities", serde_json::json!({"input": ["text"], "output": ["text", "text"]})),
    ] {
        let mut v = base.clone();
        v["providers"][0]["models"][0][key] = value;
        rejects(v);
    }
    let mut v = base.clone();
    let rows = v["providers"][0]["models"].as_array_mut().unwrap();
    rows.push(rows[0].clone());
    rejects(v);
    let mut v = base.clone();
    let rows = v["providers"][0]["models"].as_array_mut().unwrap();
    rows.truncate(rows.len() / 2);
    rejects(v);
    let mut v = base;
    let providers = v["providers"].as_array_mut().unwrap();
    providers.push(providers[0].clone());
    rejects(v);
    assert!(Snapshot::parse(&SHIPPED[..500]).is_err());
    assert!(Snapshot::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
}
