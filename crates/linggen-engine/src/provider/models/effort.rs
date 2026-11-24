//! The one decision of a call's effort ("how hard the model thinks").
//!
//! Every model call asks [`ModelManager`] with an [`EffortAsk`]; the manager
//! resolves it here, after the call's actual model (fallbacks included) is
//! known, and hands the backend an [`Effort`] that only this module can make.
//! A backend never looks at a row's, an agent's, a session's or a default's
//! effort itself. Order, first one set wins:
//!
//! 1. the internal turn's own intent (the light turn's "low"), told to no one
//! 2. the session's choice (`SessionMeta::effort`)
//! 3. the agent spec's (legacy)
//! 4. `[uses] chat_effort`, asked for interactive chat only
//! 5. the model row's `reasoning_effort` (legacy)
//! 6. the provider's default: no field on the wire
//!
//! `"auto"` at any step is the provider's default on purpose: it stops the
//! walk, so a later step cannot bring a value back. A session's or the chat
//! default's value the model does not offer also means the provider's default
//! for that call; the choice is kept where it is stored, and the caller is
//! told ([`Resolved::held_back`]) so the conversation can say so once.
//! A model's effort choices come from its catalog, cut to what its wire
//! can carry. ChatGPT sign-in uses its own catalog, not API-key facts.

use super::{chatgpt_catalog, curated, ModelManager};
use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::provider::anthropic::AnthropicClient;
use crate::provider::openai::{responses_api, OpenAiClient};

/// What the caller of a call has to say about effort, by who says it.
#[derive(Clone, Copy, Debug, Default)]
pub struct EffortAsk<'a> {
    /// An internal turn's own intent; beats the person's choices.
    pub forced: Option<&'a str>,
    pub session: Option<&'a str>,
    pub agent: Option<&'a str>,
    /// `[uses] chat_effort`; the caller passes it for interactive chat only.
    pub remembered: Option<&'a std::collections::BTreeMap<String, String>>,
}

impl<'a> EffortAsk<'a> {
    pub fn forced(value: &'a str) -> Self {
        Self { forced: Some(value), ..Self::default() }
    }

    pub fn agent(value: Option<&'a str>) -> Self {
        Self { agent: value, ..Self::default() }
    }
}

/// The effort a backend sends. Only [`resolve`] makes one with a value, so a
/// backend cannot build a request from any other source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effort(Option<String>);

impl Effort {
    /// The value to put on the wire; `None`: send no effort field.
    pub fn wire(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub effort: Effort,
    /// The person's value (session or chat default) this model does not
    /// offer, so the call goes to the provider's default.
    pub held_back: Option<String>,
}

/// Where a candidate came from decides what is done with a value the model
/// does not offer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The turn's own intent: the OpenAI-compatible adapter's old gate
    /// decides, as it did; never told.
    Forced,
    /// The person's choice for this conversation: omit and tell.
    Choice,
    /// Stored before effort had a picker (agent spec, row): the OpenAI-
    /// compatible adapter's old gate decides, as it did; others omit.
    Legacy,
}

fn candidates<'a>(ask: &EffortAsk<'a>, row: &'a ModelConfig) -> [(Source, Option<&'a str>); 5] {
    [
        (Source::Forced, ask.forced),
        (Source::Choice, ask.session),
        (Source::Legacy, ask.agent),
        (Source::Choice, ask.remembered.and_then(|values| values.get(&row.id)).map(String::as_str)),
        (Source::Legacy, row.reasoning_effort.as_deref()),
    ]
}

/// What a row offers: its catalog's values that its wire carries.
fn choices(row: &ModelConfig) -> Vec<&'static str> {
    match row.sign_in() {
        SignIn::ChatGpt => chatgpt_choices(row, &chatgpt_catalog::current()),
        SignIn::Key => key_choices(row),
        // Claude sign-in does not send the effort field.
        SignIn::Claude | SignIn::LinggenAccount | SignIn::Unknown => Vec::new(),
    }
}

fn chatgpt_choices(row: &ModelConfig, catalog: &chatgpt_catalog::Snapshot) -> Vec<&'static str> {
    let levels = catalog.reasoning_levels(&row.model);
    if levels.is_empty() {
        // Before the backend supplies levels (including old caches), offer
        // the official baseline: low, medium and high.
        return responses_api::BASE_EFFORT_VALUES.to_vec();
    }
    responses_api::EFFORT_VALUES.iter().copied()
        .filter(|value| levels.iter().any(|level| level == *value))
        .collect()
}

