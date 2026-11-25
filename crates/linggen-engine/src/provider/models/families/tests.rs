use super::*;
use super::super::family_catalog::Target as Approved;
use super::evidence::{ListFailed, ModelLister, Target};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

const TODAY: &str = "2026-11-23";
const KEY: &str = "test-only-marker";
const ANTHROPIC: &str = "https://api.anthropic.com";

/// A provider whose list a test sets, and whose asks are counted.
#[derive(Default)]
struct Provider {
    lists: Mutex<std::collections::HashMap<&'static str, BTreeSet<String>>>,
    asks: AtomicUsize,
    /// Which list endpoint an address is, under the list the world runs on.
    bases: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ModelLister for Arc<Provider> {
    async fn list(&self, target: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> {
        self.asks.fetch_add(1, Ordering::SeqCst);
        assert!(!target.key.is_empty());
        let key = self.bases.lock().unwrap().iter().find(|(url, _)| credentials::same_endpoint("", url, "", target.url)).map(|(_, key)| key.clone()).unwrap();
        self.lists.lock().unwrap().get(key.as_str()).cloned().ok_or(ListFailed)
    }
}

impl Provider {
    fn knows(&self, list: &Bundle) {
        let places = list.offered(curated::Edition::International).chain(list.offered(curated::Edition::Mainland));
        *self.bases.lock().unwrap() = places.map(|p| (p.base.to_string(), p.endpoint_key.to_string())).collect();
    }

    fn lists(&self, preset: &'static str, ids: &[&str]) {
        self.lists.lock().unwrap().insert(preset, ids.iter().map(|s| s.to_string()).collect());
    }
}

struct World {
    families: Families,
    provider: Arc<Provider>,
    settings: Config,
    creds: Credentials,
    list: Arc<Bundle>,
}

fn day(s: &str) -> NaiveDate { s.parse().unwrap() }

fn world(today: &str) -> World {
    let provider = Arc::new(Provider::default());
    provider.lists("anthropic", &["claude-opus-5", "claude-opus-5-5", "claude-sonnet-5-5", "claude-sonnet-5", "claude-haiku-5-5"]);
    // The shipped list: anthropic approves opus-5-5, sonnet-5-5, haiku-5-5, fable-5-2; facts observed 2026-11-23.
    let list = curated::shipped().clone();
    provider.knows(&list);
    let mut families = Families::with(Evidence::new(Arc::new(provider.clone()), std::time::Duration::from_secs(3600)), Some(day(today)));
    families.fixed_catalog = Some(list.clone());
    let mut creds = Credentials::default();
    creds.set_endpoint_key("anthropic", ANTHROPIC, Some(KEY.into()));
    World { families, provider, settings: Config::default(), creds, list }
}

impl World {
    /// The same world under another list: the next one the owner publishes.
    fn under(&mut self, list: Bundle) {
        let list = Arc::new(list);
        self.provider.knows(&list);
        self.families.fixed_catalog = Some(list.clone());
        self.list = list;
    }

    async fn gather(&self) {
        self.families.gather(&self.settings, &self.creds, Some(("anthropic", ANTHROPIC))).await;
    }

    fn act(&mut self, family: &str, action: Action) -> Result<bool, Refusal> {
        self.families.act(&mut self.settings, &self.creds, ("anthropic", ANTHROPIC), family, action)
    }

    fn row(&self, family: &str) -> &ModelConfig {
        binding::row_of(&self.list, &self.settings, "anthropic", family).expect("ticked")
    }

    /// A following row on an older model, as an earlier tick would have left it.
    fn ticked_on(&mut self, family: &str, model: &str, mode: Mode) -> String {
        let place = self.list.place_at(ANTHROPIC).unwrap();
        let facts = self.list.facts_for(place.endpoint_key, model);
        binding::tick(&mut self.settings, &place, family, Approved { id: model, facts }, mode)
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
async fn only_the_first_call_on_a_moved_model_can_send_the_row_back() {
    let refusal = anyhow::anyhow!("anthropic error (404 Not Found): not_found_error: model: claude-opus-5-5");
    let mut w = world(TODAY);
    let id = w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    assert!(!w.families.is_armed(&id), "nothing has moved");
    w.families.follow(&mut w.settings, &w.creds);
    assert!(w.families.is_armed(&id));
    // The moved model answers once: a refusal weeks later is not the first call.
    w.families.model_answered(&id);
    assert!(!w.families.is_armed(&id));
    w.families.model_refused(&w.settings, &id, &refusal);
    assert!(!w.families.go_back_refused(&mut w.settings));
    let row = w.row("claude-opus");
    assert_eq!((row.model.as_str(), row.family_binding.as_ref().unwrap().follows()), ("claude-opus-5-5", true));
    // A restart forgets what was armed: no go-back for a move of an earlier run.
    let mut later = world(TODAY);
    later.settings = w.settings.clone();
    assert!(!later.families.is_armed(&id));
}

#[tokio::test]
async fn a_row_whose_address_was_edited_away_is_an_exact_row() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.settings.models[0].url = "https://example.invalid/v1".into();
    w.gather().await;
    assert!(!w.families.follow(&mut w.settings, &w.creds));
    assert_eq!(w.settings.models[0].model, "claude-opus-5");
    let json = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("anthropic", ANTHROPIC)).unwrap()).unwrap();
    let opus = json["families"].as_array().unwrap().iter().find(|f| f["family"] == "claude-opus").unwrap();
    assert_eq!(opus["ticked"], false);
    assert_eq!(w.act("claude-opus", Action::Pin { moved: None }), Err(Refusal::Missing));
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
async fn the_view_lists_the_lists_families_in_its_order_with_price_and_notice() {
    let mut w = world(TODAY);
    w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
    w.gather().await;
    w.families.follow(&mut w.settings, &w.creds);
    let json = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("anthropic", ANTHROPIC)).unwrap()).unwrap();
    assert_eq!(json["catalog"], serde_json::json!({"observed_at": TODAY, "age_days": 0, "stale": false}));
    let families: Vec<_> = json["families"].as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap()).collect();
    assert_eq!(families, ["claude-fable", "claude-opus", "claude-sonnet", "claude-haiku"]);
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
    let list = curated::shipped().clone();
    let place = list.place_at(ANTHROPIC).unwrap();
    let at = |id| Approved { id, facts: list.facts_for(place.endpoint_key, id) };
    let id = binding::tick(&mut config, &place, "claude-opus", at("claude-opus-5"), Mode::Follow);
    binding::follow_to(&mut config, &id, "claude-opus-5", at("claude-opus-5-5"), day(TODAY));
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

