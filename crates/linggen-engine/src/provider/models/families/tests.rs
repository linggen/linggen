use super::*;
use super::evidence::{ListFailed, ModelLister, Target};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

const FIXTURE: &str = include_str!("../family_catalog/family-fixture.json");
const TODAY: &str = "2026-11-23";
const KEY: &str = "test-only-marker";
const ANTHROPIC: &str = "https://api.anthropic.com";

/// A provider whose list a test sets, and whose asks are counted.
#[derive(Default)]
struct Provider {
    lists: Mutex<std::collections::HashMap<&'static str, BTreeSet<String>>>,
    asks: AtomicUsize,
}

#[async_trait]
impl ModelLister for Arc<Provider> {
    async fn list(&self, target: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> {
        self.asks.fetch_add(1, Ordering::SeqCst);
        assert!(!target.key.is_empty());
        let preset = crate::config::providers::PRESETS.iter().find(|p| p.url == target.url).unwrap();
        self.lists.lock().unwrap().get(preset.key).cloned().ok_or(ListFailed)
    }
}

impl Provider {
    fn lists(&self, preset: &'static str, ids: &[&str]) {
        self.lists.lock().unwrap().insert(preset, ids.iter().map(|s| s.to_string()).collect());
    }
}

struct World {
    families: Families,
    provider: Arc<Provider>,
    settings: Config,
    creds: Credentials,
    snapshot: Arc<Snapshot>,
}

fn day(s: &str) -> NaiveDate { s.parse().unwrap() }

fn world(today: &str) -> World {
    let provider = Arc::new(Provider::default());
    provider.lists("anthropic", &["claude-opus-5", "claude-opus-5-5", "claude-sonnet-5-5", "claude-sonnet-5", "claude-haiku-5-5"]);
    let snapshot = Arc::new(Snapshot::parse(FIXTURE.as_bytes()).unwrap());
    let mut families = Families::with(Evidence::new(Arc::new(provider.clone()), std::time::Duration::from_secs(3600)), Some(day(today)));
    families.fixed_catalog = Some(snapshot.clone());
    let mut creds = Credentials::default();
    creds.set_endpoint_key("anthropic", ANTHROPIC, Some(KEY.into()));
    World { families, provider, settings: Config::default(), creds, snapshot }
}

impl World {
    async fn gather(&self) {
        self.families.gather(&self.settings, &self.creds, Some(("anthropic", ANTHROPIC))).await;
    }

    fn act(&mut self, family: &str, action: Action) -> Result<bool, Refusal> {
        self.families.act(&mut self.settings, &self.creds, ("anthropic", ANTHROPIC), family, action)
    }

    fn row(&self, family: &str) -> &ModelConfig {
        binding::row_of(&self.settings, "anthropic", family).expect("ticked")
    }

