//! What can be done about a problem — the offers a raiser hands the resident
//! beside the facts (`doc/perception-spec.md` §5).
//!
//! A problem is a record, never a sentence: its key, its facts, its level, and
//! what the person could have done about it with one tap — or, when nothing
//! can be done from here, the reason. [`Remedy`] holds exactly one of the two;
//! it cannot be built empty, so a raiser with neither cannot register.
//!
//! The engine names no app. What a Mac problem can be offered is declared by
//! the skills (`offers:` in their frontmatter, keyed by the problem), mirrored
//! here by the skill loader so a glance reads them without waiting on it.

use crate::engine::permission::ActionClass;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// One thing the person may tap. Run as a user button — never by an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Offer {
    /// A skill's declared tool, run through the skill-tool route.
    Tool {
        skill: String,
        tool: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        args: serde_json::Value,
        /// The tool's class: anything but a read asks the person before it
        /// runs.
        tier: ActionClass,
    },
    /// A skill's page, at one of its views when it names one.
    Open {
        skill: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
}

impl Offer {
    /// The offer as a fact she reads — what it does, never what it is called.
    fn fact(&self) -> String {
        match self {
            Offer::Tool { skill, tool, tier, .. } => {
                let tier = if tier.changes_things() {
                    "changes things, asks the person first"
                } else {
                    "only reads"
                };
                format!("run {skill}'s {tool} ({tier})")
            }
            Offer::Open { skill, view: Some(v) } => format!("open the {skill} page at {v}"),
            Offer::Open { skill, view: None } => format!("open the {skill} page"),
        }
    }
}

/// An offer as the person sees it: her label on it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shown {
    pub label: String,
    pub offer: Offer,
}

/// The offers, or why there are none. Never neither.
#[derive(Debug, Clone, PartialEq)]
pub struct Remedy {
    offers: Vec<Offer>,
    none: Option<&'static str>,
}

impl Remedy {
    /// `offers`, or — when there are none — `why_none`, a fact she words.
    pub fn of(offers: Vec<Offer>, why_none: &'static str) -> Self {
        let none = offers.is_empty().then_some(why_none);
        Self { offers, none }
    }

    pub fn offers(&self) -> &[Offer] {
        &self.offers
    }

    pub fn none(&self) -> Option<&'static str> {
        self.none
    }

    /// The remedy as she is shown it: numbered offers, or the reason there
    /// are none. Facts only — her words are hers.
    pub fn facts(&self) -> String {
        if let Some(why) = self.none {
            return format!("nothing can be offered from here: {why}");
        }
        let list: Vec<String> = self
            .offers
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{}. {}", i + 1, o.fact()))
            .collect();
        format!("what can be offered, each a button the person may tap: {}", list.join("; "))
    }
}

/// One `offers:` entry as a skill wrote it: exactly one of `tool` (one of its
/// declared tools) or `open` (a view of its page; empty for the page itself).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decl {
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    open: Option<String>,
    #[serde(default)]
    args: Option<serde_json::Value>,
}

/// An offer a skill declared for a problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Declared {
    pub problem: String,
    pub offer: Offer,
}

/// Problems and offers one skill may declare, bounded.
const PROBLEMS_MAX: usize = 8;
const OFFERS_MAX: usize = 3;
const KEY_MAX: usize = 32;

/// What the skill declares, checked where its frontmatter is read. A bad entry
/// is logged and left out; the skill still loads. `tools`: the skill's declared
/// tools and their tiers; `has_page`: whether it has an app to open.
pub fn usable(
    raw: Option<serde_json::Value>,
    skill: &str,
    tools: &[(String, ActionClass)],
    has_page: bool,
) -> Vec<Declared> {
    let Some(serde_json::Value::Object(map)) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (problem, decls) in map.into_iter().take(PROBLEMS_MAX) {
        if problem.is_empty() || problem.len() > KEY_MAX {
            tracing::warn!("skill '{skill}': offers for '{problem}': not a problem key; ignored");
            continue;
        }
        let decls: Vec<Decl> = match serde_json::from_value(decls) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("skill '{skill}': offers for '{problem}': {e}; ignored");
                continue;
            }
        };
        for d in decls.into_iter().take(OFFERS_MAX) {
            match resolve(d, skill, tools, has_page) {
                Ok(offer) => out.push(Declared { problem: problem.clone(), offer }),
                Err(why) => tracing::warn!("skill '{skill}': offer for '{problem}': {why}; ignored"),
            }
        }
    }
    out
}

fn resolve(d: Decl, skill: &str, tools: &[(String, ActionClass)], has_page: bool) -> Result<Offer, String> {
    match (d.tool, d.open) {
        (Some(tool), None) => {
            let (_, tier) = tools
                .iter()
                .find(|(name, _)| *name == tool)
                .ok_or_else(|| format!("'{tool}' is not one of its tools"))?;
            let args = d.args.unwrap_or(serde_json::Value::Null);
            if !(args.is_null() || args.is_object()) {
                return Err("args must be a map".into());
            }
            Ok(Offer::Tool {
                skill: skill.to_string(),
                tool,
                args,
                tier: *tier,
            })
        }
        (None, Some(view)) if has_page => Ok(Offer::Open {
            skill: skill.to_string(),
            view: Some(view).filter(|v| !v.is_empty()),
        }),
        (None, Some(_)) => Err("it has no page to open".into()),
        _ => Err("name exactly one of tool or open".into()),
    }
}

static DECLARED: Mutex<Vec<Declared>> = Mutex::new(Vec::new());

/// The skills' offers, as the loader last saw them.
pub fn declare(offers: Vec<Declared>) {
    *DECLARED.lock().unwrap_or_else(|e| e.into_inner()) = offers;
}

#[cfg(test)]
thread_local! {
    static TEST_DECLARED: std::cell::RefCell<Option<Vec<Declared>>> = const { std::cell::RefCell::new(None) };
}

/// Make this thread read `offers` instead of the loader's list.
#[cfg(test)]
pub(crate) fn declare_for_this_thread(offers: Vec<Declared>) {
    TEST_DECLARED.with(|t| *t.borrow_mut() = Some(offers));
}

/// What the installed skills offer for `problem`, in their order.
pub fn declared_for(problem: &str) -> Vec<Offer> {
    #[cfg(test)]
    if let Some(d) = TEST_DECLARED.with(|t| t.borrow().clone()) {
        return pick(&d, problem);
    }
    pick(&DECLARED.lock().unwrap_or_else(|e| e.into_inner()), problem)
}

fn pick(all: &[Declared], problem: &str) -> Vec<Offer> {
    all.iter()
        .filter(|d| d.problem == problem)
        .map(|d| d.offer.clone())
        .take(OFFERS_MAX)
        .collect()
}

#[cfg(test)]
mod tests;
