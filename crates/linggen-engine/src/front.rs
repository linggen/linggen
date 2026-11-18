//! The foreground page reading shared by presence and perception.

/// The page in front of the person: the focused live surface with the
/// freshest input. `app` is the skill the page names; `None` is Linggen's own
/// main page (or a page too old to name itself).
#[derive(Debug, Clone, PartialEq)]
pub struct Front {
    pub app: Option<String>,
    pub last_input_at: u64,
}