    /// A following row on an older model, as an earlier tick would have left it.
    fn ticked_on(&mut self, family: &str, model: &str, mode: Mode) -> String {
        let preset = crate::config::providers::PRESETS.iter().find(|p| p.key == "anthropic").unwrap();
        let facts = self.snapshot.entry(preset, model).unwrap();
        binding::tick(&mut self.settings, preset, family, facts, mode)
    }
}

#[tokio::test]
async fn a_tick_files_one_ordinary_row_with_an_opaque_id_and_the_real_endpoint() {
    let mut w = world(TODAY);
    assert_eq!(w.act("claude-opus", Action::Tick), Err(Refusal::NeedsChoice), "no list, no evidence, no tick");
    w.gather().await;
    assert_eq!(w.act("claude-opus", Action::Tick), Ok(true));
    let row = w.row("claude-opus");
    assert!(row.id.starts_with("fam-") && !row.id.contains("opus"), "{}", row.id);
    assert_eq!(row.provider, "anthropic");
    assert_eq!((row.url.as_str(), row.model.as_str()), (ANTHROPIC, "claude-opus-5-5"));
    assert!(row.api_key.is_none() && row.family_binding.as_ref().unwrap().follows());
    assert_eq!(w.act("claude-opus", Action::Tick), Ok(false), "a second tick changes nothing");
    assert_eq!(w.settings.models.len(), 1);
    assert_eq!(w.act("claude-opus", Action::Choose { model: "claude-opus-5".into() }).map(|_| ()), Ok(()));
    assert_eq!(w.act("never-seen", Action::Tick), Err(Refusal::Missing));
    let mut keyless = world(TODAY);
    keyless.creds = Credentials::default();
    assert_eq!(keyless.act("claude-opus", Action::Tick), Err(Refusal::NoKey));
}

#[tokio::test]
async fn a_following_row_moves_with_its_notice_and_keeps_its_id_and_references() {
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.settings.make_default(&id);
    w.settings.pet.model = id.clone();
    w.gather().await;
    assert!(w.families.follow(&mut w.settings, &w.creds));
    let row = w.row("claude-opus");
    assert_eq!((row.id.as_str(), row.model.as_str()), (id.as_str(), "claude-opus-5-5"));
    assert_eq!((w.settings.routing.default_models[0].as_str(), w.settings.pet.model.as_str()), (id.as_str(), id.as_str()));
    let notice = row.family_binding.as_ref().unwrap().notice().unwrap();
    assert_eq!((notice.seq(), notice.kind(), notice.from(), notice.to(), notice.on()), (1, MoveKind::Followed, "claude-opus-5", "claude-opus-5-5", TODAY));
    assert!(!w.families.follow(&mut w.settings, &w.creds), "nothing newer: no second move");
}

#[tokio::test]
async fn nothing_moves_without_evidence_a_key_a_fresh_catalog_or_a_following_row() {
    let cases: [(&str, fn(&mut World), &str); 5] = [
        ("no list gathered", |_| {}, TODAY),
        ("model not listed", |w| w.provider.lists("anthropic", &["claude-opus-5"]), TODAY),
        ("list fetch failed", |w| { w.provider.lists.lock().unwrap().clear(); }, TODAY),
        ("catalog older than 21 days", |_| {}, "2026-12-15"),
        ("key removed", |w| w.creds = Credentials::default(), TODAY),
    ];
    for (why, change, today) in cases {
        let mut w = world(today);
        w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
        if why != "no list gathered" {
            change(&mut w);
            w.gather().await;
        }
        assert!(!w.families.follow(&mut w.settings, &w.creds), "{why}");
        assert_eq!(w.row("claude-opus").model, "claude-opus-5", "{why}");
    }
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Pinned);
    w.gather().await;
    assert!(!w.families.follow(&mut w.settings, &w.creds), "a pinned row stays");
}

#[tokio::test]
async fn a_changed_key_discards_the_held_list_and_an_exact_row_never_moves() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    // An exact row: same endpoint, an older Opus, no binding.
    let mut exact = w.settings.models[0].clone();
    exact.id = "my-exact-opus".into();
    exact.family_binding = None;
    w.settings.models.push(exact);
    w.gather().await;
    w.creds.set_endpoint_key("anthropic", ANTHROPIC, Some("another-test-marker".into()));
    assert!(!w.families.follow(&mut w.settings, &w.creds), "a list held for the old key is not evidence for the new one");
    w.gather().await;
    assert_eq!(w.provider.asks.load(Ordering::SeqCst), 2);
    assert!(w.families.follow(&mut w.settings, &w.creds));
    let exact = w.settings.models.iter().find(|r| r.id == "my-exact-opus").unwrap();
    assert_eq!(exact.model, "claude-opus-5", "an exact row never moves");
    assert!(exact.family_binding.is_none());
}

