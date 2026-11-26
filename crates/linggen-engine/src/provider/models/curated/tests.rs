use super::bundle::{accept, decode, verify, MAX_BYTES};
use super::store::{Cache, Fetched};
use super::trust::OWNER_PUBLIC_KEY;
use super::{shipped, Bundle, Edition};
use base64::Engine as _;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::sync::Arc;

// The site's shared fixture, signed by its own throwaway key. The site's test
// proves these bytes equal its copy; this module proves the engine reads them.
const FIXTURE: &[u8] = include_bytes!("fixture/bundle.json");
const FIXTURE_SIGNATURE: &str = include_str!("fixture/bundle.json.sig");
const FIXTURE_KEY_HEX: &str = include_str!("fixture/public-key.hex");

fn day(s: &str) -> NaiveDate { s.parse().unwrap() }
fn today() -> NaiveDate { day("2026-11-24") }
fn now() -> DateTime<Utc> { "2026-11-24T12:00:00Z".parse().unwrap() }

fn fixture_key() -> [u8; 32] {
    let hex = FIXTURE_KEY_HEX.trim();
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() { *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap(); }
    key
}

/// A signer for variants of the fixture (a changed revision, a bad field).
struct Signer(Ed25519KeyPair);
impl Signer {
    fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Self(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
    }
    fn key(&self) -> [u8; 32] { self.0.public_key().as_ref().try_into().unwrap() }
    fn sign(&self, bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.0.sign(bytes).as_ref())
    }
}

/// The fixture with `change` applied: a list the owner could have written.
fn variant(change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    change(&mut value);
    format!("{value}\n").into_bytes()
}

fn fixture() -> Bundle { decode(FIXTURE).expect("fixture reads") }
fn seed() -> Arc<Bundle> { Arc::new(fixture()) }

// ── the shipped copy ──

#[test]
fn the_shipped_list_passes_the_same_check_and_carries_both_editions() {
    let list = shipped();
    assert!(list.revision() >= 1 && list.list_revision() >= 1);
    let mainland = list.providers(Edition::Mainland);
    assert!(!mainland.is_empty());
    assert!(mainland.iter().flat_map(|p| &p.entries).all(|e| e.unverified && !e.starter), "mainland stays unverified");
    let international = list.providers(Edition::International);
    assert!(international.iter().any(|p| p.key == "anthropic"));
}

#[test]
fn no_signature_is_needed_to_read_and_the_kept_signing_code_still_verifies() {
    // Reading takes bytes only.
    assert!(decode(FIXTURE).is_ok());
    // The kept signing code, on the signed fixture only: the shipped list is not signed (owner's decision, 2026-11-24).
    assert!(verify(FIXTURE, FIXTURE_SIGNATURE, &fixture_key()).is_ok());
}

#[test]
fn the_list_agrees_with_the_engine_on_what_it_runs() {
    use crate::provider::models::{catalog::LINGGEN_CLOUD_MODEL_ID, chatgpt_catalog::Snapshot};
    let list = shipped();
    for edition in [Edition::International, Edition::Mainland] {
        let providers = list.providers(edition);
        let models = |key: &str| -> Vec<&str> {
            providers.iter().filter(|p| p.key == key).flat_map(|p| &p.entries).map(|e| e.model.as_str()).collect()
        };
        assert_eq!(models("chatgpt"), Snapshot::seed().builtin_ids().iter().map(String::as_str).collect::<Vec<_>>(), "chatgpt built-ins");
        // Linggen Cloud lists exactly three uses: chat, live voice, read-aloud.
        assert_eq!(models("linggen-cloud"), [LINGGEN_CLOUD_MODEL_ID, "qwen-audio-3.1-realtime-flash", "qwen3-tts-flash"]);
        let live: Vec<_> = providers.iter().flat_map(|p| &p.entries).filter(|e| e.live.is_some()).collect();
        assert_eq!(live.len(), 2, "Qwen Audio Realtime only (on its own card and through Linggen Cloud); Gemini Live is development-only");
        for entry in &live {
            assert!(crate::live::PROVIDERS.iter().any(|p| Some(p.id) == entry.live.as_ref().map(|l| l.service.as_str()) && p.model == entry.model));
        }
        let speech: Vec<_> = providers.iter().flat_map(|p| &p.entries).filter(|e| e.speech.is_some()).collect();
        assert_eq!(speech.len(), 1, "one read-aloud voice, Linggen Cloud's");
        assert_eq!(
            (speech[0].model.as_str(), speech[0].speech.as_ref().map(|s| s.voice.as_str())),
            (crate::account::cloud_voice::SPEECH_MODEL, Some(crate::account::cloud_voice::SPEECH_VOICE))
        );
        assert_eq!(list.cloud_uses(edition).count(), 2, "the uses besides chat");
        assert!(list.lists_cloud_speech());
    }
}