// ── the list decides what is offered ──

const LAB: &str = "https://api.example-lab.test/v1";

/// The shipped list plus one provider and one model nobody wrote code for.
fn with_a_new_provider() -> Bundle {
    Bundle::variant(|v| {
        v["endpoints"]["lab"] = serde_json::json!({"base": LAB});
        v["international"].as_array_mut().unwrap().push(serde_json::json!({
            "key": "lab", "label": "Example Lab", "name": "openai", "sign_in": "key",
            "adapter": "openai-compatible", "default_endpoint": "lab",
            "entries": [
                {"key": "lab-one", "label": "Lab One", "endpoint": "lab", "type": "exact", "model": "lab-1", "starter": true},
                {"key": "lab-flash", "label": "Lab Flash", "endpoint": "lab", "type": "family", "model": "lab-flash-2",
                 "family": {"source_id": "lab", "name": "lab-flash"}, "starter": false}
            ]
        }));
    })
}

#[tokio::test]
async fn a_provider_and_a_model_added_to_the_list_are_offered_and_usable_with_no_code() {
    let mut w = world(TODAY);
    w.under(with_a_new_provider());
    // Offered: on the page's list, in the list's order, last.
    let offered: Vec<_> = w.list.offered(curated::Edition::International).map(|p| (p.provider.label.clone(), p.base.to_string())).collect();
    assert_eq!(offered.last().unwrap(), &("Example Lab".to_string(), LAB.to_string()));
    // Usable: a key at its address, a provider list that holds the models, a tick.
    w.creds.set_endpoint_key("openai", LAB, Some(KEY.into()));
    w.provider.lists("lab", &["lab-1", "lab-flash-2"]);
    w.families.gather(&w.settings, &w.creds, Some(("openai", LAB))).await;
    let at = ("openai", LAB);
    // The first verified connection ticks the exact starter, pinned, with no facts to borrow.
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, at));
    assert_eq!(w.families.act(&mut w.settings, &w.creds, at, "lab-flash", Action::Tick), Ok(true));
    let rows: Vec<_> = w.settings.models.iter().map(|r| (r.model.as_str(), r.url.as_str(), r.context_window)).collect();
    assert!(rows.contains(&("lab-flash-2", LAB, None)) && rows.contains(&("lab-1", LAB, None)), "{rows:?}");
    let pinned = w.settings.models.iter().find(|r| r.model == "lab-1").unwrap();
    assert!(!pinned.family_binding.as_ref().unwrap().follows(), "an exact starter stays put");
    assert_eq!(w.families.act(&mut w.settings, &w.creds, at, "lab-one", Action::Follow), Err(Refusal::Unsupported));
    // Chat uses them: both rows are chat rows the model manager serves.
    let manager = crate::provider::models::ModelManager::new_with_credentials(w.settings.models.clone(), &w.creds);
    for row in &w.settings.models {
        assert_eq!(crate::provider::kinds::kind_in(row, &w.list), crate::provider::kinds::Kind::Chat);
        assert!(manager.list_models().iter().any(|m| m.id == row.id), "{} is not served", row.model);
    }
}