#[tokio::test]
async fn every_notice_action_answers_only_the_move_it_names() {
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    let before = w.settings.clone();
    let same = |w: &World| toml::to_string(&w.settings).unwrap() == toml::to_string(&before).unwrap();
    // A stale number changes nothing, whatever the action.
    for action in [Action::Ack { moved: 9 }, Action::Back { moved: 9 }, Action::Pin { moved: Some(9) }] {
        assert_eq!(w.act("claude-opus", action), Ok(false));
        assert!(same(&w));
    }
    // 知道了: acknowledged once; a repeat does nothing.
    assert_eq!(w.act("claude-opus", Action::Ack { moved: 1 }), Ok(true));
    assert!(w.row("claude-opus").family_binding.as_ref().unwrap().notice().is_none());
    assert_eq!(w.act("claude-opus", Action::Ack { moved: 1 }), Ok(false));
    assert_eq!(w.row("claude-opus").model, "claude-opus-5-5");
    // 换回 after the notice was answered is stale.
    assert_eq!(w.act("claude-opus", Action::Back { moved: 1 }), Ok(false));
    assert_eq!(w.row("claude-opus").id, id);
}

#[tokio::test]
async fn going_back_pins_the_previous_model_after_access_is_checked_again() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    // The previous model left the provider's list: no way back.
    w.provider.lists("anthropic", &["claude-opus-5-5"]);
    w.creds.set_endpoint_key("anthropic", ANTHROPIC, Some("another-test-marker".into()));
    w.gather().await;
    assert_eq!(w.act("claude-opus", Action::Back { moved: 1 }), Err(Refusal::NoAccess));
    assert_eq!(w.row("claude-opus").model, "claude-opus-5-5");
    w.provider.lists("anthropic", &["claude-opus-5", "claude-opus-5-5"]);
    w.creds.set_endpoint_key("anthropic", ANTHROPIC, Some("third-test-marker".into()));
    w.gather().await;
    assert_eq!(w.act("claude-opus", Action::Back { moved: 1 }), Ok(true));
    let row = w.row("claude-opus");
    let held = row.family_binding.as_ref().unwrap();
    assert_eq!((row.model.as_str(), held.mode(), held.notice().is_none()), ("claude-opus-5", Mode::Pinned, true));
    assert!(!w.families.follow(&mut w.settings, &w.creds), "pinned: no move again");
    assert_eq!(w.act("claude-opus", Action::Back { moved: 1 }), Ok(false));
    // 恢复自动跟上: follows again and moves at once when the answer is clear.
    assert_eq!(w.act("claude-opus", Action::Follow), Ok(true));
    let row = w.row("claude-opus");
    assert_eq!((row.model.as_str(), row.family_binding.as_ref().unwrap().follows()), ("claude-opus-5-5", true));
    assert_eq!(row.family_binding.as_ref().unwrap().notice().unwrap().seq(), 2);
    // 固定在这个版本: pinned where it is; naming the latest move.
    assert_eq!(w.act("claude-opus", Action::Pin { moved: Some(1) }), Ok(false), "move 1 is not the latest");
    assert_eq!(w.act("claude-opus", Action::Pin { moved: Some(2) }), Ok(true));
    assert_eq!(w.act("claude-opus", Action::Pin { moved: Some(2) }), Ok(false));
}

#[tokio::test]
async fn resuming_follow_moves_nothing_when_the_answer_is_unclear() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Pinned);
    // No list held: the row follows again, but stays where it is.
    assert_eq!(w.act("claude-opus", Action::Follow), Ok(true));
    let row = w.row("claude-opus");
    assert_eq!(row.model, "claude-opus-5");
    assert!(row.family_binding.as_ref().unwrap().follows() && row.family_binding.as_ref().unwrap().notice().is_none());
}