/// Where Linggen Cloud's entry of this type sits in the shipped list's international side.
fn cloud_entry(v: &serde_json::Value, kind: &str) -> (usize, usize) {
    let providers = v["international"].as_array().unwrap();
    let c = providers.iter().position(|p| p["key"] == "linggen-cloud").unwrap();
    let e = providers[c]["entries"].as_array().unwrap().iter().position(|e| e["type"] == kind).unwrap();
    (c, e)
}

/// What Linggen Cloud's row may claim: only a live service the site holds keys for, and only the
/// read-aloud voice the engine has a wire for — whatever the list says.
#[test]
fn the_list_cannot_claim_a_cloud_use_the_engine_has_no_wire_for() {
    let with = |change: fn(&mut serde_json::Value)| {
        let mut value: serde_json::Value = serde_json::from_slice(shipped().raw()).unwrap();
        change(&mut value);
        format!("{value}\n").into_bytes()
    };
    assert!(decode(&with(|_| {})).is_ok());
    let changes: [(&str, fn(&mut serde_json::Value)); 7] = [
        ("another read-aloud model", |v| { let (c, e) = cloud_entry(v, "speech"); v["international"][c]["entries"][e]["model"] = "qwen3-tts-other".into(); }),
        ("another read-aloud voice", |v| { let (c, e) = cloud_entry(v, "speech"); v["international"][c]["entries"][e]["speech"]["voice"] = "Cherry".into(); }),
        ("a read-aloud starter", |v| { let (c, e) = cloud_entry(v, "speech"); v["international"][c]["entries"][e]["starter"] = true.into(); }),
        ("a speech entry with no descriptor", |v| { let (c, e) = cloud_entry(v, "speech"); v["international"][c]["entries"][e]["speech"] = serde_json::Value::Null; }),
        ("a live use for a service the site holds no key for", |v| {
            let (c, e) = cloud_entry(v, "live");
            v["international"][c]["entries"][e]["live"]["service"] = "google".into();
            v["international"][c]["entries"][e]["model"] = "gemini-3.8-live".into();
        }),
        ("a live use of a model the engine does not run", |v| { let (c, e) = cloud_entry(v, "live"); v["international"][c]["entries"][e]["model"] = "qwen-audio-9".into(); }),
        ("read-aloud under a key provider", |v| {
            let (c, e) = cloud_entry(v, "speech");
            let speech = v["international"][c]["entries"][e].clone();
            let at = v["international"].as_array().unwrap().iter().position(|p| p["key"] == "anthropic").unwrap();
            v["international"][at]["entries"].as_array_mut().unwrap().push(speech);
        }),
    ];
    for (name, change) in changes {
        assert!(decode(&with(change)).is_err(), "{name}");
    }
}

#[test]
fn the_shipped_list_joins_facts_exactly_and_borrows_none_for_qwen_cloud() {
    let list = shipped();
    assert!(list.facts_for("anthropic", "claude-sonnet-5-5").is_some());
    assert!(list.facts_for("anthropic", "claude-sonnet").is_none(), "no alias or prefix match");
    assert!(list.facts_for("deepseek", "deepseek-flash").is_some(), "the address mapping holds");
    assert!(list.facts_for("dashscope-cn", "qwen-plus").is_some());
    assert!(list.facts_for("qwen-cloud", "qwen-plus").is_none(), "no price from another region");
    assert!(list.facts_for("chatgpt", "gpt-6.2-sol").is_none(), "a binding has no address");
}

