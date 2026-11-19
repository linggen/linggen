use super::*;

fn seeded_in(dir: &Path) -> Vec<String> {
    seeded(&dir.join(SEEDED)).unwrap()
}

fn parse(text: &str) -> Mapping {
    let (yaml, _) = crate::extensions::frontmatter::split(text);
    serde_norway::from_str::<Mapping>(yaml.unwrap()).unwrap()
}

fn shipped(name: &str) -> String {
    String::from_utf8(builtin(name).unwrap().data.to_vec()).unwrap()
}

#[test]
fn a_fresh_home_gets_every_built_in_mission_once() {
    let dir = tempfile::tempdir().unwrap();
    assert!(sync(dir.path()).unwrap() >= 3);
    for name in ["dream", "memory-scan", "memory-solve"] {
        assert!(dir.path().join(name).join("mission.md").is_file(), "{name}");
    }
    assert_eq!(seeded_in(dir.path()), names());
    assert_eq!(sync(dir.path()).unwrap(), 0);
}

#[test]
fn an_old_install_gets_the_new_missions_and_not_a_deleted_dream() {
    let dir = tempfile::tempdir().unwrap();
    // What an older engine left: the empty sentinel, and the user had
    // deleted `dream`.
    fs::write(dir.path().join(SEEDED), b"").unwrap();
    sync(dir.path()).unwrap();
    assert!(
        !dir.path().join("dream").exists(),
        "a deleted dream came back"
    );
    assert!(dir.path().join("memory-scan/mission.md").is_file());
    assert!(dir.path().join("memory-solve/mission.md").is_file());
    assert!(seeded_in(dir.path()).contains(&"dream".to_string()));
}

#[test]
fn refresh_replaces_engine_fields_but_keeps_personal_settings() {
    let dir = tempfile::tempdir().unwrap();
    sync(dir.path()).unwrap();
    let path = dir.path().join("dream/mission.md");
    let old = "---\nname: dream\nschedule: '17 5 * * *'\nenabled: false\nmodel: my-model\ncatchup_hours: 0\nallowed-tools: [OldTool]\npermission: {paths: [{path: '~', mode: admin}]}\nkickoff: [Old kickoff]\n---\n\nOld instructions.\n";
    fs::write(&path, old).unwrap();
    sync(dir.path()).unwrap();
    let refreshed = fs::read_to_string(&path).unwrap();
    let mut expected = parse(&shipped("dream"));
    for (key, value) in [
        ("schedule", "'17 5 * * *'"),
        ("enabled", "false"),
        ("model", "my-model"),
        ("catchup_hours", "0"),
    ] {
        expected.insert(key.into(), serde_norway::from_str(value).unwrap());
    }
    assert_eq!(parse(&refreshed), expected);
    assert_eq!(fs::read_to_string(path.with_extension("md.prev")).unwrap(), old);

    // A later refresh keeps the first backup; a personal field the file
    // lacks takes the built-in's default (a dream installed before
    // `catchup_hours` existed gets the built-in's 24), and one the
    // built-in does not set stays unset.
    fs::write(&path, "---\nname: dream\n---\n\nAnother older body.\n").unwrap();
    sync(dir.path()).unwrap();
    let refreshed = parse(&fs::read_to_string(&path).unwrap());
    let builtin = parse(&shipped("dream"));
    for key in PERSONAL_MISSION_FIELDS {
        assert_eq!(refreshed.get(*key), builtin.get(*key), "{key}");
    }
    assert_eq!(
        refreshed.get("catchup_hours"),
        Some(&serde_norway::Value::from(24))
    );
    assert_eq!(fs::read_to_string(path.with_extension("md.prev")).unwrap(), old);
}

