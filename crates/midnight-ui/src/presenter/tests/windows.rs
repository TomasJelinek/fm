use super::*;

pub(super) fn test_root() -> PathBuf {
    PathBuf::from("C:\\")
}

#[test]
fn unique_bookmarks_compare_native_utf16_without_display_loss() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let second = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd801]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let mut p = Presenter::new(first.clone(), second.clone());
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
    assert!(p.add_bookmark(PaneId::Left, "First".into(), None, &|_| true));
    assert!(p.add_bookmark(PaneId::Right, "Second".into(), None, &|_| true));
    assert!(
        p.add_bookmark(PaneId::Left, "Duplicate".into(), None, &|_| panic!(
            "duplicate"
        ))
    );
    assert_eq!(p.bookmarks().len(), 2);
    assert_eq!(p.bookmarks()[0].path.as_os_str(), first.as_os_str());
    assert_eq!(p.bookmarks()[1].path.as_os_str(), second.as_os_str());
}

#[test]
fn native_choice_tokens_preserve_unpaired_utf16_and_reject_display_alias_replacement() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let second = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd801]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let mut p = Presenter::new(test_root(), test_root());
    let update = |path| Event::DesktopContext {
        shell: ShellFlavor::PowerShell,
        locations: vec![NavigationLocation {
            id: "desktop".into(),
            label: "Same caption".into(),
            path,
            kind: LocationKind::Desktop,
            drive_letter: None,
        }],
    };
    p.apply(update(first.clone()));
    let token = p.navigation_choice_token(0);
    assert!(
        p.choose_navigation_choice(PaneId::Left, &token, &|command| {
            assert!(matches!(command, Command::Navigate { path, .. } if path == first));
            true
        })
    );
    p.apply(update(second));
    assert!(!p.choose_navigation_choice(PaneId::Right, &token, &|_| panic!("stale")));
}

#[test]
fn bookmark_and_tab_actions_preserve_unpaired_native_utf16() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let mut p = Presenter::new(path.clone(), test_root());
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
    assert!(
        p.add_bookmark(PaneId::Left, "Raw native".into(), Some(1), &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            assert_eq!(preferences.bookmarks[0].path, path);
            true
        })
    );
    assert!(p.new_tab(PaneId::Left, &|command| {
        match command {
            Command::Navigate { path: actual, .. } => assert_eq!(actual, path),
            Command::SavePreferences { preferences, .. } => {
                assert_eq!(preferences.tabs[0].paths, [path.clone(), path.clone()]);
            }
            _ => panic!("expected navigation or persisted tab locations"),
        }
        true
    }));
    assert_eq!(p.tabs(PaneId::Left)[1].path, path);
    assert!(p.jump_bookmark(PaneId::Right, 1, &|command| {
        match command {
            Command::Navigate {
                pane, path: actual, ..
            } => {
                assert_eq!(pane, PaneId::Right);
                assert_eq!(actual, path);
            }
            Command::SavePreferences { preferences, .. } => {
                assert_eq!(
                    preferences.tabs[1].paths.as_slice(),
                    std::slice::from_ref(&path)
                );
            }
            _ => panic!("expected navigation or its persisted tab location"),
        }
        true
    }));
}

#[test]
fn volumes_revision_distinguishes_raw_roots_with_identical_unicode_display() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let second = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd801]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let mut presenter = Presenter::new(test_root(), test_root());
    let update = |root| {
        Event::VolumesUpdated(vec![Volume {
            id: "same-display".into(),
            label: "Drive".into(),
            root,
        }])
    };
    presenter.apply(update(first));
    let before = presenter.volumes_revision;
    presenter.apply(update(second));
    assert_ne!(presenter.volumes_revision, before);
}

#[test]
fn opening_uses_raw_path_even_when_display_name_is_lossy() {
    use std::os::windows::ffi::OsStringExt;
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800]));
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![Entry::new(EntryId(1), path.clone(), EntryKind::Directory)],
    );
    assert!(p.open(PaneId::Left, 0, &|command| {
        match command {
            Command::Navigate { path: actual, .. } => assert_eq!(actual, path),
            _ => panic!(),
        };
        true
    }));
}

#[test]
fn file_association_request_preserves_unpaired_native_utf16() {
    use std::os::windows::ffi::OsStringExt;
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800]));
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![Entry::new(EntryId(0), path.clone(), EntryKind::File)],
    );
    assert!(p.open(PaneId::Left, 0, &|command| {
        assert!(matches!(command, Command::OpenEntry { path: actual, .. } if actual == path));
        true
    }));
}
#[test]
fn submitting_unchanged_lossy_path_text_reuses_the_original_raw_path() {
    use std::os::windows::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800]));
    let mut p = Presenter::new(path.clone(), "right".into());
    assert!(
        p.navigate_text(PaneId::Left, path.to_string_lossy().as_ref(), &|command| {
            assert!(matches!(command, Command::Navigate { path: actual, .. } if actual == path));
            true
        })
    );
}