// ── what the list places ──

fn row(provider: &str, url: &str) -> crate::config::ModelConfig {
    serde_json::from_value(serde_json::json!({"id": "m", "provider": provider, "url": url, "model": "x"})).unwrap()
}

#[test]
fn a_row_is_placed_only_at_the_exact_address_and_wire_the_list_names() {
    let list = shipped();
    let at = |provider, url: &str| list.place_of(&row(provider, url)).map(|p| p.endpoint_key.to_string());
    assert_eq!(at("anthropic", "https://api.anthropic.com").as_deref(), Some("anthropic"));
    assert_eq!(at("anthropic", "https://api.anthropic.com/").as_deref(), Some("anthropic"), "a trailing slash is the same address");
    assert_eq!(at("anthropic", "").as_deref(), Some("anthropic"), "the native client's own base");
    // Not the same address, not the same place: no host, prefix or path match.
    for url in ["https://api.anthropic.com/v1", "http://api.anthropic.com", "https://api.anthropic.com.evil.test", "https://gateway.example.com"] {
        assert_eq!(at("anthropic", url), None, "{url}");
    }
    // The wire is part of the match: a compatible client at Anthropic's address is not Anthropic's list.
    assert_eq!(at("openai", "https://api.anthropic.com"), None);
    // Rows kept from the other edition still have their place (label, facts, price).
    assert_eq!(at("qwen", "https://dashscope.aliyuncs.com/compatible-mode/v1").as_deref(), Some("dashscope-cn"));
    assert!(list.facts_for("dashscope-cn", "qwen-plus").is_some());
    assert_eq!(list.label_for("whatever", "https://api.deepseek.com/v1"), "DeepSeek");
    assert_eq!(list.label_for("qwen", "https://my-gateway.example/v1"), list.label_for("qwen", "https://other.example"), "by name when the address is unknown");
}

#[test]
fn the_offered_edition_is_what_the_page_lists_and_a_new_entry_needs_no_code() {
    let list = shipped();
    let offered: Vec<_> = list.offered(Edition::International).map(|p| p.provider.key.as_str()).collect();
    assert_eq!(offered.first(), Some(&"chatgpt"));
    assert!(offered.contains(&"anthropic") && !offered.contains(&"zhipu"), "the international edition only");
    let grown = Bundle::variant(|v| {
        v["international"][2]["entries"].as_array_mut().unwrap().push(serde_json::json!({
            "key": "brand-new", "label": "Brand New", "endpoint": "anthropic", "type": "exact", "model": "claude-brand-new", "starter": false
        }));
    });
    let anthropic = grown.offered(Edition::International).find(|p| p.provider.key == "anthropic").unwrap();
    assert!(anthropic.tickable().any(|(family, entry)| family == "brand-new" && entry.model == "claude-brand-new"));
}

// ── the shared fixture ──

#[test]
fn the_fixture_reads_and_joins_as_the_site_built_it() {
    let list = fixture();
    assert_eq!((list.revision(), list.list_revision()), (7, 3));
    assert_eq!(list.facts_observed_on(), day("2026-11-20"));
    assert!(list.facts_for("deepseek", "deepseek-flash").is_some());
    assert!(list.facts_for("deepseek", "deepseek-flash-0731").is_none());
    assert!(list.facts_for("qwen-cloud", "qwen-plus").is_none());
    assert_eq!(list.providers(Edition::Mainland).len(), 1);
    assert_eq!(list.raw(), FIXTURE);
}