#[tokio::test]
async fn a_list_update_never_ticks_a_starter() {
    let mut w = world(TODAY);
    w.gather().await;
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, ("anthropic", ANTHROPIC)));
    assert_eq!(w.settings.models.len(), 1, "the first connection ticked Sonnet");
    w.act("claude-sonnet", Action::Untick { replacement: None }).unwrap();
    // The owner now marks Opus a starter too, and approves a newer Sonnet.
    w.under(Bundle::variant(|v| {
        let anthropic = v["international"].as_array_mut().unwrap().iter_mut().find(|p| p["key"] == "anthropic").unwrap();
        for entry in anthropic["entries"].as_array_mut().unwrap() { entry["starter"] = true.into(); }
    }));
    w.gather().await;
    assert!(!w.families.start_endpoint(&mut w.settings, &w.creds, ("anthropic", ANTHROPIC)), "already recorded");
    assert!(!w.families.follow(&mut w.settings, &w.creds));
    assert!(w.settings.models.is_empty(), "a list update ticks nothing");
}

// ── THE GUARD: a key is filed under its exact address and goes nowhere else ──

/// The anthropic endpoint at `base`, its facts following it (as the owner's builder writes them).
fn anthropic_moved_to(base: &'static str) -> Bundle {
    Bundle::variant(move |v| {
        v["endpoints"]["anthropic"]["base"] = base.into();
        for record in v["facts"].as_array_mut().unwrap() {
            if record["address"] == ANTHROPIC { record["address"] = base.into(); }
        }
    })
}