fn key_choices(row: &ModelConfig) -> Vec<&'static str> {
    let wire: &[&str] = match row.provider_kind() {
        ProviderKind::OpenAiCompatible if OpenAiClient::model_supports_reasoning(
            &row.model, OpenAiClient::gemini_wire(&row.url),
        ) => &OpenAiClient::EFFORT_VALUES,
        ProviderKind::Anthropic => &AnthropicClient::EFFORT_VALUES,
        ProviderKind::OpenAiCompatible | ProviderKind::ChatGpt | ProviderKind::Ollama | ProviderKind::Proxy => return Vec::new(),
    };
    let catalog = curated::current();
    let Some(facts) = catalog.exact(row) else { return Vec::new() };
    wire.iter().copied().filter(|value| facts.effort_values().any(|v| v == *value)).collect()
}

/// Preserve the old gate for forced and legacy values outside the choices.
fn passes_legacy_gate(row: &ModelConfig, value: &str) -> bool {
    if row.sign_in() == SignIn::ChatGpt {
        return OpenAiClient::model_supports_reasoning(&row.model, false)
            && responses_api::BASE_EFFORT_VALUES.iter().any(|v| v.eq_ignore_ascii_case(value));
    }
    row.provider_kind() == ProviderKind::OpenAiCompatible
}

pub(super) fn resolve(row: &ModelConfig, ask: &EffortAsk<'_>) -> Resolved {
    let omit = |held_back| Resolved { effort: Effort(None), held_back };
    let Some((source, value)) = candidates(ask, row).into_iter()
        .find_map(|(source, value)| value.filter(|v| !v.trim().is_empty()).map(|v| (source, v)))
    else {
        return omit(None);
    };
    if value == "auto" {
        return omit(None);
    }
    if let Some(offered) = choices(row).into_iter().find(|offered| offered.eq_ignore_ascii_case(value)) {
        return Resolved { effort: Effort(Some(offered.to_string())), held_back: None };
    }
    match source {
        Source::Choice => omit(Some(value.to_string())),
        Source::Forced | Source::Legacy if passes_legacy_gate(row, value) => {
            Resolved { effort: Effort(Some(value.to_string())), held_back: None }
        }
        Source::Forced | Source::Legacy => omit(None),
    }
}

impl ModelManager {
    pub fn display_name(&self, row: &ModelConfig) -> String {
        curated::current().exact(row).map_or_else(|| row.model.clone(), |facts| facts.name().to_string())
    }