#[test]
fn a_tampered_body_signature_or_key_fails_the_kept_check() {
    let key = fixture_key();
    let mut body = FIXTURE.to_vec();
    body[40] ^= 1;
    assert!(verify(&body, FIXTURE_SIGNATURE, &key).is_err());
    let mut sig = FIXTURE_SIGNATURE.trim().to_string();
    sig.replace_range(..4, "AAAA");
    assert!(verify(FIXTURE, &sig, &key).is_err());
    assert!(verify(FIXTURE, "", &key).is_err());
    assert!(verify(FIXTURE, "not base64 !!", &key).is_err());
    assert!(verify(FIXTURE, FIXTURE_SIGNATURE, &Signer::new().key()).is_err());
    assert!(verify(FIXTURE, FIXTURE_SIGNATURE, &OWNER_PUBLIC_KEY).is_err(), "the fixture is not signed by the owner key");
    let signer = Signer::new();
    let bytes = variant(|_| {});
    assert!(verify(&bytes, &signer.sign(&bytes), &signer.key()).is_ok());
}

#[test]
fn an_invalid_list_is_refused_as_a_unit() {
    let bad: Vec<(&str, Box<dyn Fn(&mut serde_json::Value)>)> = vec![
        ("schema", Box::new(|v| v["schema"] = 3.into())),
        ("revision zero", Box::new(|v| v["revision"] = 0.into())),
        ("unknown field", Box::new(|v| { v["extra"] = 1.into(); })),
        ("bad date", Box::new(|v| v["facts_observed_at"] = "2026-02-30".into())),
        ("http endpoint", Box::new(|v| v["endpoints"]["anthropic"]["base"] = "http://api.anthropic.com".into())),
        ("base and binding", Box::new(|v| v["endpoints"]["chatgpt"]["base"] = "https://x.example".into())),
        ("unknown binding", Box::new(|v| v["endpoints"]["chatgpt"]["binding"] = "mystery".into())),
        ("unknown adapter", Box::new(|v| v["international"][1]["adapter"] = "bespoke".into())),
        ("unknown sign-in", Box::new(|v| v["international"][1]["sign_in"] = "magic".into())),
        ("name the config lacks", Box::new(|v| v["international"][1]["name"] = "newcomer".into())),
        ("missing endpoint", Box::new(|v| v["international"][1]["default_endpoint"] = "nowhere".into())),
        ("entry on a foreign endpoint", Box::new(|v| v["international"][1]["entries"][0]["endpoint"] = "deepseek".into())),
        ("duplicate provider", Box::new(|v| { let p = v["international"][0].clone(); v["international"].as_array_mut().unwrap().push(p); })),
        ("undeclared live model", Box::new(|v| v["international"][3]["entries"][1]["model"] = "qwen-audio-9".into())),
        ("undeclared live service", Box::new(|v| v["international"][3]["entries"][1]["live"]["service"] = "bing".into())),
        ("live starter", Box::new(|v| v["international"][3]["entries"][1]["starter"] = true.into())),
        ("live without descriptor", Box::new(|v| { v["international"][3]["entries"][1].as_object_mut().unwrap().remove("live"); })),
        ("pin with a matcher", Box::new(|v| v["international"][2]["entries"][0]["family"] = serde_json::json!({"source_id": "a", "name": "b"}))),
        ("orphan facts", Box::new(|v| v["facts"][0]["address"] = "https://orphan.example".into())),
        ("duplicate facts", Box::new(|v| { let f = v["facts"][0].clone(); v["facts"].as_array_mut().unwrap().push(f); })),
        ("facts date after the observation", Box::new(|v| v["facts"][0]["models"][0]["cost"]["as_of"] = "2026-11-21".into())),
    ];
    for (name, change) in bad {
        assert!(decode(&variant(change)).is_err(), "{name}");
    }
    assert!(decode(&vec![b' '; MAX_BYTES + 1]).is_err());
}

