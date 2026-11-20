use super::*;

#[test]
fn permission_notice_keeps_the_exact_option_labels() {
    use crate::engine::permission::{build_exceeds_ceiling_question, PermissionMode};

    let question = build_exceeds_ceiling_question(
        "Edit the notes",
        &PermissionMode::Edit,
        "/work/Project Notes/file.md",
        &["/work/Project Notes".into()],
    );
    let notice = ask_kickoff("ling", "ask-1", &[question]);
    assert!(notice.contains(
        "The options are: grant:at:edit:/work/Project Notes / grant:all:edit / grant:all:admin / Deny."
    ), "{notice}");
}

/// What reaches the builder's companion — a finished reply's topic, a
/// question — comes only from the builder's own sessions.
#[tokio::test]
async fn only_the_builders_sessions_reach_the_companion() {
    let empty = || std::sync::Arc::new(crate::engine::test_registries::Empty);
    let mut config = crate::config::Config::default();
    config.agent.daily_session = Some(true);
    let manager = crate::engine::agent::AgentManager::new(
        config,
        None,
        empty(),
        empty(),
        empty(),
        crate::engine::InterfaceMode::Web,
    )
    .0;
    let made_for = |id: &str, user: Option<&str>, creator: &str| {
        let mut meta = crate::state_fs::sessions::SessionMeta::default();
        meta.id = id.into();
        meta.user_id = user.map(str::to_string);
        meta.creator = creator.into();
        manager.global_sessions.add_session(&meta).unwrap();
    };
    made_for("sess-1790000201-4e4a1d01", None, "user");
    made_for("sess-1790000202-4e4a1d02", Some("u-friend"), "user");
    made_for("sess-1790000203-4e4a1d03", None, "external");
    assert!(whose(&manager, Some("sess-1790000201-4e4a1d01")).await);
    assert!(whose(&manager, None).await);
    assert!(!whose(&manager, Some("sess-1790000202-4e4a1d02")).await);
    // A run another tool started (`agent_run`): its caller has it.
    assert!(!whose(&manager, Some("sess-1790000203-4e4a1d03")).await);
}

#[test]
fn only_external_sessions_are_delegated() {
    use crate::state_fs::sessions::SessionMeta;
    for creator in ["user", "agent", "skill", "mission"] {
        let meta = SessionMeta {
            creator: creator.into(),
            ..Default::default()
        };
        assert!(!is_delegated(Some(&meta)), "{creator}");
    }
    let meta = SessionMeta {
        creator: "external".into(),
        ..Default::default()
    };
    assert!(is_delegated(Some(&meta)));
    assert!(!is_delegated(None));
}

/// A question kept to an app reaches her only as that the app waits:
/// nothing of what it asks, and no way to answer it for the person.
#[test]
fn a_kept_question_reaches_her_without_its_words() {
    let kickoff = kept_ask_kickoff("zz-ledger");
    assert!(kickoff.contains("zz-ledger"));
    assert!(!kickoff.contains("answer_prompt"));
    assert!(!kickoff.contains("options"));
}

/// The mark of a told reply lands on its run's last row, once.
#[test]
fn a_told_reply_marks_its_runs_last_row() {
    REPLIES.lock().unwrap().push((
        ("sess-mark-rows".into(), "run-a".into()),
        Told {
            at: 9,
            line: "Ready.".into(),
        },
    ));
    let mut rows = vec![
        serde_json::json!({"run_id": "run-a"}),
        serde_json::json!({"run_id": "run-a"}),
        serde_json::json!({"run_id": "run-b"}),
        serde_json::json!({"from": "user"}),
    ];
    mark_rows("sess-mark-rows", rows.iter_mut());
    assert!(rows[0].get("told_aloud_at").is_none());
    assert_eq!(rows[1]["told_aloud_at"], 9);
    assert_eq!(rows[1]["told_aloud_line"], "Ready.");
    assert!(rows[2].get("told_aloud_at").is_none());
    mark_rows("sess-another", rows.iter_mut());
}

/// Nothing tells a person about Ling's work but [`tell`]: the turn of a
/// notice, its cue and its mark are made in this file alone, and the bus's
/// notices are only turned into a [`Herald`] where they arrive.
mod one_entry {
    use crate::server::lock_scan::production_code;