#[tokio::test]
async fn a_list_that_moves_an_address_gives_a_new_connection_with_no_key_and_sends_none_there() {
    for new in ["https://api.anthropic.example.test", "https://api.anthropic.com/proxy/v2"] {
        let mut w = world(TODAY);
        w.ticked_on("claude-opus", "claude-opus-5", Mode::Follow);
        w.gather().await;
        assert_eq!(w.provider.asks.load(Ordering::SeqCst), 1);
        // Anthropic's native client with no address written: its key is filed at its own default base.
        let mut bare = w.settings.models[0].clone();
        (bare.id, bare.url, bare.family_binding) = ("bare".into(), String::new(), None);
        w.settings.models.push(bare);
        w.creds.set_endpoint_key("anthropic", "", Some(KEY.into()));
        let (config, creds) = (toml::to_string(&w.settings).unwrap(), serde_json::to_string(&w.creds).unwrap());
        w.under(anthropic_moved_to(new));
        w.provider.lists("anthropic", &["claude-opus-5-5"]);
        // Everything the engine does on a list update, and a page read of both addresses.
        w.families.gather(&w.settings, &w.creds, None).await;
        w.families.gather(&w.settings, &w.creds, Some(("anthropic", new))).await;
        assert!(!w.families.follow(&mut w.settings, &w.creds), "{new}: the old row is not this endpoint's family now");
        assert!(!w.families.start_endpoint(&mut w.settings, &w.creds, ("anthropic", new)), "{new}: no key, no connection");
        assert_eq!(w.provider.asks.load(Ordering::SeqCst), 1, "{new}: no request was made, so none carried a key");
        // Nothing was written, and no key resolves at the new address.
        assert_eq!((toml::to_string(&w.settings).unwrap(), serde_json::to_string(&w.creds).unwrap()), (config, creds), "{new}");
        let probe: ModelConfig = serde_json::from_value(serde_json::json!({"id": "p", "provider": "anthropic", "url": new, "model": "-"})).unwrap();
        assert!(credentials::resolve_api_key(&probe, &w.creds).is_none(), "{new}");
        assert_eq!(w.creds.endpoint_key("anthropic", ANTHROPIC), Some(KEY), "{new}: the old key stays at the old address");
        let seen = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("anthropic", new)).unwrap()).unwrap();
        assert!(seen["families"].as_array().unwrap().iter().all(|f| f["hold"] == "no_key" && f["ticked"] == false), "{new}");
        // The old address is no longer a connection the list knows.
        assert!(w.families.view(&w.settings, &w.creds, ("anthropic", ANTHROPIC)).is_none(), "{new}");
    }
}

#[test]
fn a_list_update_does_not_touch_keys_it_has_no_writer_for() {
    // The only code that copies a key to another address is the settings page's
    // address edit (`Edit::Address`). Nothing under `provider/models` writes keys.
    fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() { visit(&path, hits); continue; }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("tests.rs") { continue; }
            let text = std::fs::read_to_string(&path).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or("");
            if production.contains("set_endpoint_key(") { hits.push(path.display().to_string()); }
        }
    }
    let mut hits = Vec::new();
    for dir in ["families", "curated", "family_catalog"] {
        visit(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider/models").join(dir), &mut hits);
    }
    assert!(hits.is_empty(), "a list or family path writes a key: {hits:?}");
}

// ── an exact entry with no facts ──

#[tokio::test]
async fn an_exact_starter_with_no_facts_is_ticked_pinned_on_the_first_verified_connection() {
    let mut w = world(TODAY);
    const QWEN: &str = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1";
    w.creds.set_endpoint_key("qwen", QWEN, Some(KEY.into()));
    w.provider.lists("qwen-cloud", &["qwen-plus", "qwen-max"]);
    w.families.gather(&w.settings, &w.creds, Some(("qwen", QWEN))).await;
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, ("qwen", QWEN)));
    let row = &w.settings.models[0];
    assert_eq!((row.model.as_str(), row.url.as_str(), row.context_window, row.supports_tools), ("qwen-plus", QWEN, None, None));
    assert!(!row.family_binding.as_ref().unwrap().follows());
    assert!(w.list.facts_for("qwen-cloud", "qwen-plus").is_none(), "no price borrowed from DashScope");
    let seen = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("qwen", QWEN)).unwrap()).unwrap();
    assert_eq!(seen["rates_unavailable"], true);
}

#[tokio::test]
async fn a_key_on_a_provider_with_nothing_approved_is_still_recorded_as_its_first_connection() {
    let mut w = world(TODAY);
    const GROQ: &str = "https://api.groq.com/openai/v1";
    w.creds.set_endpoint_key("groq", GROQ, Some(KEY.into()));
    w.provider.lists("groq", &["llama-3.3-70b-versatile"]);
    w.families.gather(&w.settings, &w.creds, Some(("groq", GROQ))).await;
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, ("groq", GROQ)));
    assert!(w.settings.models.is_empty() && w.settings.family_starts.contains("groq"));
    assert!(w.families.view(&w.settings, &w.creds, ("groq", GROQ)).is_none(), "nothing to tick, nothing shown");
}

