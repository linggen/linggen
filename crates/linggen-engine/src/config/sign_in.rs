//! How a model signs in and what its provider makes the engine do — the one
//! place a model's `auth_mode` and `provider` strings are read.
//!
//! The provider's *name* is an open set (`KNOWN_PROVIDERS`, plus any
//! OpenAI-compatible host); only a few names change what the engine does,
//! and those are [`ProviderKind`]. The sign-in is a closed set,
//! [`AuthMode`], and what the two say together is [`SignIn`]. Every other
//! module matches on these enums; a lock (`server/model_locks.rs`) fails
//! when code outside this file compares the raw strings again.

use super::ModelConfig;
use serde::{Deserialize, Serialize};

/// A model's `auth_mode`, parsed once when the config is read. A string the
/// engine does not know is kept as written, so a config file reads and saves
/// exactly as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum AuthMode {
    /// `"api_key"`: the key itself — the same as no auth mode.
    ApiKey,
    /// `"chatgpt_oauth"`: the ChatGPT subscription's sign-in.
    ChatgptOauth,
    /// `"claude_oauth"`: Claude Code's sign-in (a Claude subscription).
    ClaudeOauth,
    /// `"linggen_account"`: Linggen Cloud, on the linggen.dev account.
    LinggenAccount,
    /// Any other string, kept as written.
    Other(String),
}

impl From<String> for AuthMode {
    fn from(s: String) -> AuthMode {
        match s.as_str() {
            "api_key" => AuthMode::ApiKey,
            "chatgpt_oauth" => AuthMode::ChatgptOauth,
            "claude_oauth" => AuthMode::ClaudeOauth,
            "linggen_account" => AuthMode::LinggenAccount,
            _ => AuthMode::Other(s),
        }
    }
}

impl From<&str> for AuthMode {
    fn from(s: &str) -> AuthMode {
        AuthMode::from(s.to_string())
    }
}

impl From<AuthMode> for String {
    fn from(m: AuthMode) -> String {
        m.as_str().to_string()
    }
}

impl AuthMode {
    /// The string the config file holds.
    pub fn as_str(&self) -> &str {
        match self {
            AuthMode::ApiKey => "api_key",
            AuthMode::ChatgptOauth => "chatgpt_oauth",
            AuthMode::ClaudeOauth => "claude_oauth",
            AuthMode::LinggenAccount => "linggen_account",
            AuthMode::Other(s) => s,
        }
    }
}

/// The provider names that change what the engine does; every other name
/// speaks the OpenAI-compatible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    /// The ChatGPT subscription's backend.
    ChatGpt,
    Ollama,
    /// A room's shared model, reached through its owner.
    Proxy,
    OpenAiCompatible,
}

impl ProviderKind {
    fn of(name: &str) -> ProviderKind {
        match name {
            "anthropic" => ProviderKind::Anthropic,
            "chatgpt" => ProviderKind::ChatGpt,
            "ollama" => ProviderKind::Ollama,
            "proxy" => ProviderKind::Proxy,
            _ => ProviderKind::OpenAiCompatible,
        }
    }
}

/// How a model's calls are signed in: what its auth mode and provider say
/// together. The one answer for the client the engine builds, the
/// pre-flight, readiness, billing, `ling status`, the phone's pairing list
/// and what the UI is told (`signs_in_with_chatgpt` on a model row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// A key (or none: a local server, a room's proxy).
    Key,
    /// The ChatGPT sign-in: `auth_mode = "chatgpt_oauth"`, or a `chatgpt`
    /// provider whatever else its auth mode says (but Linggen Cloud's).
    ChatGpt,
    /// Claude Code's sign-in.
    Claude,
    /// Linggen Cloud's account.
    LinggenAccount,
    /// An auth mode the engine does not know: no key is asked for, and no
    /// sign-in is checked.
    Unknown,
}

impl ModelConfig {
    /// The provider's name as configured (`deepseek`, `my-gateway`, …).
    pub fn provider_name(&self) -> &str {
        &self.provider
    }

    pub fn provider_kind(&self) -> ProviderKind {
        ProviderKind::of(&self.provider)
    }