#[tokio::test]
async fn a_refused_first_call_sends_the_row_back_pinned_with_one_notice() {
    let refusal = || anyhow::anyhow!("anthropic error (404 Not Found): {{\"type\":\"error\",\"error\":{{\"type\":\"not_found_error\",\"message\":\"model: claude-opus-5-5\"}}}}");
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    // Nothing moved yet: a refusal on a row with no outstanding move queues nothing.
    w.families.model_refused(&w.settings, &id, &refusal());
    assert!(!w.families.go_back_refused(&mut w.settings));
    w.families.follow(&mut w.settings, &w.creds);
    // Another kind of failure (a rate limit) does not send it back.
    w.families.model_refused(&w.settings, &id, &anyhow::anyhow!("rate limit (429): slow down"));
    assert!(!w.families.go_back_refused(&mut w.settings));
    // The model is not there, not allowed: back.
    w.families.model_refused(&w.settings, &id, &refusal());
    assert!(w.families.go_back_refused(&mut w.settings));
    let row = w.row("claude-opus");
    let held = row.family_binding.as_ref().unwrap();
    let said = held.notice().unwrap();
    assert_eq!((row.model.as_str(), held.mode(), said.kind(), said.from(), said.to()), ("claude-opus-5", Mode::Pinned, MoveKind::WentBack, "claude-opus-5-5", "claude-opus-5"));
    assert_eq!(said.seq(), 2);
    // Said once: a second refusal and a second drain change nothing; it stays pinned.
    w.families.model_refused(&w.settings, &id, &refusal());
    assert!(!w.families.go_back_refused(&mut w.settings));
    assert!(!w.families.follow(&mut w.settings, &w.creds));
    // 恢复自动跟上 later tries again.
    assert_eq!(w.act("claude-opus", Action::Ack { moved: 2 }), Ok(true));
}

#[tokio::test]
async fn a_move_the_person_acknowledged_or_a_pinned_row_is_never_sent_back() {
    let refusal = anyhow::anyhow!("openai error (404): The model `x` does not exist or you do not have access to it");
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    w.act("claude-opus", Action::Ack { moved: 1 }).unwrap();
    w.families.model_refused(&w.settings, &id, &refusal);
    assert!(!w.families.go_back_refused(&mut w.settings));
    assert_eq!(w.row("claude-opus").model, "claude-opus-5-5");
}

#[tokio::test]
async fn starter_families_are_ticked_once_per_endpoint_and_never_again() {
    let mut w = world(TODAY);
    let endpoint = ("anthropic", ANTHROPIC);
    assert!(!w.families.start_endpoint(&mut w.settings, &w.creds, endpoint), "a key not proven by a list ticks and records nothing");
    assert!(w.settings.family_starts.is_empty());
    w.gather().await;
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, endpoint));
    assert_eq!(w.settings.models.len(), 1);
    assert_eq!(w.row("claude-sonnet").model, "claude-sonnet-5-5");
    assert!(w.settings.family_starts.contains("anthropic"));
    // Unticked by hand, restarted, key saved again: it does not come back.
    let id = w.row("claude-sonnet").id.clone();
    assert_eq!(w.act("claude-sonnet", Action::Untick { replacement: None }), Ok(true));
    assert!(w.settings.models.is_empty());
    let _ = id;
    assert!(!w.families.start_endpoint(&mut w.settings, &w.creds, endpoint));
    assert!(w.settings.models.is_empty());
    // The record survives a save and a load.
    let again: Config = toml::from_str(&toml::to_string(&w.settings).unwrap()).unwrap();
    assert!(again.family_starts.contains("anthropic"));
}

#[tokio::test]
async fn an_endpoint_that_already_has_rows_is_not_a_new_connection() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5-5", Mode::Pinned);
    w.gather().await;
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, ("anthropic", ANTHROPIC)), "recorded");
    assert_eq!(w.settings.models.len(), 1, "no starter was ticked");
    assert!(w.settings.family_starts.contains("anthropic"));
}

