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
//! Which values a model offers: the catalog's `reasoning_options` for it,
//! cut to what its wire can carry ([`wire_values`]).

use super::{family_catalog, ModelManager};
use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::provider::openai::OpenAiClient;

/// What the caller of a call has to say about effort, by who says it.
#[derive(Clone, Copy, Debug, Default)]
pub struct EffortAsk<'a> {
    /// An internal turn's own intent; beats the person's choices.
    pub forced: Option<&'a str>,
    pub session: Option<&'a str>,
    pub agent: Option<&'a str>,
    /// `[uses] chat_effort`; the caller passes it for interactive chat only.
    pub chat_default: Option<&'a str>,
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
        (Source::Choice, ask.chat_default),
        (Source::Legacy, row.reasoning_effort.as_deref()),
    ]
}

/// The values a wire can carry, all models of it. Exhaustive: a new kind of
/// provider says here what it carries.
fn wire_values(kind: ProviderKind) -> &'static [&'static str] {
    match kind {
        ProviderKind::OpenAiCompatible | ProviderKind::ChatGpt => &OpenAiClient::EFFORT_VALUES,
        // Nothing is forwarded to these today.
        ProviderKind::Anthropic | ProviderKind::Ollama | ProviderKind::Proxy => &[],
    }
}

/// Whether the backend has its own gate on a value it is given
/// (`OpenAiClient::apply_reasoning_effort`), so a legacy value is left to it.
fn backend_gates_itself(kind: ProviderKind) -> bool {
    matches!(kind, ProviderKind::OpenAiCompatible | ProviderKind::ChatGpt)
}

/// What a row offers: its catalog's values that its wire carries. Only a
/// key-signed-in row has facts here. A sign-in's capabilities must come from
/// its own backend, not from the file of API facts; the Claude sign-in is
/// left out until the docs say the field is accepted there.
fn choices(row: &ModelConfig) -> Vec<&'static str> {
    let wire = wire_values(row.provider_kind());
    let openai = row.provider_kind() == ProviderKind::OpenAiCompatible;
    if row.sign_in() != SignIn::Key || wire.is_empty()
        || (openai && !OpenAiClient::model_supports_reasoning(&row.model, OpenAiClient::gemini_wire(&row.url)))
    {
        return Vec::new();
    }
    let catalog = family_catalog::current();
    let Some(facts) = catalog.exact(row) else { return Vec::new() };
    wire.iter().copied().filter(|value| facts.effort_values().any(|v| v == *value)).collect()
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
        Source::Forced | Source::Legacy if backend_gates_itself(row.provider_kind()) => {
            Resolved { effort: Effort(Some(value.to_string())), held_back: None }
        }
        Source::Forced | Source::Legacy => omit(None),
    }
}

impl ModelManager {
    pub fn display_name(&self, row: &ModelConfig) -> String {
        family_catalog::current().exact(row).map_or_else(|| row.model.clone(), |facts| facts.name().to_string())
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

    fn gpt(model: &str, effort: Option<&str>) -> ModelConfig {
        row(model, "openai", "https://api.openai.com/v1", None, effort)
    }

    fn sent(row: &ModelConfig, ask: EffortAsk<'_>) -> Option<String> {
        resolve(row, &ask).effort.wire().map(str::to_string)
    }

    #[test]
    fn only_the_overlap_of_exact_catalog_values_and_wire_values_is_offered() {
        let mut row = gpt("gpt-5.4-pro", None);
        assert_eq!(choices(&row), ["medium", "high"]);
        row.provider = "ollama".into();
        assert!(choices(&row).is_empty());
        row.provider = "openai".into();
        row.url = "https://gateway.invalid/v1".into();
        assert!(choices(&row).is_empty());
    }

    #[test]
    fn a_sign_in_offers_nothing_until_its_own_backend_says() {
        // ChatGPT's capabilities come from its backend, not the API-facts file.
        assert!(choices(&row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None)).is_empty());
    }

