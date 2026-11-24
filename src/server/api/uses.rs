//! Owner's defaults: a snapshot for the card and one validated edit per PUT.

use std::sync::Arc;

use axum::{body::Bytes, extract::State, http::HeaderMap, response::{IntoResponse, Response}, Json};
use serde::{Deserialize, Serialize};
use linggen_engine::pictures;

use super::{providers::{owner_refusal, parse, save_config, Failure, BAD}, tts};
use crate::{
    config::{Config, ModelConfig, providers::Way}, credentials::{self, Credentials}, live,
    provider::models::{ModelManager, NotReady, providers_view::provider_source}, runtime, server::ServerState,
};

#[derive(Clone, Serialize)]
struct Source {
    way: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_gb: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    free: bool,
}

impl Source {
    fn new(way: Way, provider: Option<String>) -> Self {
        Self { way: way.as_str(), provider, size_gb: None, free: way == Way::Local }
    }

    fn chat(row: &ModelConfig) -> Self {
        let (provider, way) = provider_source(row.sign_in(), row.provider_name(), &row.url);
        Self::new(way, (way != Way::Subscription && !provider.is_empty()).then_some(provider))
    }
}

#[derive(Serialize)]
struct OptionRow {
    id: String,
    name: String,
    source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<NotReady>,
}

impl OptionRow {
    fn new(id: &str, name: &str, source: Source, state: &'static str) -> Self {
        Self { id: id.into(), name: name.into(), source, model_id: None, state, reason: None }
    }

    fn local(id: &str, model: &str, ready: bool, supported: bool) -> Self {
        let hub = runtime::HUB_MODELS.iter().find(|m| m.repo == model).expect("declared local model");
        let source = Source { size_gb: Some(hub.size_gb), ..Source::new(Way::Local, None) };
        let state = if ready { "ready" } else if supported { "download" } else { "unavailable" };
        OptionRow { model_id: Some(model.into()), ..Self::new(id, hub.name, source, state) }
    }
}

#[derive(Serialize)]
struct Use {
    choice: Option<String>,
    options: Vec<OptionRow>,
    effective: Option<String>,
}

impl Use {
    fn new(choice: Option<&str>, options: Vec<OptionRow>, effective: Option<&str>) -> Self {
        Self { choice: choice.map(Into::into), options, effective: effective.map(Into::into) }
    }

    fn accepts(&self, id: &str, states: &[&str]) -> bool {
        self.options.iter().any(|o| o.id == id && states.contains(&o.state))
    }
}

#[derive(Serialize)]
struct App {
    id: String,
    label: String,
    choice: Option<String>,
}

#[derive(Serialize)]
struct Table {
    chat: Use,
    chain: Vec<String>,
    model_fallback: bool,
    effort: Use,
    live: Use,
    speak: Use,
    voice: Use,
    draw: Use,
    hear: Use,
    apps: Vec<App>,
}

fn chat_row(models: &ModelManager, row: &ModelConfig) -> OptionRow {
    let reason = models.readiness(&row.id).and_then(|r| r.reason);
    let state = match reason {
        None => "ready",
        Some(NotReady::NoKey) => "no_key",
        Some(NotReady::NotSignedIn | NotReady::SignInExpired | NotReady::ProviderSignIn) => "signed_out",
        Some(_) => "unavailable",
    };
    let name = models.display_name(row);
    let name = if name == row.model { name.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&name) } else { &name };
    OptionRow {
        reason, model_id: (name != row.model).then(|| row.model.clone()),
        ..OptionRow::new(&row.id, name, Source::chat(row), state)
    }
}

