//! The ids of the two built-in agents the store and the perception layer
//! name. A leaf module: they need the words, not the engine that runs the
//! agents. (`engine::agent` re-exports both.)

/// The resident companion agent — a built-in agent like `ling`, not a skill.
/// Everything the engine does specially for her (heralds, the presenter
/// registry, relaying another agent's prompt) keys on this one id.
pub const COMPANION_AGENT_ID: &str = "yinyue";
/// The agent an ordinary session seats, and the one that answers on the Mac
/// when a message names nobody (`doc/shared-session-spec.md` § Who answers).
pub const LEAD_AGENT_ID: &str = "ling";
/// The agent that writes the chronicle (`agents/chronicler.md`, run by the
/// built-in `chronicle` mission): it reads the person's whole record, and
/// `own_memory::fence` lets it write the chronicle alone.
pub const CHRONICLER_AGENT_ID: &str = "chronicler";