/// A file that lacks only a personal field the built-in sets is refreshed
/// to carry it, and that is no change of its instructions; one whose
/// instructions differ is a replacement of them.
#[test]
fn a_missing_personal_field_gets_the_built_ins_default() {
    let builtin = shipped("dream");
    let mut without = parse(&builtin);
    without.remove("catchup_hours");
    let body = crate::extensions::frontmatter::split(&builtin).1;
    let installed = format!("---\n{}---{body}", serde_norway::to_string(&without).unwrap());
    let merged = merge(&installed, &builtin).unwrap().expect("refreshed");
    assert!(!merged.instructions_replaced);
    assert_eq!(
        parse(&merged.text).get("catchup_hours"),
        Some(&serde_norway::Value::from(24))
    );
    let edited = builtin.replacen("\n\n", "\n\nMy own words.\n\n", 1);
    assert!(merge(&edited, &builtin).unwrap().unwrap().instructions_replaced);
    assert!(merge(&builtin, &builtin).unwrap().is_none());
}

#[test]
fn refresh_ignores_edits_to_personal_settings_alone() {
    let dir = tempfile::tempdir().unwrap();
    sync(dir.path()).unwrap();
    let path = dir.path().join("dream/mission.md");
    let customized = fs::read_to_string(&path)
        .unwrap()
        .replace("enabled: true", "enabled: false # paused");
    fs::write(&path, &customized).unwrap();
    assert_eq!(sync(dir.path()).unwrap(), 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), customized);
    assert!(!path.with_extension("md.prev").exists());
}

/// A person's own file under a built-in's name is left alone — unless an
/// earlier engine listed that name in the install marker: the marker cannot
/// tell it from a built-in edited by hand, so it is refreshed, and the
/// person's file is kept as `mission.md.prev`.
#[test]
fn a_users_own_file_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let mine = dir.path().join("memory-scan/mission.md");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::write(&mine, "mine").unwrap();
    sync(dir.path()).unwrap();
    assert_eq!(sync(dir.path()).unwrap(), 0);
    assert_eq!(fs::read_to_string(&mine).unwrap(), "mine");
    assert!(!mine.with_extension("md.prev").exists());
    assert!(!is_builtin(dir.path(), "memory-scan"));

    let listed = tempfile::tempdir().unwrap();
    let theirs = listed.path().join("memory-solve/mission.md");
    fs::create_dir_all(theirs.parent().unwrap()).unwrap();
    let own = "---\nname: my solve\nschedule: '0 9 * * *'\n---\n\nMy own solve.\n";
    fs::write(&theirs, own).unwrap();
    fs::write(listed.path().join(SEEDED), "dream\nmemory-solve\n").unwrap();
    sync(listed.path()).unwrap();
    let refreshed = fs::read_to_string(&theirs).unwrap();
    assert_eq!(
        crate::extensions::frontmatter::split(&refreshed).1,
        crate::extensions::frontmatter::split(&shipped("memory-solve")).1,
        "refreshed to the built-in"
    );
    assert_eq!(parse(&refreshed)["schedule"], "0 9 * * *", "its schedule kept");
    assert_eq!(fs::read_to_string(theirs.with_extension("md.prev")).unwrap(), own);
}

/// A change to a built-in may touch only the person's fields; any other
/// mission is anyone's to change.
#[test]
fn a_built_ins_instructions_are_not_the_persons_to_change() {
    let dir = tempfile::tempdir().unwrap();
    sync(dir.path()).unwrap();
    let now = fs::read_to_string(dir.path().join("dream/mission.md")).unwrap();
    let paused = with_personal(&now, &[("enabled", Some(false.into()))]).unwrap();
    assert!(check_change(dir.path(), "dream", Some(&now), &paused).is_ok());
    let rewritten = now.replacen("\n\n", "\n\nDo something else.\n\n", 1);
    let refused = check_change(dir.path(), "dream", Some(&now), &rewritten).unwrap_err();
    assert!(refused.downcast_ref::<BuiltInOwned>().is_some());
    assert!(refused.to_string().contains("copy it under a new name"), "{refused}");
    assert!(check_change(dir.path(), "dream", None, &now).is_err(), "a deleted built-in");
    assert!(check_change(dir.path(), "mine", None, "anything").is_ok());
}