// ── editions: what is offered changes, what a person has does not ──

const DEEPSEEK: &str = "https://api.deepseek.com/v1";

fn deepseek_world(choice: crate::provider::models::EditionChoice) -> World {
    let mut w = world(TODAY);
    w.creds.set_endpoint_key("deepseek", DEEPSEEK, Some(KEY.into()));
    w.provider.lists("deepseek", &["deepseek-flash", "deepseek-v4-pro"]);
    w.settings.model_catalog = crate::provider::models::ModelCatalog::chosen(choice);
    w
}

#[tokio::test]
async fn a_mainland_first_key_ticks_no_starter_and_a_later_switch_never_replays_one() {
    use crate::provider::models::EditionChoice::{International, Mainland};
    let mut w = deepseek_world(Mainland);
    w.families.gather(&w.settings, &w.creds, Some(("deepseek", DEEPSEEK))).await;
    let seen = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("deepseek", DEEPSEEK)).unwrap()).unwrap();
    assert!(seen["families"].as_array().unwrap().iter().all(|f| f["unverified"] == true && f["ticked"] == false), "{seen}");
    assert!(w.families.start_endpoint(&mut w.settings, &w.creds, ("deepseek", DEEPSEEK)), "the first connection is recorded");
    assert!(w.settings.models.is_empty(), "mainland entries are unverified: no starter");
    // Now the owner picks the international edition: the record stands, nothing is ticked.
    let (config, creds) = (toml::to_string(&w.settings).unwrap(), serde_json::to_string(&w.creds).unwrap());
    w.settings.model_catalog = crate::provider::models::ModelCatalog::chosen(International);
    assert!(!w.families.start_endpoint(&mut w.settings, &w.creds, ("deepseek", DEEPSEEK)), "no starter replay");
    assert!(!w.families.follow(&mut w.settings, &w.creds));
    assert_eq!(serde_json::to_string(&w.creds).unwrap(), creds, "no key moved or copied");
    assert_eq!(toml::to_string(&w.settings).unwrap().replace("[model_catalog]\nedition = \"international\"\n", ""), config.replace("[model_catalog]\nedition = \"mainland\"\n", ""), "no row or default moved");
    let seen = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("deepseek", DEEPSEEK)).unwrap()).unwrap();
    assert!(seen["families"].as_array().unwrap().iter().all(|f| f.get("unverified").is_none()), "the international line carries no mark: {seen}");
}

#[tokio::test]
async fn a_ticked_row_follows_the_same_target_in_either_edition() {
    use crate::provider::models::EditionChoice::{International, Mainland};
    let mut moved = Vec::new();
    for choice in [International, Mainland] {
        let mut w = deepseek_world(choice);
        w.provider.lists("deepseek", &["deepseek-flash"]);
        let place = w.list.place_at(DEEPSEEK).unwrap();
        binding::tick(&mut w.settings, &place, "deepseek-flash", Approved { id: "deepseek-flash", facts: None }, Mode::Follow);
        w.families.gather(&w.settings, &w.creds, None).await;
        w.families.follow(&mut w.settings, &w.creds);
        let view = serde_json::to_value(w.families.view(&w.settings, &w.creds, ("deepseek", DEEPSEEK)).unwrap()).unwrap();
        let flash = view["families"].as_array().unwrap().iter().find(|f| f["family"] == "deepseek-flash").unwrap().clone();
        assert_eq!(flash["ticked"], true, "{choice:?}: the tick stays on the page");
        moved.push((w.settings.models[0].model.clone(), w.settings.models[0].url.clone()));
    }
    assert_eq!(moved[0], moved[1], "the edition does not change what a family follows");
}
