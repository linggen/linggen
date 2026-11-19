//! The person's presence, read from every surface that reports it.
//!
//! Every page that shows Linggen beats `POST /api/presence` on a 4s cadence —
//! the main UI in each tab, every skill page, the app shell — and a turn typed
//! anywhere (the phone included) stamps one too. Each surface keeps its own
//! reading here; the person's state is the most present of the live ones. One
//! shared slot made it last-writer-wins: a focused-but-idle tab overwrote the
//! surface the person was working in, a run finishing a moment later read as
//! "away", and Yinyue heralded a reply to someone watching it arrive.

use crate::front::Front;
use std::collections::HashMap;

/// Live user-presence signal fed by the clients' throttled beat. Only recency,
/// focus, and a typing flag — never keystroke content. The `sense` tool reads it
/// so Yinyue can tell whether the user is here, reading, or away.
#[derive(Debug, Clone, Default)]
pub struct Presence {
    /// Unix secs of the user's last input (key/pointer). 0 = never reported.
    pub last_input_at: u64,
    /// Tab/window focused at the last beat.
    pub focused: bool,
    /// User was actively typing at the last beat.
    pub typing: bool,
    /// Unix secs of the last beat — a stale value means no live client.
    pub updated_at: u64,
    /// The app (skill) the focused surface shows, when its beat names one.
    /// None: a surface that says nothing about which app is in front.
    pub app: Option<String>,
}

/// How long a focused beat outranks an unfocused one sharing its slot (old
/// clients that send no surface id share one slot per app).
const FOCUS_HOLDS_SECS: u64 = 10;
/// A surface that has not beaten this long is not live: it reads as away.
const BEAT_LIVE_SECS: u64 = 60;
/// A surface silent this long is forgotten — the tab closed, the page left.
const SURFACE_EXPIRES_SECS: u64 = 300;
/// The most surfaces kept; the longest silent go first.
const MAX_SURFACES: usize = 32;
/// The surface a typed turn stamps (`AgentManager::mark_user_turn_presence`).
/// It says the person is here, not which page is in front of them.
pub const USER_TURN_SURFACE: &str = "user-turn";

impl Presence {
    /// Whether a beat saying "not focused" must be ignored: the same slot said
    /// it WAS focused a moment ago. Every reporter beats on a 4s cadence, so a
    /// focused reading older than [`FOCUS_HOLDS_SECS`] belongs to a surface
    /// that has gone rather than one still in front of the user.
    pub fn holds_focus(&self, now: u64) -> bool {
        self.focused && now.saturating_sub(self.updated_at) < FOCUS_HOLDS_SECS
    }

    /// Derive the three-state read at `now` (unix secs): `"typing"` /
    /// `"present_reading"` / `"away"`. The single source of this logic — both the
    /// `sense` tool and Yinyue's herald watch read through it.
    pub fn state(&self, now: u64) -> &'static str {
        let beat_age = now.saturating_sub(self.updated_at);
        let idle = now.saturating_sub(self.last_input_at);
        if self.updated_at == 0 || beat_age > BEAT_LIVE_SECS || !self.focused {
            "away" // no live client, or tab hidden/blurred
        } else if self.typing || idle < 5 {
            "typing"
        } else if idle < 120 {
            "present_reading"
        } else {
            "away" // focused tab, but long idle — stepped away
        }
    }

    /// How present this reading is: typing > reading > away, then focus,
    /// then freshest input, then freshest beat.
    fn rank(&self, now: u64) -> (u8, bool, u64, u64) {
        let state = match self.state(now) {
            "typing" => 2,
            "present_reading" => 1,
            _ => 0,
        };
        (state, self.focused, self.last_input_at, self.updated_at)
    }
}

/// One beat, as a surface sent it.
#[derive(Debug, Clone, Default)]
pub struct Beat {
    /// The surface's own id (one per page load). None from old clients.
    pub surface: Option<String>,
    pub focused: bool,
    pub typing: bool,
    /// Milliseconds since the user's last input, measured client-side.
    pub idle_ms: u64,
    pub app: Option<String>,
}