    fn count(needle: &str) -> Vec<(String, usize)> {
        production_code(&["src", "crates/linggen-engine/src"])
            .into_iter()
            // A `#[cfg(test)] mod tests;` file is read too: not production.
            .filter(|(rel, _)| !rel.ends_with("/tests.rs"))
            .map(|(rel, code)| (rel, code.matches(needle).count()))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    fn only_in(needle: &str, listed: &[(&str, usize)]) {
        let mut found = count(needle);
        found.sort();
        let mut want: Vec<(String, usize)> =
            listed.iter().map(|(f, n)| (f.to_string(), *n)).collect();
        want.sort();
        assert_eq!(found, want, "`{needle}`: a notice of Ling's work is told by `herald::tell` alone — look at it, then list it");
    }

    #[test]
    fn a_notice_is_told_through_the_one_entry_only() {
        // Built from parts, so this test's own text is no match.
        let p = |a: &str, b: &str| format!("{a}{b}");
        const HERE: &str = "src/server/resident/herald.rs";
        // The notice's cue: made here, and defined once.
        only_in(
            &p("emit_", "notice("),
            &[("src/server/api/yinyue.rs", 1), (HERE, 1)],
        );
        // Its mark on what it is about.
        only_in(
            &p("ServerEvent::", "NoticeTold"),
            &[(HERE, 1), ("src/server/ui_events/chat.rs", 1)],
        );
        // The turn of a wait's notice, and the gate of whose it is.
        only_in(&p("herald_", "line("), &[(HERE, 2)]);
        only_in(&p("fn who", "se("), &[(HERE, 1)]);
        only_in(&p("Origin::", "Herald"), &[(HERE, 2)]);
        // The three notices are built where the bus delivers them, and
        // handed to the entry there.
        only_in(
            &p("herald::", "tell("),
            &[("src/server/resident/triggers.rs", 3)],
        );
        only_in(
            &p("Herald::", "Waiting {"),
            &[("src/server/resident/triggers.rs", 1), (HERE, 1)],
        );
        // A mission run's fault keeps its own wake, from its own file.
        only_in(
            &p("wake_", "fault("),
            &[
                ("src/server/resident/mission_fact.rs", 1),
                ("src/server/resident/triggers.rs", 1),
            ],
        );
    }

    /// The general doors her lines go through are listed by every caller:
    /// telling Ling's work through any of them (a restart's wake, a plain
    /// wake, a bare cue, her own turn) is a new call site, and fails here.
    #[test]
    fn no_general_door_tells_lings_work() {
        let p = |a: &str, b: &str| format!("{a}{b}");
        only_in(
            &p("wake_to_", "tell("),
            &[
                ("src/server/restart.rs", 1),
                ("src/server/resident/triggers.rs", 1),
            ],
        );
        only_in(
            &p("wake_and_", "speak("),
            &[
                ("src/server/resident/ambient.rs", 1),
                ("src/server/resident/look_again.rs", 1),
                ("src/server/resident/triggers.rs", 4),
                ("src/server/resident/offer_outcome.rs", 1),
            ],
        );
        only_in(
            &p("wake_and_", "offer("),
            &[
                ("src/server/resident/ambient.rs", 1),
                ("src/server/resident/triggers.rs", 2),
            ],
        );
        only_in(
            &p("emit_", "speak("),
            &[
                ("src/server/resident/spoken.rs", 1),
                ("src/server/api/yinyue.rs", 1),
            ],
        );
        only_in(
            &p("emit_speak_", "offering("),
            &[
                ("src/server/resident/spoken.rs", 1),
                ("src/server/api/yinyue.rs", 2),
            ],
        );
        only_in(
            &p("run_yinyue_", "turn("),
            &[
                ("src/server/resident/triggers.rs", 2),
                ("src/server/resident/turn.rs", 1),
                ("src/server/resident/herald.rs", 2),
                ("src/server/api/yinyue.rs", 1),
            ],
        );
        // The notice's event by any name, a glob import too.
        only_in(
            &p("Notice", "Told"),
            &[
                ("crates/linggen-engine/src/engine/events.rs", 1),
                ("src/server/resident/herald.rs", 1),
                ("src/server/ui_events/chat.rs", 3),
                ("src/server/ui_events/mod.rs", 1),
                ("src/server/ui_events/data.rs", 1),
            ],
        );
    }

    /// A glob import would let a variant be named bare, past every count
    /// above: none in production, but the one event router's.
    #[test]
    fn no_glob_import_hides_a_notice() {
        let p = |a: &str, b: &str| format!("{a}{b}");
        only_in(
            &p("ServerEvent::", "*"),
            &[("src/server/ui_events/mod.rs", 1)],
        );
        for glob in [
            p("Origin::", "*"),
            p("NotificationPayload::", "*"),
            p("Herald::", "*"),
        ] {
            only_in(&glob, &[]);
        }
    }

    /// Every history view marks its rows: a file that builds a page's rows
    /// (`with_run_calls`) calls `mark_rows` as often.
    #[test]
    fn every_history_view_carries_the_marks() {
        let p = |a: &str, b: &str| format!("{a}{b}");
        let mut views: Vec<(String, usize)> = count(&p("run_calls::with_", "run_calls("));
        let mut marks: Vec<(String, usize)> = count(&p("resident::mark_", "rows("));
        views.sort();
        marks.sort();
        assert!(views.len() >= 3, "{views:?}");
        assert_eq!(marks, views, "a history view without the marks");
    }
}