#[test]
fn an_older_or_conflicting_revision_is_refused_and_a_newer_or_identical_one_is_not() {
    let floor = fixture();
    let make = |revision: u32, note: &str| variant(|v| { v["revision"] = revision.into(); v["endpoints"]["chatgpt"]["note"] = note.into(); });
    assert!(accept(&make(6, "x"), &[&floor], today()).is_err(), "older");
    assert!(accept(&make(7, "different bytes"), &[&floor], today()).is_err(), "same revision, other bytes");
    assert_eq!(accept(&make(8, "x"), &[&floor], today()).unwrap().revision(), 8);
    assert!(accept(FIXTURE, &[&floor], today()).is_ok(), "the identical list is not older");
    let future = variant(|v| { v["revision"] = 9.into(); v["facts_observed_at"] = "2026-11-30".into(); });
    assert!(accept(&future, &[], today()).is_err(), "facts dated in the future");
}

// ── last good, 304 ──

fn cache(dir: &tempfile::TempDir) -> Cache {
    Cache::open(dir.path().join("list"), seed(), now())
}

fn body(revision: u32) -> Fetched {
    Fetched::Body { bytes: variant(|v| v["revision"] = revision.into()), etag: Some("\"r\"".into()) }
}

#[test]
fn a_good_refresh_is_kept_and_a_bad_one_leaves_the_last_good_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    assert!(cache.bundle.is_none() && cache.due(now()));
    assert!(cache.apply(now(), Fetched::Unchanged).is_err(), "a 304 with nothing to keep");
    assert_eq!(cache.apply(now(), body(8)).unwrap().unwrap().revision(), 8);
    assert!(!cache.due(now()));
    let stored = std::fs::read(dir.path().join("list")).unwrap();
    assert!(cache.apply(now(), body(6)).is_err(), "older");
    let same_number = variant(|v| { v["revision"] = 8.into(); v["endpoints"]["chatgpt"]["note"] = "other".into(); });
    assert!(cache.apply(now(), Fetched::Body { bytes: same_number, etag: None }).is_err(), "same revision, other bytes");
    assert!(cache.apply(now(), Fetched::Body { bytes: b"{}".to_vec(), etag: None }).is_err(), "not a list");
    assert_eq!(std::fs::read(dir.path().join("list")).unwrap(), stored, "disk untouched");
    assert_eq!(cache.bundle.as_ref().unwrap().revision(), 8);
    let reopened = Cache::open(dir.path().join("list"), seed(), now());
    assert_eq!(reopened.bundle.unwrap().revision(), 8);
}

#[test]
fn a_304_changes_the_check_time_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    cache.apply(now(), body(8)).unwrap();
    let later = now() + Duration::days(7);
    assert!(cache.due(later));
    let before = cache.bundle.clone().unwrap();
    assert!(cache.apply(later, Fetched::Unchanged).unwrap().is_none());
    assert!(!cache.due(later));
    assert_eq!(cache.bundle.as_ref().unwrap().raw(), before.raw());
    let reopened = Cache::open(dir.path().join("list"), seed(), later);
    assert_eq!(reopened.bundle.clone().unwrap().raw(), before.raw());
    assert!(!reopened.due(later), "the 304 moved the check time");
}

#[test]
fn a_damaged_cache_file_is_ignored_and_the_seed_stays() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("list"), b"2026-11-24T00:00:00Z\n\n{}").unwrap();
    assert!(cache(&dir).bundle.is_none());
    std::fs::write(dir.path().join("list"), b"garbage").unwrap();
    assert!(cache(&dir).bundle.is_none());
    assert_eq!(seed().revision(), 7);
}

// ── the transport ──

struct Fake { list: Vec<u8>, unchanged: bool }
impl super::transport::Transport for Fake {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<super::transport::Reply> {
        use super::transport::Reply;
        assert_eq!(url, "https://linggen.dev/model-catalog/v2.json", "one address, no signature file");
        if self.unchanged { assert!(etag.is_some()); return Ok(Reply::Unchanged); }
        assert_eq!(limit, MAX_BYTES);
        Ok(Reply::Body { bytes: self.list.clone(), etag: Some("\"r\"".into()) })
    }
}

