use std::path::PathBuf;

pub(super) fn test_root() -> PathBuf {
    PathBuf::from("/")
}

#[test]
fn unique_bookmarks_compare_native_bytes_without_display_loss() {
    use crate::Presenter;
    use midnight_core::{Event, PaneId, UserPreferences};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_vec(b"/tmp/raw-\x80".to_vec()));
    let second = PathBuf::from(OsString::from_vec(b"/tmp/raw-\x81".to_vec()));
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
fn native_choice_tokens_preserve_non_utf8_bytes_and_reject_display_alias_replacement() {
    use crate::Presenter;
    use midnight_core::{Command, Event, LocationKind, NavigationLocation, PaneId, ShellFlavor};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_vec(b"/tmp/raw-\x80".to_vec()));
    let second = PathBuf::from(OsString::from_vec(b"/tmp/raw-\x81".to_vec()));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    let mut p = Presenter::new(test_root(), test_root());
    let update = |path| Event::DesktopContext {
        shell: ShellFlavor::Posix,
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
fn bookmark_and_tab_actions_preserve_non_utf8_native_bytes() {
    use crate::Presenter;
    use midnight_core::{Command, Event, PaneId, UserPreferences};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_vec(b"/tmp/\xff".to_vec()));
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
    use crate::Presenter;
    use midnight_core::{Event, Volume};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let first = PathBuf::from(OsString::from_vec(b"/mnt/raw-\x80".to_vec()));
    let second = PathBuf::from(OsString::from_vec(b"/mnt/raw-\x81".to_vec()));
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
fn file_association_request_preserves_native_non_utf8_bytes() {
    use super::{begin, finish};
    use crate::Presenter;
    use midnight_core::{Command, Entry, EntryId, EntryKind, PaneId};
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mut presenter = Presenter::new("a".into(), "right".into());
    let navigation = begin(&mut presenter, PaneId::Left, "a");
    let path = PathBuf::from(OsString::from_vec(b"/tmp/raw-\xff".to_vec()));
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(0), path.clone(), EntryKind::File)],
    );
    assert!(presenter.open(PaneId::Left, 0, &|command| {
        assert!(matches!(command, Command::OpenEntry { path: actual, .. } if actual == path));
        true
    }));
}
