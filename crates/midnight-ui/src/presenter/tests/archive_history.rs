//! One history per tab for native folders and folders inside archives: entering an
//! archive, an archive inside it, its folders and leaving it are recorded like native
//! folders. Back into an archive that the pane has left opens it again, archives inside
//! archives included, at the same folder; Hot Paths and saved tabs keep every level.
use super::*;

/// A session of an archive opened from a native file, as the backend opens it again.
#[derive(Debug)]
struct Root(u64);
impl ArchiveMount for Root {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI performed listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI performed reading")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("UI performed validation")
    }
}

/// The last native navigation sent, and its folder.
fn last_navigate(sent: &RefCell<Vec<Command>>) -> (NavigationId, PathBuf) {
    sent.borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                navigation, path, ..
            } => Some((*navigation, path.clone())),
            _ => None,
        })
        .expect("a native navigation")
}

/// The folder that holds `sample.zip`, listed.
fn native_listed(p: &mut Presenter, navigation: NavigationId) {
    finish(
        p,
        PaneId::Left,
        navigation,
        vec![Entry::new(
            EntryId(7),
            test_root().join("sample.zip"),
            EntryKind::File,
        )],
    );
}

/// The archive inside the archive that a saved address opens again: the backend lists
/// it through new sessions, `outer` for the archive opened from the native file.
fn reopened_inner(saved: &ArchiveLocation, outer: u64, inner: u64) -> ArchiveLocation {
    assert!(
        saved.mount.is_none(),
        "a saved address opens its archives again"
    );
    let level = &saved.address.nested[0];
    let enclosing = ArchiveLocation {
        mount: Some(Arc::new(Root(outer))),
        address: ArchiveAddress {
            nested: Vec::new(),
            ..saved.address.clone()
        },
        directory: 1,
        trail: vec![(1, "sample.zip".into())],
    };
    let parent = ArchiveParent {
        location: enclosing,
        member: VirtualEntryId {
            mount: outer,
            entry: 5,
        },
        name: String::from_utf8_lossy(&level.member).into_owned(),
        raw_name: level.member.clone(),
    };
    let mut location = inner_root(&parent, inner);
    for name in &level.components {
        location.directory = 2;
        location
            .trail
            .push((2, String::from_utf8_lossy(name).into_owned()));
        location.address.components.push(name.clone());
    }
    location
}

fn shown_mount(p: &Presenter) -> Option<u64> {
    p.panes[0].archive.as_ref()?.location.mount_id()
}