    #[test]
    fn the_order_is_forced_session_agent_chat_row() {
        let r = gpt("gpt-5", Some("high"));
        let all = EffortAsk { forced: Some("low"), session: Some("high"), agent: Some("medium"), chat_default: Some("low") };
        assert_eq!(sent(&r, all).as_deref(), Some("low"), "forced");
        assert_eq!(sent(&r, EffortAsk { forced: None, ..all }).as_deref(), Some("high"), "session");
        assert_eq!(sent(&r, EffortAsk { forced: None, session: None, ..all }).as_deref(), Some("medium"), "agent");
        assert_eq!(sent(&r, EffortAsk { agent: None, forced: None, session: None, ..all }).as_deref(), Some("low"), "chat default");
        assert_eq!(sent(&r, EffortAsk::default()).as_deref(), Some("high"), "the legacy row value");
        assert_eq!(sent(&gpt("gpt-5", None), EffortAsk::default()), None, "provider default");
    }

    #[test]
    fn auto_is_the_provider_default_and_stops_the_walk() {
        let r = gpt("gpt-5", Some("high"));
        let chat_auto = EffortAsk { chat_default: Some("auto"), ..EffortAsk::default() };
        assert_eq!(resolve(&r, &chat_auto), Resolved { effort: Effort(None), held_back: None });
        let session_auto = EffortAsk { session: Some("auto"), chat_default: Some("high"), ..EffortAsk::default() };
        assert_eq!(sent(&r, session_auto), None);
    }

    #[test]
    fn a_value_the_model_lacks_is_the_provider_default_and_is_kept_and_told() {
        // gpt-5.4-pro takes medium and high: the person's low stays; this call omits it.
        let r = gpt("gpt-5.4-pro", None);
        let ask = EffortAsk { session: Some("low"), ..EffortAsk::default() };
        assert_eq!(resolve(&r, &ask), Resolved { effort: Effort(None), held_back: Some("low".into()) });
        // It does not fall through to a lower step.
        let ask = EffortAsk { session: Some("low"), chat_default: Some("high"), ..EffortAsk::default() };
        assert_eq!(sent(&r, ask), None);
        // The same value on a model that has it is sent, with nothing held back.
        let ask = EffortAsk { session: Some("low"), ..EffortAsk::default() };
        let on_gpt5 = resolve(&gpt("gpt-5", None), &ask);
        assert_eq!((on_gpt5.effort.wire(), on_gpt5.held_back), (Some("low"), None));
        // A model with no effort at all, and the chat default's value there.
        let ask = EffortAsk { chat_default: Some("high"), ..EffortAsk::default() };
        assert_eq!(resolve(&gpt("gpt-4o", None), &ask).held_back.as_deref(), Some("high"));
    }

    #[test]
    fn legacy_and_forced_values_keep_the_openai_adapters_own_gate_and_are_never_told() {
        let gateway = row("gpt-5.4", "openai", "https://gateway.invalid/v1", None, None);
        let agent = EffortAsk::agent(Some("low"));
        assert_eq!(resolve(&gateway, &agent), Resolved { effort: Effort(Some("low".into())), held_back: None });
        assert_eq!(resolve(&gateway, &EffortAsk::forced("low")).effort.wire(), Some("low"));
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        assert_eq!(resolve(&sol, &agent).effort.wire(), Some("low"), "the Responses gate is the old one");
        // Where no backend takes effort, a legacy value was never sent and still is not.
        let ollama = row("llama", "ollama", "http://localhost:11434", None, Some("high"));
        assert_eq!(sent(&ollama, EffortAsk::forced("low")), None);
        assert_eq!(resolve(&ollama, &agent).held_back, None);
    }

    #[test]
    fn a_default_that_does_not_fit_a_sign_in_row_omits_and_is_told() {
        let sol = row("gpt-6.2-sol", "chatgpt", "", Some("chatgpt_oauth"), None);
        let ask = EffortAsk { chat_default: Some("high"), ..EffortAsk::default() };
        assert_eq!(resolve(&sol, &ask), Resolved { effort: Effort(None), held_back: Some("high".into()) });
    }

    #[test]
    fn an_unknown_model_asks_for_nothing() {
        let models = ModelManager::new_with_credentials(vec![], &Default::default());
        let ask = EffortAsk { session: Some("high"), ..EffortAsk::default() };
        assert_eq!(models.resolve_effort("not-a-model", &ask).effort.wire(), None);
        assert!(models.effort_choices("gpt-6-luna").is_empty());
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
            ("src/server/api/uses.rs", 2, "the default card shows and saves the person's default; it builds no request"),
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
        let fields = [concat!(".reasoning", "_effort"), concat!(".chat", "_effort")];
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
