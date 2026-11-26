use midnight_core::*;

#[test]
fn find_default_preserves_saved_alt_f7_assignments() {
    let chord = ShortcutChord::parse("Alt+F7").unwrap();
    let defaults = ShortcutMap::new(&[]).unwrap();
    assert_eq!(
        defaults.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.find"
    );
    let existing = ShortcutBinding {
        action: "files.properties".into(),
        keys: vec!["Alt+F7".into()],
    };
    let migrated = ShortcutMap::new(std::slice::from_ref(&existing)).unwrap();
    assert_eq!(
        migrated.resolve(&chord, ShortcutContext::Files).unwrap().id,
        "files.properties"
    );
    assert_eq!(
        migrated.bindings("files.find").unwrap(),
        &[ShortcutChord::parse("Ctrl+F").unwrap()],
        "claiming Alt+F7 must preserve the independent Ctrl+F default"
    );
    assert!(
        ShortcutMap::new(&[
            existing,
            ShortcutBinding {
                action: "files.find".into(),
                keys: vec!["Alt+F7".into()],
            },
        ])
        .is_err()
    );
}

#[test]
fn find_defaults_match_literal_recursive_native_search_and_validate_limits() {
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![std::env::current_dir().unwrap()],
    );
    assert!(request.recursive);
    assert_eq!(request.masks, "*.*");
    assert!(!request.name_case_sensitive);
    assert!(request.content.is_none());
    assert!(request.modified_within.is_none());
    assert!(!request.duplicates);
    assert!(request.validate().is_ok());
    request.limits.max_results = 0;
    assert!(request.validate().is_err());
}

#[test]
fn find_modified_age_and_duplicate_candidate_limits_are_bounded() {
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![std::env::current_dir().unwrap()],
    );
    request.modified_within = Some(std::time::Duration::ZERO);
    assert!(request.validate().is_err());
    request.modified_within = Some(std::time::Duration::from_secs(60));
    assert!(request.validate().is_ok());
    request.limits.max_duplicate_candidates = 0;
    assert!(request.validate().is_err());
}

#[test]
fn find_scan_count_is_unlimited_by_default_and_accepts_explicit_budgets() {
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![std::env::current_dir().unwrap()],
    );
    assert_eq!(request.limits.max_nodes, 0);
    assert_eq!(request.limits.max_results, 100_000);
    assert!(request.validate().is_ok());
    for maximum in [1, 2_000_000, u64::MAX] {
        request.limits.max_nodes = maximum;
        assert!(request.validate().is_ok(), "scan budget {maximum}");
    }
    request.limits.max_results = 100_001;
    assert!(request.validate().is_err());
    request.limits.max_results = 0;
    assert!(request.validate().is_err(), "retained results stay bounded");
}

#[test]
fn find_rejects_ambiguous_roots_and_empty_or_unbounded_content_queries() {
    let mut request =
        FindRequest::new(FindSessionId(1), FindGeneration(1), vec!["relative".into()]);
    assert!(request.validate().is_err());
    request.roots = vec![std::env::current_dir().unwrap()];
    request.content = Some(FindContentQuery {
        text: String::new(),
        case_sensitive: false,
        whole_words: false,
        regex: false,
    });
    assert!(request.validate().is_err());
    request.content.as_mut().unwrap().text = "č".repeat(4097);
    assert!(request.validate().is_err());
}

fn captured_file() -> FindEntry {
    FindEntry {
        path: "a.zip".into(),
        kind: EntryKind::File,
        metadata: Default::default(),
        identity: TransferIdentity {
            device: 7,
            object: [9; 16],
        },
        revision: FindRevision([11, 12, 13, 14, 15, 16]),
    }
}

/// Only the change time may move: on Windows ChangeTime (slot 4), on Linux the ctime
/// seconds and nanoseconds (slots 3 and 4). Every other slot (size, write time, creation
/// time or mtime nanoseconds, attributes or mode) still counts as a change.
#[test]
fn find_accepts_a_change_of_the_change_time_alone() {
    let captured = captured_file();
    assert!(captured.matches_capture_except_change_time(&captured));
    #[cfg(windows)]
    assert_eq!(FindRevision::CHANGE_TIME, &[4]);
    #[cfg(not(windows))]
    assert_eq!(FindRevision::CHANGE_TIME, &[3, 4]);
    let mut all_change_slots = captured.clone();
    for slot in 0..6 {
        let mut moved = captured.clone();
        moved.revision.0[slot] += 1;
        let change_time = FindRevision::CHANGE_TIME.contains(&slot);
        assert_eq!(
            moved.matches_capture_except_change_time(&captured),
            change_time,
            "slot {slot}"
        );
        assert!(!moved.matches_capture(&captured), "slot {slot}");
        if change_time {
            all_change_slots.revision.0[slot] += 100;
        }
    }
    assert!(all_change_slots.matches_capture_except_change_time(&captured));
}

#[test]
fn find_rejects_another_object_whatever_its_change_time() {
    let captured = captured_file();
    let mut other = captured.clone();
    other.identity.device += 1;
    assert!(!other.matches_capture_except_change_time(&captured));
    let mut other = captured.clone();
    other.identity.object[0] ^= 1;
    assert!(!other.matches_capture_except_change_time(&captured));
    let mut other = captured.clone();
    other.path = "b.zip".into();
    assert!(!other.matches_capture_except_change_time(&captured));
    let mut other = captured.clone();
    other.kind = EntryKind::Directory;
    assert!(!other.matches_capture_except_change_time(&captured));
}
