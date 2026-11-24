//! Archives inside archives: Enter opens an archive member as an archive in the same
//! pane; Backspace, path parts and history move between the levels; the depth limit,
//! unreadable members and failed opens explain themselves; a failed inner session
//! returns to the folder that contains it.
use super::*;

#[path = "archive_history.rs"]
mod archive_history;
#[path = "archive_history_report.rs"]
mod archive_history_report;

/// An archive opened from a member of `parent`'s archive. The UI only passes it along.
#[derive(Debug)]
struct Inner(u64, ArchiveParent);
impl ArchiveMount for Inner {
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
    fn parent(&self) -> Option<ArchiveParent> {
        Some(self.1.clone())
    }
}

fn member(mount: u64, entry: u64, name: &str, kind: EntryKind, readable: bool) -> Entry {
    Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId { mount, entry },
        parent: 1,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind,
        size: Some(3 * 1024 * 1024),
        readable,
        access: Default::default(),
    })
}

/// What an archive root holds in these tests, listed through session `mount`.
fn root_entries(mount: u64) -> Vec<Entry> {
    vec![
        member(mount, 2, "folder", EntryKind::Directory, false),
        member(mount, 5, "inner.zip", EntryKind::File, true),
        member(mount, 6, "locked.zip", EntryKind::File, false),
        member(mount, 7, "notes.txt", EntryKind::File, true),
        member(mount, 8, "other.tar.gz", EntryKind::File, true),
    ]
}

fn listing(location: ArchiveLocation, entries: Vec<Entry>) -> ArchiveListing {
    let entries: Arc<[Entry]> = entries.into();
    ArchiveListing {
        location,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    }
}

/// The root of `sample.zip`, listed through the fixture's session (mount 42), with
/// ZIP and gzipped TAR providers installed.
fn outer_pane(sent: &RefCell<Vec<Command>>) -> Presenter {
    let mut p = archive_pane();
    p.apply(Event::ArchiveAvailability(vec![
        ArchiveFormat::Zip,
        ArchiveFormat::TarGzip,
    ]));
    let location = p.panes[0].archive.as_ref().unwrap().location.clone();
    assert!(p.refresh_location(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    listed(&mut p, Ok(listing(location, root_entries(42))), sent);
    p
}

/// The display row of `name`, as `open` counts rows.
fn row(p: &Presenter, name: &str) -> usize {
    let pane = &p.panes[0];
    (0..pane.entries.len())
        .find(|index| {
            pane.display_entry(*index)
                .is_some_and(|entry| entry.display_name == name)
        })
        .expect("a listed member")
        + pane.parent_offset()
}

fn last_open(sent: &RefCell<Vec<Command>>) -> Option<(NavigationId, ArchiveParent, ArchiveFormat)> {
    sent.borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::ArchiveNavigate {
                navigation,
                action: ArchiveAction::OpenMember { parent, format },
                ..
            } => Some((*navigation, parent.clone(), *format)),
            _ => None,
        })
}

/// The root of the archive opened from `parent`, through session `mount`.
fn inner_root(parent: &ArchiveParent, mount: u64) -> ArchiveLocation {
    ArchiveLocation {
        mount: Some(Arc::new(Inner(mount, parent.clone()))),
        address: ArchiveAddress {
            nested: Vec::new(),
            provider: "midnight.archive".into(),
            format: Some(ArchiveFormat::Zip),
            container: parent.location.address.container.clone(),
            source_version: vec![9, mount as u8],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, parent.name.clone())],
    }
}

/// Enter on `inner.zip` in the shown root, opened through session `mount`.
fn enter_inner(p: &mut Presenter, sent: &RefCell<Vec<Command>>, mount: u64) -> ArchiveParent {
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.open(PaneId::Left, row(p, "inner.zip"), &send));
    let (_, parent, _) = last_open(sent).expect("an archive inside the archive opens");
    listed(
        p,
        Ok(listing(inner_root(&parent, mount), root_entries(mount))),
        sent,
    );
    assert_eq!(
        p.panes[0].archive.as_ref().unwrap().location.mount_id(),
        Some(mount)
    );
    parent
}