    /// The values the model `id` offers, in the wire's order: what the chat
    /// menu, the default card and a session's choice are checked against.
    pub fn effort_choices(&self, id: &str) -> Vec<&'static str> {
        self.models.get(self.key(id).as_ref()).map_or_else(Vec::new, |instance| choices(&instance.config))
    }

    /// What a call to `id` carries, and whether a person's value was held
    /// back. The streams ask this for the model they are about to call;
    /// the engine asks it to tell the conversation.
    pub fn resolve_effort(&self, id: &str, ask: &EffortAsk<'_>) -> Resolved {
        self.models.get(self.key(id).as_ref()).map_or_else(
            || Resolved { effort: Effort(None), held_back: None },
            |instance| resolve(&instance.config, ask),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, provider: &str, url: &str, auth: Option<&str>, effort: Option<&str>) -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": "r", "model": model, "provider": provider, "url": url,
            "auth_mode": auth, "reasoning_effort": effort,
        })).unwrap()
    }

    fn remembered(value: &str) -> std::collections::BTreeMap<String, String> {
        [("r".into(), value.into())].into()
    }

    fn claude(model: &str) -> ModelConfig {
        row(model, "anthropic", "https://api.anthropic.com", None, None)
    }

    fn sent(row: &ModelConfig, ask: EffortAsk<'_>) -> Option<String> {
        resolve(row, &ask).effort.wire().map(str::to_string)
    }

    #[test]
    fn only_the_overlap_of_exact_catalog_values_and_wire_values_is_offered() {
        let mut row = row("gpt-5.4-pro", "openai", "https://api.openai.com/v1", None, None);
        assert_eq!(choices(&row), ["medium", "high"]);
        row.provider = "ollama".into();
        assert!(choices(&row).is_empty());
        row.provider = "openai".into();
        row.url = "https://gateway.invalid/v1".into();
        assert!(choices(&row).is_empty());
    }

    #[test]
    fn claude_offers_what_the_catalog_lists_and_the_wire_carries() {
        assert_eq!(choices(&claude("claude-sonnet-5-5")), ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(choices(&claude("claude-opus-4-6")), ["low", "medium", "high", "max"]);
        assert_eq!(choices(&claude("claude-opus-4-5")), ["low", "medium", "high"]);
        // Budget-token-only models have no effort field.
        assert!(choices(&claude("claude-haiku-4-5")).is_empty());
        // Another host behind the same wire is not Anthropic's catalog.
        assert!(choices(&row("claude-sonnet-5-5", "anthropic", "https://gateway.invalid", None, None)).is_empty());
    }

    #[test]
    fn claude_sign_in_offers_no_effort() {
        assert!(choices(&row("claude-sonnet-5-5", "anthropic", "", Some("claude_oauth"), None)).is_empty());
    }

    #[test]
    fn chatgpt_sign_in_without_levels_offers_low_medium_high() {
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        assert_eq!(choices(&sol), ["low", "medium", "high"]);
    }

    #[test]
    fn chatgpt_sign_in_offers_only_the_exact_models_levels_the_wire_can_send() {
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        let mut catalog = chatgpt_catalog::Snapshot::seed();
        catalog.listed = chatgpt_catalog::listing::parse_list(&serde_json::json!({"models": [
            {"slug": "gpt-6.2-sol", "supported_reasoning_levels": [
                {"effort": "ultra"}, {"effort": "max"}, {"effort": "medium"},
                {"effort": "xhigh"}, {"effort": "future"}, {"effort": "medium"}
            ]},
            {"slug": "gpt-6.1-terra", "supported_reasoning_levels": [{"effort": "future"}]},
            {"slug": "gpt-6-luna", "supported_reasoning_levels": []},
        ]}));
        assert_eq!(chatgpt_choices(&sol, &catalog), ["medium", "xhigh", "max", "ultra"]);
        let terra = row("gpt-6.1-terra", "chatgpt", "", None, None);
        assert!(chatgpt_choices(&terra, &catalog).is_empty(), "a nonempty list does not fall back");
        let luna = row("gpt-6-luna", "chatgpt", "", None, None);
        assert_eq!(chatgpt_choices(&luna, &catalog), ["low", "medium", "high"]);
    }

    #[test]
    fn the_order_is_forced_session_agent_chat_row() {
        let r = row("claude-sonnet-5-5", "anthropic", "https://api.anthropic.com", None, Some("max"));
        let saved = remembered("xhigh");
        let all = EffortAsk { forced: Some("low"), session: Some("high"), agent: Some("medium"), remembered: Some(&saved) };
        assert_eq!(sent(&r, all).as_deref(), Some("low"));
        assert_eq!(sent(&r, EffortAsk { forced: None, ..all }).as_deref(), Some("high"));
        assert_eq!(sent(&r, EffortAsk { forced: None, session: None, ..all }).as_deref(), Some("medium"));
        assert_eq!(sent(&r, EffortAsk { agent: None, forced: None, session: None, ..all }).as_deref(), Some("xhigh"));
        assert_eq!(sent(&r, EffortAsk::default()).as_deref(), Some("max"), "the legacy row value");
        assert_eq!(sent(&claude("claude-sonnet-5-5"), EffortAsk::default()), None, "provider default");
    }

    #[test]
    fn auto_is_the_provider_default_and_stops_the_walk() {
        let r = row("claude-sonnet-5-5", "anthropic", "https://api.anthropic.com", None, Some("max"));
        let chat_auto = EffortAsk { remembered: Some(&remembered("auto")), ..EffortAsk::default() };
        assert_eq!(resolve(&r, &chat_auto), Resolved { effort: Effort(None), held_back: None });
        let session_auto = EffortAsk { session: Some("auto"), remembered: Some(&remembered("high")), ..EffortAsk::default() };
        assert_eq!(sent(&r, session_auto), None);
    }

    #[test]
    fn a_value_the_model_lacks_is_the_provider_default_and_is_kept_and_told() {
        // Opus 4.5 has no xhigh: the person's choice stays; this call omits it.
        let r = claude("claude-opus-4-5");
        let ask = EffortAsk { session: Some("xhigh"), ..EffortAsk::default() };
        assert_eq!(resolve(&r, &ask), Resolved { effort: Effort(None), held_back: Some("xhigh".into()) });
        // It does not fall through to a lower step.
        let ask = EffortAsk { session: Some("xhigh"), remembered: Some(&remembered("low")), ..EffortAsk::default() };
        assert_eq!(sent(&r, ask), None);
        // The same value on a model that has it is sent, with nothing held back.
        let ask = EffortAsk { session: Some("xhigh"), ..EffortAsk::default() };
        let on_sonnet = resolve(&claude("claude-sonnet-5-5"), &ask);
        assert_eq!((on_sonnet.effort.wire(), on_sonnet.held_back), (Some("xhigh"), None));
        // A model with no effort at all, and the chat default's value there.
        let ask = EffortAsk { remembered: Some(&remembered("high")), ..EffortAsk::default() };
        assert_eq!(resolve(&claude("claude-haiku-4-5"), &ask).held_back.as_deref(), Some("high"));
    }

    #[test]
    fn legacy_and_forced_values_keep_the_openai_adapters_own_gate_and_are_never_told() {
        let gpt = row("gpt-5.4", "openai", "https://gateway.invalid/v1", None, None);
        let agent = EffortAsk::agent(Some("low"));
        assert_eq!(resolve(&gpt, &agent), Resolved { effort: Effort(Some("low".into())), held_back: None });
        assert_eq!(resolve(&gpt, &EffortAsk::forced("low")).effort.wire(), Some("low"));
        // Where no gate exists, a legacy value was never sent and still is not.
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        assert_eq!(resolve(&sol, &agent).effort.wire(), Some("low"), "the Responses gate is the old one");
        assert_eq!(sent(&claude("claude-haiku-4-5"), agent), None);
        let ollama = row("llama", "ollama", "http://localhost:11434", None, Some("high"));
        assert_eq!(sent(&ollama, EffortAsk::forced("low")), None);
        assert_eq!(resolve(&ollama, &agent).held_back, None);
    }

    #[test]
    fn chatgpt_without_levels_sends_a_baseline_choice_but_not_an_unadvertised_extra() {
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        let ask = EffortAsk { remembered: Some(&remembered("high")), ..EffortAsk::default() };
        assert_eq!(resolve(&sol, &ask), Resolved { effort: Effort(Some("high".into())), held_back: None });
        let ask = EffortAsk { remembered: Some(&remembered("xhigh")), ..EffortAsk::default() };
        assert_eq!(resolve(&sol, &ask), Resolved { effort: Effort(None), held_back: Some("xhigh".into()) });
        assert_eq!(sent(&sol, EffortAsk::agent(Some("ultra"))), None);
    }

    #[test]
    fn an_unknown_model_asks_for_nothing() {
        let models = ModelManager::new_with_credentials(vec![], &Default::default());
        let ask = EffortAsk { session: Some("high"), ..EffortAsk::default() };
        assert_eq!(models.resolve_effort("not-a-model", &ask).effort.wire(), None);
        assert!(models.effort_choices("not-a-model").is_empty());
        assert_eq!(models.effort_choices("gpt-6-luna"), ["low", "medium", "high"], "the built-in is known");
    }

    /// The lock: the row's, agent's and the chat default's effort are read by
    /// the resolver and by the engine's builders of an [`EffortAsk`]
    /// (`AgentEngine::effort_ask`, `agent_effort_ask`), nowhere else. (A
    /// backend cannot build a request from another source: [`Effort`] is
    /// made by [`resolve`] alone.) A new reader fails here until it is looked
    /// at and listed.
    #[test]
    fn only_the_resolver_and_the_asks_read_the_effort_fields() {
        use std::path::{Path, PathBuf};
        const READERS: &[(&str, usize, &str)] = &[
            ("crates/linggen-engine/src/provider/models/effort.rs", 1, "the resolver reads the row's legacy value"),
            ("crates/linggen-engine/src/engine/effort_ask.rs", 3, "the engine's two ask builders"),
            ("crates/linggen-engine/src/engine/types.rs", 4, "the agent's value is set from its spec; the chat default is copied from settings"),
            ("src/server/api/uses.rs", 2, "the default card shows and saves the person's per-model value"),
            ("src/server/model_list.rs", 1, "the chat slider displays the remembered value"),
            ("crates/linggen-engine/src/config.rs", 1, "migrate the old value once"),
        ];
        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut all = Vec::new();
        for dir in ["crates/linggen-engine/src", "src"] {
            files(&root.join(dir), &mut all);
        }
        // Built in two halves so this file's own text does not match.
        let fields = [concat!(".reasoning", "_effort"), concat!(".model", "_effort")];
        let reads = |text: &str| fields.iter().map(|p| text.matches(p).count()).sum::<usize>();
        let mut found: Vec<(String, usize)> = all.iter().filter_map(|path| {
            let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            let n = reads(&std::fs::read_to_string(path).unwrap());
            (n > 0).then_some((rel, n))
        }).collect();
        found.sort();
        let mut listed: Vec<(String, usize)> = READERS.iter().map(|(f, n, _)| (f.to_string(), *n)).collect();
        listed.sort();
        assert_eq!(found, listed, "a new reader of an effort field: route it through EffortAsk, or list it with a reason");
    }
}