#[test]
fn back_and_forward_go_into_archives_inside_archives_and_out_again() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    // The native folder, then the archive's root.
    let mut p = outer_pane(&sent);
    assert_eq!(p.panes[0].history.len(), 2);
    assert_eq!(p.panes[0].history[0], test_root());
    // The archive inside it, and a folder there.
    enter_inner(&mut p, &sent, 43);
    assert!(p.open(PaneId::Left, row(&p, "folder"), &send));
    listed(&mut p, Ok(listing(last_listing(&sent), Vec::new())), &sent);
    let separator = std::path::MAIN_SEPARATOR;
    let deepest = format!("sample.zip{separator}inner.zip{separator}folder");
    assert!(p.panes[0].display_location_text().ends_with(&deepest));
    assert_eq!(p.panes[0].history.len(), 4);
    assert_eq!(p.panes[0].history_index, Some(3));
    // The history lists show every entry with its whole path.
    let back: Vec<_> = p
        .history_entries(PaneId::Left, false)
        .into_iter()
        .map(|choice| {
            choice
                .display
                .unwrap_or_else(|| choice.path.display().to_string())
        })
        .collect();
    assert_eq!(back.len(), 3);
    assert!(back[0].ends_with(&format!("sample.zip{separator}inner.zip")));
    assert!(back[1].ends_with("sample.zip"));
    assert_eq!(back[2], test_root().display().to_string());
    // Back through the archives, through their sessions.
    for (mount, directory) in [(43, 1), (42, 1)] {
        assert!(p.history(PaneId::Left, -1, &send));
        let location = last_listing(&sent);
        assert_eq!(
            (location.mount_id(), location.directory),
            (Some(mount), directory)
        );
        listed(&mut p, Ok(listing(location, root_entries(mount))), &sent);
    }
    assert_eq!(p.panes[0].history_index, Some(1));
    assert_eq!(p.panes[0].history.len(), 4, "Back keeps the entries");
    // Back out of the archive: the native folder, the cursor on the archive.
    assert!(p.history(PaneId::Left, -1, &send));
    let (navigation, folder) = last_navigate(&sent);
    assert_eq!(folder, test_root());
    assert_eq!(
        p.panes[0].pending.as_ref().unwrap().wanted_cursor,
        Some(test_root().join("sample.zip"))
    );
    native_listed(&mut p, navigation);
    assert!(p.panes[0].archive.is_none());
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    assert_eq!(p.panes[0].history_index, Some(0));
    assert!(
        history::archives(&p.panes[0].history).all(|location| location.mount.is_none()),
        "the pane let go of the archive's sessions"
    );
    // Forward opens the archive again, then the archive inside it, then its folder.
    assert!(p.history(PaneId::Left, 1, &send));
    let root = last_listing(&sent);
    assert!(root.mount.is_none() && root.address.nested.is_empty());
    let mut reopened = inner_root(
        &ArchiveParent {
            location: root.clone(),
            member: VirtualEntryId { mount: 0, entry: 0 },
            name: String::new(),
            raw_name: Vec::new(),
        },
        50,
    );
    reopened.mount = Some(Arc::new(Root(50)));
    reopened.trail = vec![(1, "sample.zip".into())];
    listed(&mut p, Ok(listing(reopened, root_entries(50))), &sent);
    assert_eq!(shown_mount(&p), Some(50));
    assert_eq!(p.panes[0].history_index, Some(1));
    assert!(p.history(PaneId::Left, 1, &send));
    let inner = last_listing(&sent);
    assert_eq!(inner.address.nested.len(), 1);
    assert_eq!(inner.address.nested[0].member, b"inner.zip");
    assert!(inner.address.nested[0].components.is_empty());
    listed(
        &mut p,
        Ok(listing(reopened_inner(&inner, 50, 51), root_entries(51))),
        &sent,
    );
    assert_eq!(shown_mount(&p), Some(51));
    assert!(p.history(PaneId::Left, 1, &send));
    let folder = last_listing(&sent);
    assert_eq!(folder.address.nested[0].components, [b"folder".to_vec()]);
    assert!(
        folder.display_text().ends_with(&deepest),
        "{}",
        folder.display_text()
    );
    listed(
        &mut p,
        Ok(listing(reopened_inner(&folder, 52, 53), Vec::new())),
        &sent,
    );
    assert!(p.panes[0].display_location_text().ends_with(&deepest));
    assert_eq!(p.panes[0].history_index, Some(3));
    assert_eq!(p.panes[0].history.len(), 4, "Forward records nothing new");
    assert!(!p.history(PaneId::Left, 1, &send));
}

#[test]
fn backspace_out_of_an_archive_is_recorded_and_back_opens_it_again() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    assert!(p.up(PaneId::Left, &send));
    let (navigation, _) = last_navigate(&sent);
    native_listed(&mut p, navigation);
    assert_eq!(
        p.panes[0].history.len(),
        3,
        "leaving the archive is a navigation"
    );
    // Back re-opens the closed archive at its root.
    assert!(p.history(PaneId::Left, -1, &send));
    let root = last_listing(&sent);
    assert!(root.mount.is_none());
    assert!(root.address.components.is_empty() && root.address.nested.is_empty());
    // The archive no longer opens: the native folder stays, a notice says why, and the
    // folder is read again so that the backend serves it as before.
    let before = sent.borrow().len();
    listed(
        &mut p,
        Err(FsError::from_io(
            "open archive",
            &root.address.container,
            io::Error::new(io::ErrorKind::NotFound, "The archive was deleted"),
        )),
        &sent,
    );
    assert!(p.panes[0].archive.is_none());
    let (title, body) = p.desktop.notice.clone().expect("a notice");
    assert_eq!(title, "Archive unavailable");
    assert!(body.contains("The archive was deleted"), "{body}");
    assert_eq!(p.panes[0].history_index, Some(2));
    assert_eq!(p.panes[0].history.len(), 3);
    assert!(
        sent.borrow()[before..].iter().any(
            |command| matches!(command, Command::Refresh { path, .. } if path == &test_root())
        ),
        "the native folder is read again"
    );
}

