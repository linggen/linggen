//! Senses: facts of the person's real world that a skill may declare and the
//! engine reads for it — so a skill never goes online itself. A skill lists
//! the ones it wants in its frontmatter (`senses: [weather]`); each sense it
//! declares reaches its tool commands (the model's calls and its page's
//! alike) as env, and its page may also ask the sense's own route. The engine
//! names no app: any skill that declares a sense gets it. See
//! `doc/skill-spec.md` § Senses.

pub mod weather;

/// The person's own recent words in the session, as typed
/// (`LINGGEN_USER_WORDS`). Read from the session, so the tool's session env
/// carries it (`Tools::tool_env`) — only to a skill that declares it.
pub const USER_WORDS: &str = "user_words";

/// The senses the engine knows, by the name a skill declares.
pub const KNOWN: &[&str] = &["weather", USER_WORDS];

/// What a skill's declared senses put in its tool command's env. Never
/// waits: each sense hands over what it last knew and refreshes behind.
pub fn tool_env(senses: &[String]) -> Vec<(String, String)> {
    senses
        .iter()
        .filter_map(|s| match s.as_str() {
            "weather" => weather::tool_env(),
            // USER_WORDS comes with the session's env, not from here.
            _ => None,
        })
        .collect()
}
