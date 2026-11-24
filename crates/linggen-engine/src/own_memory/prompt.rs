//! What a Linggen session is told about its memory at start: how to use the
//! notes, and the index. The words are here and nowhere else; the engine
//! only asks which block a session gets (`engine::prompt`).

use super::fence::Audience;
use super::companion;
use super::index::{index, index_for_app};
use super::{Store, TOOL};
use std::path::Path;

/// A person's session (Ling, Yinyue): how to use memory, then the index.
pub fn owner_block(store: &Store, audience: Audience, cwd: &Path, now: i64) -> String {
    let got = index(store, audience, Some(cwd), now);
    let notes = match got.text.is_empty() {
        true => "No notes yet.\n".to_string(),
        false => got.text,
    };
    let days = match audience {
        Audience::Companion => companion::today(store, now),
        Audience::Work | Audience::Chronicler => String::new(),
    };
    format!("\n\n{}\n{notes}{days}", guidance(audience))
}

/// An app's session: the index alone, read-only — the app has no memory
/// tool. `person` says whether it knows who the person is.
pub fn app_block(store: &Store, cwd: Option<&Path>, person: bool, now: i64) -> Option<String> {
    let got = index_for_app(store, cwd, person, now);
    (!got.text.is_empty()).then_some(got.text)
}

fn guidance(audience: Audience) -> String {
    let moment = match audience {
        Audience::Companion => {
            " A life moment worth keeping (kind moment, scope moments): what happened, how the person felt, what they concluded."
        }
        Audience::Work | Audience::Chronicler => "",
    };
    let raise = match audience {
        Audience::Companion => {
            "\nA note marked \"don't raise\" you never bring up yourself; answer about it only if the person does (op=write action=no_raise sets or clears it). \
A birthday or anniversary: add the note, then op=write action=yearly day=MM-DD (YYYY-MM-DD when the first year is known). \
The chronicle (scope chronicle, found by op=search) holds month by month what the person did: read it when they ask about the past."
        }
        Audience::Work | Audience::Chronicler => "",
    };
    format!(
        "# Memory\n\
Linggen keeps notes about the person and the work. The index below lists them; trust a note unless the person just said otherwise, and say when you use one. \
Read a note in full with {TOOL} op=read id=…; find others with op=search words=… — a note not listed is only forgotten, not gone.\n\
Write with {TOOL} op=write only at these moments: the person says remember, forget or \"that's wrong\"; a decision or rule is stated (with its why); you find a gotcha.{moment} \
Never what was done or where work stands — git and the docs hold that. One fact per note; update the old note, do not add a second. \
A note in the person's own words (voice user) changes only after you show it and they agree: then pass user_directed true. A note that is wrong: action=void with the reason.{raise}"
    )
}
