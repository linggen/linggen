//! The one switch for rooms (`doc/room-spec.md`).
//!
//! Rooms are off unless `room_config.toml` says `rooms_on = true`; a machine
//! that never set it has them off, whatever else the file holds. The room
//! code is all still here — the switch only decides whether any door into it
//! opens. It is read once per process, so turning it on or off takes a
//! restart and no peer outlives a change.
//!
//! Every way in asks [`on`] (or is handed its answer): the doors are listed
//! in [`DOORS`], and a test fails when a file outside that list asks.

use std::sync::OnceLock;

/// The one plain fact every way into rooms answers while the switch is off.
pub(crate) const OFF: &str = "rooms are off";

/// Whether rooms are on in this process.
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(super::rtc::room_config::rooms_switch_from_disk)
}

/// The files that ask [`on`], each one a way into rooms:
/// - `routes/mod.rs` — hands the answer to the room HTTP routes (`routes/rooms.rs`)
/// - `rtc/relay.rs` — a room consumer's offer
/// - `rtc/proxy_room.rs` — this machine joining another's room
/// - `background.rs` — the start-up join of the rooms this account is in
/// - `ui_events/mod.rs` — room chat on its way to a page
/// - `rtc/page_state.rs` — what the owner's page is told about rooms
#[cfg(test)]
const DOORS: &[&str] = &[
    "background.rs",
    "routes/mod.rs",
    "rtc/page_state.rs",
    "rtc/proxy_room.rs",
    "rtc/relay.rs",
    "ui_events/mod.rs",
];

#[cfg(test)]
mod tests {
    use super::DOORS;

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Only the doors ask the switch, and only the switch reads its key: a
    /// new way in must be added here, deliberately.
    #[test]
    fn only_the_listed_doors_ask_the_switch() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let src = root.join("src/server");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut askers = Vec::new();
        for file in files {
            let rel = file.strip_prefix(&src).unwrap().to_string_lossy().to_string();
            if rel == "rooms_switch.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            // The tests of this very check name the call; skip test modules.
            let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
            assert!(
                !code.contains("rooms_switch_from_disk") || rel == "rtc/room_config.rs",
                "{rel}: the switch's key is read by rooms_switch::on only"
            );
            if code.contains("rooms_switch::on()") {
                askers.push(rel);
            }
        }
        askers.sort();
        assert_eq!(askers, DOORS, "the doors that ask the switch changed");
    }
}