#[tokio::test]
async fn unticking_removes_the_row_warns_of_references_and_guards_the_first_default() {
    let mut w = world(TODAY);
    w.gather().await;
    w.act("claude-opus", Action::Tick).unwrap();
    let id = w.row("claude-opus").id.clone();
    w.settings.pet.model = id.clone();
    let mut value = serde_json::to_value(&w.settings).unwrap();
    value["agents"] = serde_json::json!([{"id": "ling", "spec_path": "x", "model": id}]);
    w.settings = serde_json::from_value(value).unwrap();
    w.settings.make_default(&id);
    let cloud = crate::provider::models::LINGGEN_CLOUD_MODEL_ID;
    let seen = w.families.view(&w.settings, &w.creds, ("anthropic", ANTHROPIC)).unwrap();
    let json = serde_json::to_value(&seen).unwrap();
    let opus = json["families"].as_array().unwrap().iter().find(|f| f["family"] == "claude-opus").unwrap();
    let places: Vec<_> = opus["references"].as_array().unwrap().iter().map(|r| r["place"].as_str().unwrap()).collect();
    assert_eq!(places, ["default_chat", "pet", "agent"]);
    let before = toml::to_string(&w.settings).unwrap();
    assert_eq!(w.act("claude-opus", Action::Untick { replacement: None }), Err(Refusal::FirstDefault));
    assert_eq!(w.act("claude-opus", Action::Untick { replacement: Some(id.clone()) }), Err(Refusal::FirstDefault));
    assert_eq!(toml::to_string(&w.settings).unwrap(), before, "a refused untick changes nothing");
    assert_eq!(w.act("claude-opus", Action::Untick { replacement: Some(cloud.into()) }), Ok(true));
    assert!(w.settings.models.is_empty());
    assert_eq!(w.settings.routing.default_models[0], cloud);
    assert_eq!(w.settings.pet.model, id, "a reference is left unavailable, never moved to another provider");
    assert_eq!(w.creds.endpoint_key("anthropic", ANTHROPIC), Some(KEY), "the key stays");
}

#[tokio::test]
async fn choosing_a_version_pins_it_and_needs_a_stable_listed_chat_model() {
    let mut w = world(TODAY);
    w.gather().await;
    assert_eq!(w.act("claude-opus", Action::Choose { model: "claude-opus-4-8".into() }), Err(Refusal::NoAccess), "not in the list");
    assert_eq!(w.act("claude-opus", Action::Choose { model: "claude-sonnet-5-5".into() }), Err(Refusal::Missing), "another family");
    assert_eq!(w.act("claude-opus", Action::Choose { model: "claude-opus-5".into() }), Ok(true));
    let held = w.row("claude-opus").family_binding.as_ref().unwrap().clone();
    assert_eq!((w.row("claude-opus").model.as_str(), held.mode()), ("claude-opus-5", Mode::Pinned));
    assert_eq!(w.act("claude-opus", Action::Choose { model: "claude-opus-5".into() }), Ok(false));
}

#[tokio::test]
async fn the_view_lists_families_newest_first_with_price_and_notice() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    let json = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("anthropic", ANTHROPIC)).unwrap()).unwrap();
    assert_eq!(json["catalog"], serde_json::json!({"observed_at": TODAY, "age_days": 0, "stale": false}));
    let families: Vec<_> = json["families"].as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap()).collect();
    assert_eq!(families, ["claude-fable", "claude-haiku", "claude-sonnet", "claude-opus"]);
    let opus = json["families"].as_array().unwrap().iter().find(|f| f["family"] == "claude-opus").unwrap();
    assert_eq!((opus["ticked"].clone(), opus["mode"].clone()), (true.into(), "follow".into()));
    assert_eq!(opus["current"]["model"], "claude-opus-5-5");
    assert_eq!(opus["current"]["price"]["source"], "catalog");
    assert!(opus["current"]["price"]["input"].as_f64().unwrap() > 0.0);
    assert_eq!(opus["notice"]["move"], 1);
    assert_eq!(opus["notice"]["kind"], "followed");
    assert_eq!(opus["notice"]["can_go_back"], true);
    assert!(!json.to_string().contains(KEY));
    // A family never ticked shows what a tick would run.
    let sonnet = json["families"].as_array().unwrap().iter().find(|f| f["family"] == "claude-sonnet").unwrap();
    assert_eq!((sonnet["ticked"].clone(), sonnet["next"]["model"].clone()), (false.into(), "claude-sonnet-5-5".into()));
    // An endpoint without families has none.
    assert!(w.families.view(&w.settings, &w.creds, ("ollama", "http://127.0.0.1:11434")).is_none());
}

