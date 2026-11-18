//! What the engine knows before it has read anything: the compiled values.
//! An engine that has never been online offers exactly these, and the
//! version presented never goes below [`CLIENT_VERSION`].

/// The Codex client version compiled in — the floor of what is presented to
/// the ChatGPT backend, which filters its model list by it.
pub(super) const CLIENT_VERSION: &str = "0.169.0";

/// The built-in families and the model each starts on. Sol 6.2 (everyday
/// work, paid plans), Terra 6.1 (balanced) and Luna (fast, every plan).
pub(super) const FAMILIES: &[(&str, &str)] = &[
    ("sol", "gpt-6.2-sol"),
    ("terra", "gpt-6.1-terra"),
    ("luna", "gpt-6-luna"),
];

/// The family whose current model is the primary built-in: the default
/// routing target, the picture model, and where an old id with no family
/// of its own goes.
pub(super) const PRIMARY_FAMILY: &str = "luna";

/// Ids earlier releases used, each with the family it belongs to now.
pub(super) const RETIRED: &[(&str, &str)] = &[
    ("gpt-5.5", PRIMARY_FAMILY),
    ("gpt-5.6-sol", "sol"),
    ("gpt-5.6-terra", "terra"),
    ("gpt-5.6-luna", "luna"),
    ("gpt-6-sol", "sol"),
    ("gpt-6.1-sol", "sol"),
];