async fn table(state: &ServerState, config: &Config) -> Table {
    let models = state.manager.models.read().await.clone();
    let chain = config.routing.default_models.clone();
    let first = chain.first().map(String::as_str);
    let mut chat: Vec<_> = models.list_models().into_iter().map(|row| chat_row(&models, row)).collect();
    chat.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let effort = chat.iter().find(|row| Some(row.id.as_str()) == first)
        .map(|row| models.effort_choices(&row.id).into_iter().map(|id| OptionRow::new(id, id, row.source.clone(), "ready")).collect())
        .unwrap_or_default();
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    let mut preferred = None;
    let services = live::readiness(config.uses.live.as_deref(), &config.models, &creds).map(|(p, ready, first)| {
        if first { preferred = Some(p.id); }
        let provider = crate::config::providers::label_for(p.id, &format!("https://{}", p.key_host()));
        OptionRow::new(p.id, p.model, Source::new(Way::Key, Some(provider)), if ready { "ready" } else { "no_key" })
    }).collect();
    let speech = tts::Source::readiness();
    let speak = tts::Source::ALL.into_iter().zip(speech).filter(|(s, _)| s.exists()).map(|(source, ready)| {
        match source {
            tts::Source::Qwen => OptionRow::local(source.id(), runtime::TTS_MODEL, ready, runtime::lane_verdict(&runtime::VOICE_LANE).is_ok()),
            tts::Source::Kokoro => {
                OptionRow::new(source.id(), "Kokoro", Source::new(Way::Local, None), if ready { "ready" } else { "download" })
            }
            tts::Source::System => OptionRow::new(source.id(), "macOS system voices", Source::new(Way::Local, None), if ready { "ready" } else { "unavailable" }),
        }
    }).collect();
    let signed_in = crate::provider::codex_auth::chatgpt_signed_in();
    let lane = runtime::lane_verdict(&runtime::PICTURES_LANE);
    let drawing = vec![
        // ChatGPT picks the image generator; the Responses model is only its host.
        OptionRow::new("chatgpt", "ChatGPT drawing", Source::new(Way::Subscription, None), if signed_in { "ready" } else { "signed_out" }),
        OptionRow::local("local", runtime::PICTURE_MODEL, lane.is_ok() && runtime::pictures_ready(), lane.is_ok()),
    ];
    let drawer = pictures::Source::select(config.uses.draw.as_deref(), signed_in, lane).ok().map(|source| source.id());
    let mut apps: Vec<_> = state.skills.list_skills().await.into_iter().filter_map(|skill| {
        let app = skill.text_model_app()?;
        Some(App { label: app.title.clone().unwrap_or_else(|| skill.name.clone()), choice: config.uses.apps.get(&skill.name).cloned(), id: skill.name })
    }).collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    Table {
        chat: Use::new(first, chat, first), chain,
        model_fallback: config.routing.model_fallback,
        effort: Use::new(config.uses.chat_effort.as_deref(), effort, None),
        live: Use::new(config.uses.live.as_deref(), services, preferred),
        speak: Use::new(config.uses.speak.as_deref(), speak, tts::default_provider(config.uses.speak.as_deref(), speech).map(|s| s.id())),
        voice: Use::new(Some(&config.pet.voice), super::tts_mlx::SPEAKERS.iter().map(|id| OptionRow::new(id, id, Source::new(Way::Local, None), "ready")).collect(), None),
        draw: Use::new(config.uses.draw.as_deref(), drawing, drawer),
        hear: Use::new(None, vec![OptionRow::local("local-stt", runtime::STT_MODEL, runtime::hearing_ready(), runtime::lane_verdict(&runtime::VOICE_LANE).is_ok())], Some("local-stt")),
        apps,
    }
}

#[derive(Deserialize)]
#[serde(tag = "use", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    Chat { choice: String },
    ChatFallbacks { choices: Vec<String> },
    ModelFallback { enabled: bool },
    Effort { choice: String },
    Live { choice: Option<String> },
    Speak { choice: Option<String> },
    Voice { choice: String },
    Draw { choice: Option<String> },
    App { id: String, choice: Option<String> },
}

fn set_chat_fallbacks(view: &Table, config: &mut Config, choices: Vec<String>) -> Result<(), Failure> {
    let Some(default) = view.chat.choice.as_ref() else { return Err(BAD) };
    let existing = &view.chain[1..];
    let mut seen = std::collections::HashSet::from([default]);
    for id in &choices {
        if !seen.insert(id) || (!existing.contains(id) && !view.chat.accepts(id, &["ready"])) {
            return Err(BAD);
        }
    }
    config.routing.default_models = std::iter::once(default.clone()).chain(choices).collect();
    Ok(())
}

impl Edit {
    fn apply(self, view: &Table, config: &mut Config) -> Result<(), Failure> {
        match self {
            Self::Chat { choice } if view.chat.accepts(&choice, &["ready"]) => config.make_default(&choice),
            Self::ChatFallbacks { choices } => set_chat_fallbacks(view, config, choices)?,
            Self::ModelFallback { enabled } => config.routing.model_fallback = enabled,
            Self::Effort { choice } if choice == "auto" || view.effort.accepts(&choice, &["ready"]) => config.uses.chat_effort = Some(choice),
            Self::Live { choice } if choice.as_ref().is_none_or(|id| view.live.accepts(id, &["ready"])) => config.uses.live = choice,
            Self::Speak { choice } if choice.as_ref().is_none_or(|id| view.speak.accepts(id, &["ready", "download"])) => config.uses.speak = choice,
            Self::Voice { choice } if view.voice.accepts(&choice, &["ready"]) => config.pet.voice = choice,
            Self::Draw { choice } if choice.as_ref().is_none_or(|id| view.draw.accepts(id, &["ready", "download"])) => config.uses.draw = choice,
            Self::App { id, choice } if view.apps.iter().any(|app| app.id == id)
                && choice.as_ref().is_none_or(|model| view.chat.accepts(model, &["ready"])) => {
                match choice {
                    Some(model) => { config.uses.apps.insert(id, model); }
                    None => { config.uses.apps.remove(&id); }
                }
            }
            _ => return Err(BAD),
        }
        Ok(())
    }
}

pub(crate) async fn list(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    let _write = state.settings_write.lock().await;
    let config = state.manager.get_config_snapshot().await;
    Json(table(&state, &config).await).into_response()
}

pub(crate) async fn put(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    async {
        let edit = parse::<Edit>(&body)?;
        let _write = state.settings_write.lock().await;
        let mut config = state.manager.get_config_snapshot().await;
        edit.apply(&table(&state, &config).await, &mut config)?;
        save_config(&state, &config).await?;
        let config = state.manager.get_config_snapshot().await;
        Ok::<_, Failure>(Json(table(&state, &config).await))
    }.await.into_response()
}