#[test]
fn enter_opens_an_archive_member_inside_the_archive_and_backspace_returns_to_it() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    assert!(p.open(PaneId::Left, row(&p, "inner.zip"), &send));
    let (navigation, parent, format) = last_open(&sent).expect("an archive inside the archive");
    assert_eq!(format, ArchiveFormat::Zip);
    assert_eq!(
        parent.member,
        VirtualEntryId {
            mount: 42,
            entry: 5
        }
    );
    assert_eq!(parent.name, "inner.zip");
    assert_eq!(parent.location.mount_id(), Some(42));
    assert!(p.panes[0].loading);
    // Staging progress reaches the status bar until the listing arrives.
    p.apply(Event::ArchiveProgress {
        pane: PaneId::Left,
        navigation,
        staged: 1024 * 1024,
    });
    assert_eq!(p.desktop.status, "Opening inner.zip: 1 of 3 MiB…");
    listed(
        &mut p,
        Ok(listing(inner_root(&parent, 43), root_entries(43))),
        &sent,
    );
    assert!(
        p.desktop.status.is_empty(),
        "the progress ends with the open"
    );
    assert!(!p.panes[0].loading);
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        p.panes[0]
            .display_location_text()
            .ends_with(&format!("sample.zip{separator}inner.zip")),
        "{}",
        p.panes[0].display_location_text()
    );
    // The native folders that hold the outer archive come first; the inner archive's
    // name is the part a shortened path keeps whole.
    let parts = p.path_parts(PaneId::Left);
    let [.., outer, inner] = parts.as_slice() else {
        panic!("archive parts")
    };
    assert!(outer.label.ends_with("sample.zip"));
    assert_eq!(inner.label, format!("{separator}inner.zip"));
    assert!(inner.keep_whole && !outer.keep_whole);
    // Saved tabs and Hot Paths keep the whole path: the enclosing archive's revision and
    // folder, and the member that opens the archive inside it.
    let saved = p.panes[0].archive.as_ref().unwrap().saved_address();
    assert_eq!(saved.source_version, [1, 2, 3]);
    assert!(saved.components.is_empty());
    assert_eq!(saved.nested.len(), 1);
    assert_eq!(saved.nested[0].member, b"inner.zip");
    let bookmark = p
        .capture_bookmark_location(PaneId::Left, "inside".into())
        .unwrap();
    assert_eq!(bookmark.archive.unwrap(), saved);
    // Backspace at the inner root lists the enclosing folder, cursor on the archive.
    assert!(p.up(PaneId::Left, &send));
    let enclosing = last_listing(&sent);
    assert_eq!((enclosing.mount_id(), enclosing.directory), (Some(42), 1));
    listed(&mut p, Ok(listing(enclosing, root_entries(42))), &sent);
    assert_eq!(p.panes[0].cursor, Some(EntryId(5)));
    assert!(p.panes[0].display_location_text().ends_with("sample.zip"));
    // History goes back into the inner archive and out of it again.
    assert!(p.history(PaneId::Left, -1, &send));
    let back = last_listing(&sent);
    assert_eq!(back.mount_id(), Some(43));
    listed(&mut p, Ok(listing(back, root_entries(43))), &sent);
    assert!(p.history(PaneId::Left, 1, &send));
    assert_eq!(last_listing(&sent).mount_id(), Some(42));
}

#[test]
fn path_parts_reach_the_enclosing_folders_with_the_cursor_on_the_way_back() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    enter_inner(&mut p, &sent, 44);
    let separator = std::path::MAIN_SEPARATOR;
    assert!(p.panes[0].display_location_text().ends_with(&format!(
        "sample.zip{separator}inner.zip{separator}inner.zip"
    )));
    let parts = p.path_parts(PaneId::Left);
    let identities: HashSet<_> = parts.iter().map(|part| part.identity.clone()).collect();
    assert_eq!(
        identities.len(),
        parts.len(),
        "every part has its own identity"
    );
    // The outermost archive's part lists its root, cursor on the first inner archive.
    let outer = &parts[parts.len() - 3];
    assert!(outer.label.ends_with("sample.zip"));
    assert!(p.navigate_path_part(PaneId::Left, &outer.identity, &send));
    let outermost = last_listing(&sent);
    assert_eq!(outermost.mount_id(), Some(42));
    listed(&mut p, Ok(listing(outermost, root_entries(42))), &sent);
    assert_eq!(p.panes[0].cursor, Some(EntryId(5)));
}

#[test]
fn archives_inside_archives_stop_at_the_depth_limit_and_other_members_explain() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    for mount in 43..=45 {
        enter_inner(&mut p, &sent, mount);
    }
    assert_eq!(
        p.panes[0].archive.as_ref().unwrap().location.depth(),
        MAX_ARCHIVE_DEPTH
    );
    let opened = |sent: &RefCell<Vec<Command>>| {
        sent.borrow()
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ArchiveNavigate {
                        action: ArchiveAction::OpenMember { .. },
                        ..
                    }
                )
            })
            .count()
    };
    let before = opened(&sent);
    assert!(!p.open(PaneId::Left, row(&p, "inner.zip"), &send));
    assert_eq!(opened(&sent), before, "nothing opens past the limit");
    let (title, body) = p.desktop.notice.clone().expect("a notice");
    assert_eq!(title, "Archive nesting limit");
    assert!(body.contains("at most 4 levels deep"), "{body}");
    assert!(!p.open(PaneId::Left, row(&p, "locked.zip"), &send));
    let (title, body) = p.desktop.notice.clone().expect("a notice");
    assert_eq!(title, "Archive member");
    assert!(body.starts_with("locked.zip cannot be read"), "{body}");
    assert!(!p.open(PaneId::Left, row(&p, "notes.txt"), &send));
    let (_, body) = p.desktop.notice.clone().expect("a notice");
    assert!(body.contains("F3") && body.contains("F5"), "{body}");
    assert_eq!(opened(&sent), before);
    // Members named like enabled archives carry the archive badge when readable.
    let badge = |name: &str, readable: bool| {
        let mut entry = member(45, 9, name, EntryKind::File, readable);
        entry.archive_extensions = ArchiveExtensionHints::from_path(Path::new(name));
        p.can_browse_archive(&entry)
    };
    assert!(badge("inner.zip", true) && badge("data.tar.gz", true));
    assert!(!badge("locked.zip", false) && !badge("notes.txt", true));
}