#[tokio::test]
async fn a_stale_legacy_save_cannot_revert_a_move_or_set_a_binding() {
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    let page_read = w.settings.clone();
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    // The page saves what it read before the move, with a hand edit and a forged binding.
    let mut posted = page_read.clone();
    posted.models[0].family_binding = None;
    posted.pet.show_text = false;
    let mut forged = posted.models[0].clone();
    forged.id = "forged".into();
    forged.family_binding = page_read.models[0].family_binding.clone();
    posted.models.push(forged);
    keep_bindings(&mut posted, &w.settings);
    let kept = posted.models.iter().find(|r| r.id == id).unwrap();
    assert_eq!(kept.model, "claude-opus-5-5");
    assert_eq!(kept.family_binding, w.settings.models[0].family_binding);
    assert!(posted.models.iter().find(|r| r.id == "forged").unwrap().family_binding.is_none());
    assert!(!posted.pet.show_text, "the rest of the save stands");
}

#[test]
fn a_row_without_the_field_runs_as_an_exact_row_and_the_field_survives_a_save() {
    // A config written before the field existed: a row without it.
    let mut table: toml::Table = toml::from_str(&toml::to_string(&Config::default()).unwrap()).unwrap();
    let row: toml::Table = toml::from_str(
        "id = \"kept\"\nprovider = \"anthropic\"\nurl = \"https://api.anthropic.com\"\nmodel = \"claude-opus-5\"",
    ).unwrap();
    table.insert("models".into(), toml::Value::Array(vec![toml::Value::Table(row)]));
    let config: Config = toml::from_str(&toml::to_string(&table).unwrap()).unwrap();
    assert!(config.models[0].family_binding.is_none() && config.family_starts.is_empty());
    assert!(!toml::to_string(&config).unwrap().contains("family_binding"), "nothing is written for an exact row");
    config.validate().unwrap();
    // A bound row, with the starts and other tables present, round-trips.
    let mut config = Config::default();
    let preset = crate::config::providers::PRESETS.iter().find(|p| p.key == "anthropic").unwrap();
    let snap = Snapshot::parse(FIXTURE.as_bytes()).unwrap();
    let id = binding::tick(&mut config, preset, "claude-opus", snap.entry(preset, "claude-opus-5").unwrap(), Mode::Follow);
    binding::follow_to(&mut config, &id, "claude-opus-5", snap.entry(preset, "claude-opus-5-5").unwrap(), day(TODAY));
    binding::mark_started(&mut config, "anthropic");
    let text = toml::to_string_pretty(&config).unwrap();
    let loaded: Config = toml::from_str(&text).unwrap();
    assert_eq!(loaded.models[0].family_binding, config.models[0].family_binding);
    assert!(loaded.family_starts.contains("anthropic"));
    // The released binary reads every other field and drops this one: the
    // row it saves is a plain row that still runs, and nothing moves it.
    let mut released: toml::Table = toml::from_str(&text).unwrap();
    released.remove("family_starts");
    for row in released["models"].as_array_mut().unwrap() {
        row.as_table_mut().unwrap().remove("family_binding");
    }
    let mut after: Config = toml::from_str(&toml::to_string(&released).unwrap()).unwrap();
    after.validate().unwrap();
    assert!(after.models[0].family_binding.is_none());
    assert_eq!((after.models[0].model.as_str(), after.models[0].url.as_str()), ("claude-opus-5-5", ANTHROPIC));
    assert_eq!(crate::provider::kinds::kind_of(&after.models[0]), crate::provider::kinds::Kind::Chat);
    let families = Families::with(Evidence::new(Arc::new(Arc::new(Provider::default())), std::time::Duration::from_secs(1)), Some(day(TODAY)));
    assert!(!families.follow(&mut after, &Credentials::default()), "an exact row is not followed");
}