/// Which Linggen pages read present: Linggen's own, an app's.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PagesPresent {
    pub main: bool,
    pub app: bool,
}

/// Every surface's latest reading, keyed by surface.
#[derive(Debug, Default)]
pub struct PresenceBoard {
    surfaces: HashMap<String, Presence>,
}

impl PresenceBoard {
    /// Record one surface's beat at `now`.
    pub fn record(&mut self, beat: Beat, now: u64) {
        let (key, shared) = match beat.surface {
            Some(id) => (format!("s:{id}"), false),
            // An old client names no surface. It gets one slot per app, and
            // since several tabs may share it, a blurred beat never clears a
            // fresh focused one there; when that surface really goes, its
            // reading ages out.
            None => (
                format!("legacy:{}", beat.app.as_deref().unwrap_or("")),
                true,
            ),
        };
        let slot = self.surfaces.entry(key).or_default();
        if shared && !beat.focused && slot.holds_focus(now) {
            return;
        }
        *slot = Presence {
            last_input_at: now.saturating_sub(beat.idle_ms / 1000),
            focused: beat.focused,
            typing: beat.typing,
            updated_at: now,
            app: beat.app,
        };
        self.expire(now);
    }

    /// Forget surfaces silent past [`SURFACE_EXPIRES_SECS`], and keep at most
    /// [`MAX_SURFACES`].
    fn expire(&mut self, now: u64) {
        self.surfaces
            .retain(|_, p| now.saturating_sub(p.updated_at) <= SURFACE_EXPIRES_SECS);
        while self.surfaces.len() > MAX_SURFACES {
            let Some(oldest) = self
                .surfaces
                .iter()
                .min_by_key(|(_, p)| p.updated_at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.surfaces.remove(&oldest);
        }
    }

    /// The person's presence at `now`: the most present reading among all
    /// surfaces. Present if ANY live surface shows recent activity; away only
    /// when none does. Empty board: never reported.
    pub fn snapshot(&self, now: u64) -> Presence {
        self.surfaces
            .values()
            .max_by_key(|p| p.rank(now))
            .cloned()
            .unwrap_or_default()
    }

    /// Which page is in front at `now`, from the pages' own beats only — a
    /// typed turn (the phone, the CLI) says nothing about what is on screen.
    pub fn front(&self, now: u64) -> Option<Front> {
        let turn = format!("s:{USER_TURN_SURFACE}");
        self.surfaces
            .iter()
            .filter(|(key, _)| **key != turn)
            .map(|(_, p)| p)
            .filter(|p| p.focused && now.saturating_sub(p.updated_at) <= BEAT_LIVE_SECS)
            .max_by_key(|p| (p.last_input_at, p.updated_at))
            .map(|p| Front {
                app: p.app.clone(),
                last_input_at: p.last_input_at,
            })
    }

    /// Whether the person is at a page showing `app` at `now` (`None`:
    /// Linggen's main page): one of that page's own surfaces reads present, as
    /// [`Presence::state`] reads it. A typed turn names no page, so it never
    /// counts here.
    pub fn at_page(&self, app: Option<&str>, now: u64) -> bool {
        let turn = format!("s:{USER_TURN_SURFACE}");
        self.surfaces
            .iter()
            .filter(|(key, _)| **key != turn)
            .any(|(_, p)| p.app.as_deref() == app && p.state(now) != "away")
    }

    /// Which kind of Linggen page reads present at `now`: one of the pages' own
    /// surfaces (a typed turn names no page, so it never counts), with the
    /// freshness [`Presence::state`] uses. `main`: Linggen's own page; `app`:
    /// an app's page.
    pub fn pages_present(&self, now: u64) -> PagesPresent {
        let turn = format!("s:{USER_TURN_SURFACE}");
        let mut here = PagesPresent::default();
        for (_, p) in self.surfaces.iter().filter(|(key, _)| **key != turn) {
            if p.state(now) == "away" {
                continue;
            }
            match p.app {
                Some(_) => here.app = true,
                None => here.main = true,
            }
        }
        here
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.surfaces.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(surface: Option<&str>, focused: bool, typing: bool, idle_s: u64) -> Beat {
        Beat {
            surface: surface.map(str::to_string),
            focused,
            typing,
            idle_ms: idle_s * 1000,
            app: None,
        }
    }

    fn reading(focused: bool, typing: bool, idle: u64, at: u64) -> Presence {
        Presence {
            last_input_at: at.saturating_sub(idle),
            focused,
            typing,
            updated_at: at,
            app: None,
        }
    }

    #[test]
    fn presence_reads_typing_then_reading_then_away() {
        let now = 1_000;
        assert_eq!(reading(true, true, 0, now).state(now), "typing");
        assert_eq!(reading(true, false, 2, now).state(now), "typing");
        assert_eq!(reading(true, false, 30, now).state(now), "present_reading");
        assert_eq!(reading(true, false, 300, now).state(now), "away");
        assert_eq!(
            reading(false, false, 0, now).state(now),
            "away",
            "a hidden or blurred surface is not somewhere they are"
        );
        assert_eq!(
            reading(true, false, 0, now - 120).state(now),
            "away",
            "no beat for two minutes is no live surface at all"
        );
    }

    /// The bug: one slot, last writer wins — a focused-but-idle tab beating
    /// after the surface the person works in made them "away".
    #[test]
    fn an_active_surface_beside_an_idle_focused_one_is_present() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(beat(Some("phone-turn"), true, false, 1), now - 2);
        b.record(beat(Some("idle-tab"), true, false, 600), now);
        assert_eq!(b.snapshot(now).state(now), "typing");
        b.record(beat(Some("idle-tab"), true, false, 600), now + 30);
        assert_eq!(b.snapshot(now + 30).state(now + 30), "present_reading");
    }

    #[test]
    fn all_surfaces_idle_or_blurred_reads_away() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(beat(Some("a"), true, false, 600), now);
        b.record(beat(Some("b"), false, false, 0), now);
        assert_eq!(b.snapshot(now).state(now), "away");
        assert_eq!(PresenceBoard::default().snapshot(now).state(now), "away");
    }