#[test]
fn tabs_keep_their_own_history_through_archives() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    let first = p.active_tab(PaneId::Left);
    assert!(p.new_tab(PaneId::Left, &send));
    listed(
        &mut p,
        Ok(listing(last_listing(&sent), root_entries(43))),
        &sent,
    );
    assert_eq!(
        p.panes[0].history.len(),
        1,
        "a new tab starts its own history"
    );
    assert!(p.select_tab(PaneId::Left, first, &send));
    listed(
        &mut p,
        Ok(listing(last_listing(&sent), root_entries(43))),
        &sent,
    );
    assert_eq!(p.panes[0].history.len(), 3);
    assert_eq!(p.panes[0].history_index, Some(2));
    assert!(p.history(PaneId::Left, -1, &send));
    assert_eq!(last_listing(&sent).mount_id(), Some(42));
}

#[test]
fn hot_paths_and_saved_tabs_four_archives_deep_keep_every_level() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    for mount in 43..=45 {
        enter_inner(&mut p, &sent, mount);
    }
    let separator = std::path::MAIN_SEPARATOR;
    let whole = format!("sample.zip{separator}inner.zip{separator}inner.zip{separator}inner.zip");
    let bookmark = p
        .capture_bookmark_location(PaneId::Left, "deep".into())
        .unwrap();
    bookmark.validate().unwrap();
    let address = bookmark.archive.clone().unwrap();
    assert_eq!(address.nested.len(), 3);
    assert_eq!(address.source_version, [1, 2, 3], "the outermost revision");
    assert_eq!(bookmark.path, test_root());
    assert!(
        bookmark.display_text().ends_with(&whole),
        "{}",
        bookmark.display_text()
    );
    // The Hot Path opens every level from the native file, and the tab keeps its history.
    let history = p.panes[0].history.len();
    assert_eq!(history, 5);
    assert!(p.open_archive_address(PaneId::Left, address.clone(), &send));
    let opened = last_listing(&sent);
    assert!(opened.mount.is_none() && opened.address == address);
    assert!(p.panes[0].display_location_text().ends_with(&whole));
    assert_eq!(p.panes[0].history.len(), history);
    assert!(
        history::archives(&p.panes[0].history).all(|location| location.mount.is_none()),
        "the history let go of the sessions the Hot Path replaced"
    );
    // Saved tabs keep the same location (session restore).
    assert!(p.new_tab(PaneId::Left, &send));
    let saved = &p.preferences.values.tabs[0].archives;
    assert_eq!(saved.len(), 2);
    assert!(
        saved.values().all(|saved| saved.same_location(&address)),
        "{saved:?}"
    );
}