#[test]
fn a_failed_open_keeps_the_enclosing_folder_and_says_why() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    assert!(p.open(PaneId::Left, row(&p, "inner.zip"), &send));
    let (navigation, ..) = last_open(&sent).unwrap();
    p.apply(Event::ArchiveProgress {
        pane: PaneId::Left,
        navigation,
        staged: 1,
    });
    let not_zip = FsError::from_io(
        "open archive",
        &test_root().join("sample.zip").join("inner.zip"),
        io::Error::new(
            io::ErrorKind::InvalidData,
            "This file is not a valid ZIP archive.",
        ),
    );
    listed(&mut p, Err(not_zip), &sent);
    let pane = &p.panes[0];
    assert!(!pane.loading && pane.entries.len() == 5);
    assert_eq!(pane.archive.as_ref().unwrap().location.mount_id(), Some(42));
    assert!(
        pane.error.contains("This file is not a valid ZIP archive."),
        "{}",
        pane.error
    );
    assert!(p.desktop.status.is_empty() && p.desktop.notice.is_none());
    let count = sent.borrow().len();
    p.recover_archive(PaneId::Left, &send);
    assert_eq!(
        sent.borrow().len(),
        count,
        "a failed open needs no recovery"
    );
}

#[test]
fn a_failed_inner_session_returns_to_its_enclosing_folder() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    // The inner session has stopped: listing one of its folders fails.
    assert!(p.open(PaneId::Left, row(&p, "folder"), &send));
    assert_eq!(last_listing(&sent).mount_id(), Some(43));
    listed(&mut p, Err(stopped_session()), &sent);
    let (title, body) = p.desktop.notice.clone().expect("a notice");
    assert_eq!(title, "Archive unavailable");
    assert!(body.contains("returns to the folder that contains the archive"));
    assert!(
        history::archives(&p.panes[0].history).all(|location| location.mount_id() == Some(42)),
        "the enclosing archive's session is kept, the stopped one is not entered again"
    );
    p.recover_archive(PaneId::Left, &send);
    let enclosing = last_listing(&sent);
    assert_eq!((enclosing.mount_id(), enclosing.directory), (Some(42), 1));
    listed(&mut p, Ok(listing(enclosing, root_entries(42))), &sent);
    assert_eq!(p.panes[0].cursor, Some(EntryId(5)));
    assert!(p.panes[0].display_location_text().ends_with("sample.zip"));
    // Enter stages the archive again through a fresh session.
    let staged = |sent: &RefCell<Vec<Command>>| {
        sent.borrow()
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ArchiveNavigate {
                        action: ArchiveAction::OpenMember { .. },
                        ..
                    }
                )
            })
            .count()
    };
    let before = staged(&sent);
    assert!(p.open(PaneId::Left, row(&p, "inner.zip"), &send));
    assert_eq!(staged(&sent), before + 1);
}

#[test]
fn inner_archives_are_reentered_through_their_session_and_left_ones_make_room() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let opens = |sent: &RefCell<Vec<Command>>| {
        sent.borrow()
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ArchiveNavigate {
                        action: ArchiveAction::OpenMember { .. },
                        ..
                    }
                )
            })
            .count()
    };
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    assert!(p.up(PaneId::Left, &send));
    let enclosing = last_listing(&sent);
    listed(&mut p, Ok(listing(enclosing, root_entries(42))), &sent);
    // Enter on the same member lists the remembered session's root: nothing is staged.
    let before = opens(&sent);
    assert!(p.open(PaneId::Left, row(&p, "inner.zip"), &send));
    assert_eq!(opens(&sent), before);
    let again = last_listing(&sent);
    assert_eq!((again.mount_id(), again.directory), (Some(43), 1));
    listed(&mut p, Ok(listing(again, root_entries(43))), &sent);
    assert!(p.up(PaneId::Left, &send));
    let enclosing = last_listing(&sent);
    listed(&mut p, Ok(listing(enclosing, root_entries(42))), &sent);
    // Another member opens a new session; the one this pane left goes first.
    drop(p.take_retired());
    assert!(p.open(PaneId::Left, row(&p, "other.tar.gz"), &send));
    let (_, parent, format) = last_open(&sent).unwrap();
    assert_eq!(
        (parent.name.as_str(), format),
        ("other.tar.gz", ArchiveFormat::TarGzip)
    );
    let pane = &p.panes[0];
    assert!(
        history::archives(&pane.history).all(|location| location.mount_id() == Some(42)),
        "the left session is no longer remembered"
    );
    assert!(pane.history_index.unwrap() < pane.history.len());
    assert!(
        !p.take_retired().is_empty(),
        "its folders leave the UI thread through retirement"
    );
}