    #[test]
    fn a_surface_that_stopped_beating_expires() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(beat(Some("gone"), true, true, 0), now);
        b.record(beat(Some("idle"), true, false, 600), now + 61);
        assert_eq!(
            b.snapshot(now + 61).state(now + 61),
            "away",
            "a surface silent past a minute no longer counts as present"
        );
        b.record(beat(Some("idle"), true, false, 600), now + 301);
        assert_eq!(b.len(), 1, "a long-silent surface is forgotten");
    }

    #[test]
    fn a_surface_own_blur_counts_at_once() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(beat(Some("a"), true, false, 0), now);
        b.record(beat(Some("a"), false, false, 0), now + 1);
        assert_eq!(b.snapshot(now + 1).state(now + 1), "away");
    }

    /// Old clients send no surface id: they still count, and a blurred one
    /// sharing their slot cannot clear a fresh focused reading.
    #[test]
    fn a_beat_without_a_surface_id_still_works() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(beat(None, true, false, 0), now);
        assert_eq!(b.snapshot(now).state(now), "typing");
        b.record(beat(None, false, false, 0), now + 4);
        assert_eq!(b.snapshot(now + 4).state(now + 4), "typing");
        b.record(beat(None, false, false, 0), now + 15);
        assert_eq!(b.snapshot(now + 15).state(now + 15), "away");
        // Beside a surface that names itself, the more present one wins.
        b.record(beat(Some("x"), true, false, 30), now + 15);
        assert_eq!(b.snapshot(now + 15).state(now + 15), "present_reading");
    }

    #[test]
    fn the_front_page_is_the_focused_live_one_with_the_freshest_input() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        assert_eq!(b.front(now), None, "nothing beats: nothing in front");
        let mut page = beat(Some("page"), true, false, 30);
        page.app = Some("tiles".into());
        b.record(page, now);
        b.record(beat(Some("main"), false, false, 0), now);
        assert_eq!(b.front(now).unwrap().app.as_deref(), Some("tiles"), "a blurred page is not in front");

        // A typed turn stamps presence but is no page.
        b.record(beat(Some(USER_TURN_SURFACE), true, true, 0), now);
        assert_eq!(b.front(now).unwrap().app.as_deref(), Some("tiles"));

        // The page stops beating: nothing is in front once it is not live.
        assert_eq!(b.front(now + BEAT_LIVE_SECS + 1), None);
    }

    fn page(surface: &str, app: Option<&str>, focused: bool, idle_s: u64) -> Beat {
        Beat {
            app: app.map(str::to_string),
            ..beat(Some(surface), focused, false, idle_s)
        }
    }

    /// A person reading the page a line shows on is at that page; the page
    /// hidden, left idle, or another app's page in front is not.
    #[test]
    fn the_person_is_at_a_page_only_while_its_own_surface_reads_present() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        assert!(!b.at_page(None, now), "nothing beats: at no page");

        b.record(page("main", None, true, 3), now);
        assert!(b.at_page(None, now), "the main page, focused, input a moment ago");
        assert!(!b.at_page(Some("dj"), now), "another app's page is not in front");

        b.record(page("main", None, false, 0), now + 1);
        assert!(!b.at_page(None, now + 1), "the main page hidden or blurred");

        b.record(page("dj", Some("dj"), true, 30), now + 1);
        assert!(b.at_page(Some("dj"), now + 1), "an app's page, being read");
        assert!(!b.at_page(None, now + 1));
        assert!(!b.at_page(Some("dj"), now + 200), "walked away from it");
    }

    #[test]
    fn a_typed_turn_is_at_no_page() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        b.record(page(USER_TURN_SURFACE, None, true, 0), now);
        b.record(page(USER_TURN_SURFACE, Some("dj"), true, 0), now);
        assert!(!b.at_page(None, now));
        assert!(!b.at_page(Some("dj"), now));
        assert_eq!(b.snapshot(now).state(now), "typing", "yet they are present");
    }

    /// Her line shows on any focused Linggen page — the main one or an app's —
    /// so either counts as "a page in front"; a typed turn, a blurred tab and a
    /// long quiet read do not.
    #[test]
    fn an_app_page_in_front_counts_as_a_page_that_shows_her_line() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        assert_eq!(b.pages_present(now), PagesPresent::default());
        b.record(page("dj", Some("dj"), true, 30), now);
        assert_eq!(b.pages_present(now), PagesPresent { main: false, app: true });
        b.record(page("main", None, true, 30), now);
        assert_eq!(b.pages_present(now), PagesPresent { main: true, app: true });
        // A typed turn names no page; a page long quiet reads away.
        let mut b = PresenceBoard::default();
        b.record(page(USER_TURN_SURFACE, None, true, 0), now);
        b.record(page("dj", Some("dj"), true, 30), now);
        assert_eq!(b.pages_present(now + 200), PagesPresent::default());
        assert_eq!(b.pages_present(now), PagesPresent { main: false, app: true });
    }

    #[test]
    fn the_present_surface_names_the_app() {
        let now = 1_000;
        let mut b = PresenceBoard::default();
        let mut dj = beat(Some("dj"), true, false, 1);
        dj.app = Some("dj".into());
        b.record(dj, now);
        let mut main = beat(Some("main"), false, false, 0);
        main.app = None;
        b.record(main, now);
        assert_eq!(b.snapshot(now).app.as_deref(), Some("dj"));
    }
}