#[tokio::test]
async fn refresh_takes_one_unsigned_body_and_a_bad_one_keeps_the_last_good() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = cache(&dir);
    let bad = Fake { list: b"{}".to_vec(), unchanged: false };
    assert!(cache.refresh(now(), &bad).await.is_none());
    assert!(cache.bundle.is_none() && !cache.due(now() + Duration::minutes(14)) && cache.due(now() + Duration::minutes(15)));
    let at = now() + Duration::days(1);
    let good = Fake { list: variant(|v| v["revision"] = 8.into()), unchanged: false };
    assert_eq!(cache.refresh(at, &good).await.unwrap().revision(), 8);
    let week = at + Duration::days(7);
    assert!(cache.refresh(week, &Fake { list: Vec::new(), unchanged: true }).await.is_none());
    assert_eq!(cache.bundle.as_ref().unwrap().revision(), 8);
}

// ── locks ──

/// One reader: no other file in the engine names the v2 address or the shipped files.
#[test]
fn only_the_curated_module_reads_the_list() {
    fn visit(dir: &std::path::Path, hits: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() { visit(&path, hits); continue; }
            if path.extension().is_none_or(|e| e != "rs") { continue; }
            let text = std::fs::read_to_string(&path).unwrap();
            if text.contains("model-catalog/v2") || text.contains("model-catalog-v2") { hits.push(path); }
        }
    }
    let mut hits = Vec::new();
    visit(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    hits.sort();
    let names: Vec<_> = hits.iter().map(|p| p.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["src/provider/models/curated/store.rs", "src/provider/models/curated/tests.rs"]);
}

// ── editions ──

#[test]
fn a_connection_the_other_edition_names_is_kept_not_offered_and_a_custom_one_is_neither() {
    let list = shipped();
    use super::Edition::{International, Mainland};
    const DASHSCOPE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
    // International-only and mainland-only providers are kept in the other edition.
    assert!(list.kept_out_of(Mainland, "anthropic", "https://api.anthropic.com"));
    assert!(list.kept_out_of(Mainland, "anthropic", ""), "Anthropic's own client at its own base");
    assert!(!list.kept_out_of(International, "anthropic", "https://api.anthropic.com"));
    assert!(list.kept_out_of(International, "qwen", DASHSCOPE));
    assert!(!list.kept_out_of(Mainland, "qwen", DASHSCOPE));
    // Both editions name DeepSeek: kept in neither. A person's own address is no one's to keep.
    for edition in [International, Mainland] {
        assert!(!list.kept_out_of(edition, "deepseek", "https://api.deepseek.com/v1"), "{edition:?}");
        assert!(!list.kept_out_of(edition, "openai", "https://my-proxy.example.test/v1"), "{edition:?}");
        assert!(!list.kept_out_of(edition, "ollama", "http://127.0.0.1:11434"), "{edition:?}");
    }
}

#[test]
fn the_offered_edition_is_the_line_a_connection_is_read_from() {
    let list = shipped();
    use super::Edition::{International, Mainland};
    let flash = |edition| list.place_at_in(edition, "https://api.deepseek.com/v1").unwrap().tickable()
        .map(|(family, e)| (family.to_string(), e.starter, e.unverified)).collect::<Vec<_>>();
    assert_eq!(flash(International), [("deepseek-flash".into(), true, false), ("deepseek-pro".into(), false, false)]);
    assert_eq!(flash(Mainland), [("deepseek-flash".into(), false, true), ("deepseek-pro".into(), false, true)]);
    // A place of one edition only is found from either side, with the same endpoint key.
    assert_eq!(list.place_at_in(Mainland, "https://api.anthropic.com").unwrap().endpoint_key, "anthropic");
    assert!(list.offered(Mainland).all(|p| p.unverified() || p.provider.entries.is_empty()), "every mainland line is marked unverified");
    assert!(list.offered(International).all(|p| !p.unverified()));
}

#[test]
fn an_unlisted_address_filed_under_a_shared_name_is_not_labelled_as_the_sign_in_provider() {
    // Linggen Cloud files under the name "openai"; a key row at some other address is not it.
    assert_eq!(shipped().label_for("openai", "http://127.0.0.1:55800/v1"), "OpenAI");
    assert_eq!(shipped().label_for("openai", "https://api.openai.com/v1"), "OpenAI");
}
