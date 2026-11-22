//! May this caller run this app's action? — asked here, by every way in:
//! a model's call in a session, the page's door (a page's own call, a
//! tap on an offer, a device's call through the door), the companion's
//! `AppTool`, and an agent outside Linggen at the MCP door. One function, [`judge`], and one answer,
//! [`Verdict`]: run it, ask the person first, or refuse with the reason.
//!
//! What it is handed is what each way in knows: who asks ([`Asker`]), and
//! the action ([`Action`]: its app, name, class and working folder). The
//! level the action needs of a grant and of a caller is the class's alone
//! (`ActionClass::level_needed`, read nowhere else: lock
//! `structure_lock_tests::an_app_actions_level_is_judged_in_one_place`).
//!
//! What is checked before it stays where it was: the person's own right to
//! use the app (`People::may_use_app`, at the seam), the route's level, the
//! turn's allowlists and withheld tools, the sign-in check. The judgment
//! is asked after them, never instead.

use super::reach::Offer;
use super::tools::Refusal;
use crate::engine::permission::{ask_destroys, ActionClass, PermissionCheckResult, PermissionMode, PromptKind};
use std::path::Path;

/// An app's action as it is asked about.
pub struct Action<'a> {
    pub app: &'a str,
    pub tool: &'a str,
    pub class: ActionClass,
    /// Where it runs: the folder its grants are read on.
    pub cwd: &'a Path,
}

/// Who asks to run it.
pub enum Asker<'a> {
    /// A session's model. `session` is the permission gate's answer for
    /// the session's own grants at a level (its grants, a turn's ceiling,
    /// the rules of the person's Claude Code settings); the judgment
    /// adds what the action's class asks of any model.
    Model {
        session: &'a dyn Fn(PermissionMode) -> PermissionCheckResult,
    },
    /// The person's own act — a page's call, a tap on an offer, a device's
    /// call through the page's door — standing at `level`, for an app whose
    /// own grant on its folder is `granted`.
    Person {
        level: PermissionMode,
        granted: PermissionMode,
    },
    /// The companion through `AppTool`: what the app offers her (`offer`).
    /// A read; a change only in a turn the person started by asking her
    /// (`person_asked`) and only of what is offered anywhere; a destructive
    /// action never — the person does that themselves, in the app.
    Companion { offer: Offer, person_asked: bool },
    /// An agent outside Linggen — Claude Code, Codex — through the MCP
    /// door's `app` tool, for the owner at this Mac: what the app offers
    /// it (`offer`), for an app whose own grant on its folder is `granted`.
    /// It reads and changes under that grant; a destructive action never —
    /// it has no way to put the question to the person here — and nothing
    /// above what a change needs.
    OutsideHost { offer: Offer, granted: PermissionMode },
}

/// The most an outside host may do in an app: change what the person
/// keeps in it, as `edit` does.
const OUTSIDE_HOST_LEVEL: PermissionMode = PermissionMode::Edit;

/// The answer.
pub enum Verdict {
    Allow,
    /// Ask the person first; the gate's question carries the rest.
    Ask(PromptKind),
    Refuse(Refusal),
}

impl Verdict {
    /// For a way in that cannot ask anyone: an ask is a refusal.
    pub fn allowed_or_refusal(self, action: &Action<'_>) -> Result<(), Refusal> {
        match self {
            Verdict::Allow => Ok(()),
            Verdict::Refuse(refusal) => Err(refusal),
            Verdict::Ask(_) => Err(Refusal::NeedsPerson(format!(
                "'{}' of '{}' must be asked of the person, and nobody can be asked here",
                action.tool, action.app
            ))),
        }
    }

    pub fn is_allow(&self) -> bool {
        matches!(self, Verdict::Allow)
    }
}

/// A model's verdict is the gate's, as the gate states it.
impl From<Verdict> for PermissionCheckResult {
    fn from(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Allow => PermissionCheckResult::Allowed,
            Verdict::Ask(kind) => PermissionCheckResult::NeedsPrompt(kind),
            Verdict::Refuse(refusal) => PermissionCheckResult::Blocked(refusal.to_string()),
        }
    }
}

/// May `asker` run `action`?
pub fn judge(asker: Asker<'_>, action: &Action<'_>) -> Verdict {
    let needs = action.class.level_needed();
    match asker {
        Asker::Model { session } => {
            let mut answer = session(needs);
            if action.class.destroys() {
                // Whatever was granted, a model is asked of the person.
                answer = ask_destroys(answer, action.tool, action.cwd);
            }
            match answer {
                PermissionCheckResult::Allowed => Verdict::Allow,
                PermissionCheckResult::NeedsPrompt(kind) => Verdict::Ask(kind),
                PermissionCheckResult::Blocked(reason) => Verdict::Refuse(Refusal::ByGate(reason)),
            }
        }
        Asker::Person { level, granted } => within(action, needs, level, granted),
        Asker::Companion { offer, person_asked } => companion(offer, person_asked, action),
        Asker::OutsideHost { offer, granted } => outside_host(offer, granted, action),
    }
}

/// An action the app does not offer to the one asking, outside the app's
/// own session.
pub fn not_offered(app: &str, tool: &str) -> Refusal {
    Refusal::NotForPet(format!("'{app}' does not offer '{tool}' to you"))
}

/// What the companion may run: the offer first, then the class.
fn companion(offer: Offer, person_asked: bool, action: &Action<'_>) -> Verdict {
    let refuse = |why: String| Verdict::Refuse(Refusal::NotForPet(why));
    let (app, tool) = (action.app, action.tool);
    if offer == Offer::Nobody {
        return Verdict::Refuse(not_offered(app, tool));
    }
    if !action.class.changes_things() {
        return Verdict::Allow;
    }
    if offer == Offer::HerRead {
        return refuse(format!("'{tool}' may change '{app}'; you may only read"));
    }
    if action.class.destroys() {
        return refuse(format!(
            "'{tool}' of '{app}' destroys what it acts on: the person does this themselves, in the app"
        ));
    }
    if action.class.level_needed() > PermissionMode::Edit {
        return refuse(format!("'{tool}' may change '{app}' beyond what you may; you may only read"));
    }
    match person_asked {
        true => Verdict::Allow,
        false => refuse(format!(
            "'{tool}' changes '{app}', and this turn is yours, not something the person asked for; do it only when they ask"
        )),
    }
}

/// What an outside host may run: the offer first, then the class — a
/// destructive action is the person's own to do, in the app — then the
/// grant and the level an outside host stands at.
fn outside_host(offer: Offer, granted: PermissionMode, action: &Action<'_>) -> Verdict {
    if offer != Offer::Anywhere {
        return Verdict::Refuse(not_offered(action.app, action.tool));
    }
    if action.class.destroys() {
        return Verdict::Refuse(Refusal::NotForHost(format!(
            "'{}' of '{}' destroys what it acts on: the person does this themselves, in the app",
            action.tool, action.app
        )));
    }
    within(action, action.class.level_needed(), OUTSIDE_HOST_LEVEL, granted)
}

/// The action's level within what the app's own grant covers on its
/// folder, then within the level its caller stands at.
fn within(action: &Action<'_>, needs: PermissionMode, level: PermissionMode, granted: PermissionMode) -> Verdict {
    if needs > granted {
        return Verdict::Refuse(Refusal::BeyondGrant(format!(
            "'{}' needs {needs} on {}; the skill grants {granted}",
            action.tool,
            action.cwd.display()
        )));
    }
    if needs > level {
        return Verdict::Refuse(Refusal::BeyondLevel { level, needs });
    }
    Verdict::Allow
}
