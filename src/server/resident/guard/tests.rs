use super::*;

const NOW: u64 = 1_790_000_000;

fn pet(per_hour: u32, quiet: &str) -> PetConfig {
    PetConfig {
        unprompted_per_hour: per_hour,
        quiet_hours: quiet.to_string(),
        ..Default::default()
    }
}

fn at(h: u32, m: u32) -> u32 {
    h * 60 + m
}

#[test]
fn the_defaults_are_named() {
    let p = PetConfig::default();
    assert_eq!(
        p.unprompted_per_hour,
        crate::config::DEFAULT_UNPROMPTED_PER_HOUR
    );
    assert_eq!(p.quiet_hours, crate::config::DEFAULT_QUIET_HOURS);
    assert!(QuietHours::parse(&p.quiet_hours).is_some());
}

#[test]
fn quiet_hours_cross_midnight() {
    let q = QuietHours::parse("23:00-07:00").unwrap();
    for inside in [at(23, 0), at(23, 59), at(0, 0), at(3, 30), at(6, 59)] {
        assert!(q.contains(inside), "{inside}");
    }
    for outside in [at(7, 0), at(12, 0), at(22, 59)] {
        assert!(!q.contains(outside), "{outside}");
    }
    let day = QuietHours::parse(" 13:00 - 14:30 ").unwrap();
    assert!(day.contains(at(13, 0)) && day.contains(at(14, 29)) && !day.contains(at(14, 30)));
}

#[test]
fn empty_or_unreadable_quiet_hours_are_none() {
    for none in [
        "",
        "   ",
        "23:00",
        "23:00-23:00",
        "late-early",
        "25:00-07:00",
    ] {
        assert_eq!(QuietHours::parse(none), None, "{none}");
    }
}

#[test]
fn quiet_hours_hold_and_say_until_when() {
    let rules = Rules::of(&pet(6, "23:00-07:00"));
    let lines = Lines::new();
    assert_eq!(
        rules.check(Source::Glance, NOW, at(1, 0), &lines),
        Some(Held::Quiet {
            until: "07:00".into()
        })
    );
    assert_eq!(rules.check(Source::Glance, NOW, at(9, 0), &lines), None);
    // No quiet hours set: none at night either.
    assert_eq!(
        Rules::of(&pet(6, "")).check(Source::Glance, NOW, at(1, 0), &lines),
        None
    );
}

#[test]
fn the_ceiling_counts_a_rolling_hour() {
    let rules = Rules::of(&pet(3, ""));
    let mut lines = Lines::new();
    lines.note(NOW - 3500);
    lines.note(NOW - 1000);
    assert_eq!(
        rules.check(Source::Glance, NOW, at(12, 0), &lines),
        None,
        "two in the hour"
    );
    lines.note(NOW - 10);
    assert_eq!(
        rules.check(Source::Glance, NOW, at(12, 0), &lines),
        Some(Held::Ceiling { per_hour: 3 })
    );
    // The oldest leaves the window: room for one more.
    assert_eq!(
        rules.check(Source::Glance, NOW + 101, at(12, 2), &lines),
        None
    );
}

#[test]
fn a_ceiling_of_zero_holds_every_unprompted_line() {
    let rules = Rules::of(&pet(0, ""));
    assert_eq!(
        rules.check(Source::Glance, NOW, at(12, 0), &Lines::new()),
        Some(Held::Ceiling { per_hour: 0 })
    );
}

#[test]
fn what_is_held_is_recorded_once_without_its_words() {
    let held = Held::Quiet {
        until: "07:00".into(),
    };
    let item = Withheld {
        what: "a look-again on n3".into(),
        source: "note",
        key: "test-note-n3-1790000000".into(),
    };
    let (object, detail) = held_record(&item, &held).unwrap();
    assert_eq!(object, "a look-again on n3 (quiet hours)");
    assert!(object.chars().count() <= 64);
    assert_eq!(detail["reason"], "quiet-hours");
    assert_eq!(detail["source"], "note");
    assert_eq!(detail["until"], "07:00");
    // The same held wake, tick after tick: one record, never a doorbell loop.
    for _ in 0..100 {
        assert!(held_record(&item, &held).is_none());
    }
    let ceiling = held_record(
        &Withheld {
            what: "a line".into(),
            source: "glance",
            key: "test-line-1".into(),
        },
        &Held::Ceiling { per_hour: 6 },
    )
    .unwrap();
    assert_eq!(ceiling.1["per_hour"], 6);
}

#[test]
fn hold_is_an_engine_verb_that_reads_as_held() {
    assert!(crate::perception::activity::ENGINE_VERBS.contains(&HOLD_VERB));
    let a = crate::perception::activity::Activity {
        at: "2026-11-16T10:00:00Z".into(),
        by: super::super::YINYUE_AGENT.into(),
        device: None,
        app: "linggen".into(),
        verb: HOLD_VERB.into(),
        object: Some("a line (hourly ceiling)".into()),
        detail: None,
    };
    assert_eq!(a.line(), "yinyue held a line (hourly ceiling)");
}

/// Quiet hours let a moment from the app the person is using by; the ceiling
/// still counts it and can hold it. Another app's moment is held at night.
#[test]
fn quiet_hours_do_not_hold_a_moment_from_the_app_in_use() {
    let rules = Rules::of(&pet(2, "23:00-07:00"));
    let mut lines = Lines::new();
    let night = at(23, 30);
    assert_eq!(rules.check(Source::InUse, NOW, night, &lines), None);
    assert_eq!(
        rules.check(Source::Moment, NOW, night, &lines),
        Some(Held::Quiet {
            until: "07:00".into()
        }),
        "another app's moment waits for the morning"
    );
    lines.note(NOW - 100);
    lines.note(NOW - 50);
    assert_eq!(
        rules.check(Source::InUse, NOW, night, &lines),
        Some(Held::Ceiling { per_hour: 2 }),
        "the ceiling holds it all the same"
    );
}

/// A fault waits through quiet hours, and the hourly ceiling is not for it.
#[test]
fn a_fault_waits_through_quiet_hours_and_is_outside_the_ceiling() {
    let rules = Rules::of(&pet(1, "23:00-07:00"));
    let mut lines = Lines::new();
    assert_eq!(
        rules.check(Source::Fault, NOW, at(2, 0), &lines),
        Some(Held::Quiet {
            until: "07:00".into()
        })
    );
    lines.note(NOW - 10);
    assert_eq!(
        rules.check(Source::Fault, NOW, at(12, 0), &lines),
        None,
        "the ceiling is reached, a fault still goes"
    );
    assert_eq!(
        rules.check(Source::Glance, NOW, at(12, 0), &lines),
        Some(Held::Ceiling { per_hour: 1 })
    );
    assert!(!Source::Fault.counts() && Source::InUse.counts() && Source::Note.counts());
    let zero = Rules::of(&pet(0, ""));
    assert_eq!(
        zero.check(Source::Fault, NOW, at(12, 0), &Lines::new()),
        None
    );
}

#[test]
fn a_source_is_named_by_what_the_record_says() {
    assert_eq!(Source::InUse.name(), "moment");
    assert_eq!(Source::Moment.name(), "moment");
    assert_eq!(Source::Fault.name(), "fault");
}