    pub fn sign_in(&self) -> SignIn {
        match &self.auth_mode {
            Some(AuthMode::LinggenAccount) => SignIn::LinggenAccount,
            Some(AuthMode::ChatgptOauth) => SignIn::ChatGpt,
            _ if self.provider_kind() == ProviderKind::ChatGpt => SignIn::ChatGpt,
            Some(AuthMode::ClaudeOauth) => SignIn::Claude,
            Some(AuthMode::Other(_)) => SignIn::Unknown,
            Some(AuthMode::ApiKey) | None => SignIn::Key,
        }
    }

    pub fn signs_in_with_chatgpt(&self) -> bool {
        self.sign_in() == SignIn::ChatGpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, auth: Option<&str>) -> ModelConfig {
        let mut v = serde_json::json!({
            "id": "m", "provider": provider, "url": "", "model": "m",
        });
        if let Some(a) = auth {
            v["auth_mode"] = a.into();
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn auth_mode_reads_and_writes_every_old_form_as_it_was() {
        for (text, mode) in [
            ("api_key", AuthMode::ApiKey),
            ("chatgpt_oauth", AuthMode::ChatgptOauth),
            ("claude_oauth", AuthMode::ClaudeOauth),
            ("linggen_account", AuthMode::LinggenAccount),
            ("someday_oauth", AuthMode::Other("someday_oauth".into())),
        ] {
            let toml_in = format!(
                "id = \"m\"\nprovider = \"openai\"\nurl = \"\"\nmodel = \"m\"\nauth_mode = \"{text}\"\n"
            );
            let cfg: ModelConfig = toml::from_str(&toml_in).unwrap();
            assert_eq!(cfg.auth_mode, Some(mode));
            let out = toml::to_string(&cfg).unwrap();
            assert!(out.contains(&format!("auth_mode = \"{text}\"")), "{out}");
            let json = serde_json::to_value(&cfg).unwrap();
            assert_eq!(json["auth_mode"], text);
        }
        // Absent stays absent; null reads as absent.
        let cfg: ModelConfig =
            toml::from_str("id = \"m\"\nprovider = \"openai\"\nurl = \"\"\nmodel = \"m\"\n")
                .unwrap();
        assert_eq!(cfg.auth_mode, None);
        assert_eq!(model("openai", None).auth_mode, None);
        let null: ModelConfig = serde_json::from_value(serde_json::json!({
            "id": "m", "provider": "openai", "url": "", "model": "m", "auth_mode": null,
        }))
        .unwrap();
        assert_eq!(null.auth_mode, None);
    }

    #[test]
    fn a_chatgpt_sign_in_is_the_mode_or_the_provider_but_never_linggen_cloud() {
        assert_eq!(
            model("openai", Some("chatgpt_oauth")).sign_in(),
            SignIn::ChatGpt
        );
        assert_eq!(model("chatgpt", None).sign_in(), SignIn::ChatGpt);
        assert_eq!(model("chatgpt", Some("api_key")).sign_in(), SignIn::ChatGpt);
        assert_eq!(
            model("chatgpt", Some("linggen_account")).sign_in(),
            SignIn::LinggenAccount
        );
        assert_eq!(
            model("anthropic", Some("claude_oauth")).sign_in(),
            SignIn::Claude
        );
        assert_eq!(model("anthropic", Some("api_key")).sign_in(), SignIn::Key);
        assert_eq!(model("openai", None).sign_in(), SignIn::Key);
        assert_eq!(model("openai", Some("someday")).sign_in(), SignIn::Unknown);
    }

    #[test]
    fn provider_kind_names_the_few_providers_the_engine_treats_apart() {
        assert_eq!(
            model("anthropic", None).provider_kind(),
            ProviderKind::Anthropic
        );
        assert_eq!(
            model("chatgpt", None).provider_kind(),
            ProviderKind::ChatGpt
        );
        assert_eq!(model("ollama", None).provider_kind(), ProviderKind::Ollama);
        assert_eq!(model("proxy", None).provider_kind(), ProviderKind::Proxy);
        for other in ["openai", "deepseek", "gemini", "my-gateway"] {
            assert_eq!(
                model(other, None).provider_kind(),
                ProviderKind::OpenAiCompatible
            );
        }
    }
}