#[test]
fn a_member_that_needs_its_password_is_opened_where_the_backend_stopped() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    let address = p.panes[0].archive.as_ref().unwrap().saved_address();
    assert!(p.up(PaneId::Left, &send));
    listed(
        &mut p,
        Ok(listing(last_listing(&sent), root_entries(42))),
        &sent,
    );
    assert!(p.up(PaneId::Left, &send));
    let (navigation, _) = last_navigate(&sent);
    native_listed(&mut p, navigation);
    // Back into the archive inside the archive: the backend stops at the folder that
    // holds the member (its archive needs a password there), and the pane goes on.
    assert!(p.history(PaneId::Left, -2, &send));
    let saved = last_listing(&sent);
    assert!(saved.address.same_location(&address));
    let entries = p.panes[0].history.len();
    let mut outer = inner_root(
        &ArchiveParent {
            location: saved.clone(),
            member: VirtualEntryId { mount: 0, entry: 0 },
            name: String::new(),
            raw_name: Vec::new(),
        },
        60,
    );
    outer.mount = Some(Arc::new(Root(60)));
    outer.trail = vec![(1, "sample.zip".into())];
    outer.address.nested.clear();
    listed(&mut p, Ok(listing(outer, root_entries(60))), &sent);
    assert_eq!(
        p.panes[0].history.len(),
        entries,
        "nothing recorded on the way"
    );
    p.flush_selection(&send);
    let (_, parent, format) = last_open(&sent).expect("the member opens where it stands");
    assert_eq!(
        (parent.raw_name.as_slice(), format),
        (b"inner.zip".as_slice(), ArchiveFormat::Zip)
    );
    // Once per listing: another flush opens nothing more.
    let count = sent.borrow().len();
    p.flush_selection(&send);
    assert_eq!(sent.borrow().len(), count);
    listed(
        &mut p,
        Ok(listing(inner_root(&parent, 61), root_entries(61))),
        &sent,
    );
    // Reached: the history entry that Back chose shows it.
    assert_eq!(p.panes[0].history.len(), entries);
    assert_eq!(p.panes[0].history_index, Some(2));
    assert_eq!(shown_mount(&p), Some(61));
}

#[test]
fn a_saved_level_that_is_gone_is_reported_with_its_level() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    let mut address = p.panes[0].archive.as_ref().unwrap().saved_address();
    address.nested.push(NestedArchiveAddress {
        member: b"gone.zip".to_vec(),
        format: ArchiveFormat::Zip,
        components: Vec::new(),
    });
    assert!(p.open_archive_address(PaneId::Left, address, &send));
    let saved = last_listing(&sent);
    let mut outer = saved.clone();
    outer.mount = Some(Arc::new(Root(70)));
    outer.trail.truncate(1);
    outer.directory = 1;
    outer.address.nested.clear();
    listed(&mut p, Ok(listing(outer, root_entries(70))), &sent);
    p.flush_selection(&send);
    let (title, body) = p.desktop.notice.clone().expect("a notice");
    assert_eq!(title, "Saved location unavailable");
    assert!(
        body.contains("gone.zip") && body.contains("archive 1 of 2"),
        "{body}"
    );
    assert_eq!(shown_mount(&p), Some(70));
    let current = p.panes[0].history_index.unwrap();
    assert!(matches!(
        &p.panes[0].history[current],
        HistoryEntry::Archive(location) if location.mount_id() == Some(70)
    ));
}

#[test]
fn a_restored_tab_inside_an_archive_inside_an_archive_opens_every_level() {
    let sent = RefCell::new(Vec::new());
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    let address = p.panes[0].archive.as_ref().unwrap().saved_address();
    assert_eq!(address.nested.len(), 1);
    // Before, a restored tab showed the enclosing archive's folder instead.
    let saved = PaneTabPreferences {
        paths: vec![test_root(), test_root()],
        active: 1,
        archives: [(1, address.clone())].into(),
        networks: Default::default(),
    };
    let mut p = Presenter::new(test_root(), test_root());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(midnight_core::AcceptedPreferences {
            preferences: UserPreferences {
                tabs: [saved, PaneTabPreferences::default()],
                ..Default::default()
            },
            comparators: midnight_core::FileComparatorSnapshot::default(),
        }))),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let restored = last_listing(&sent);
    assert!(restored.mount.is_none() && restored.address == address);
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        p.panes[0]
            .display_location_text()
            .ends_with(&format!("sample.zip{separator}inner.zip"))
    );
}
