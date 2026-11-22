//! The password dialog's action stays bound to what it was opened for: the pane, the
//! archive session that it showed and the member, selection or folder that the action
//! named. A tool window can move that pane while the dialog is open (Find's Go to file):
//! OK then unlocks and opens nothing, and the status line says that the archive was
//! left. While the pane still shows the archive, the action takes what it captured,
//! never the row under the cursor now.
use super::*;
use crate::presenter::{copy_dialog::CopyDraftInput, find::FindForm};

/// What the status line says when OK finds that the panel has left the archive.
const LEFT_MEMBER: &str = "The archive was left; open the member again.";
const LEFT_COPY: &str = "The archive was left; copy the members again.";
const LEFT_FOLDER: &str = "The archive was left; open the folder again.";

/// The second session of `share.zip`, which Go to file of one of its members opens.
#[derive(Debug)]
struct Reopened;
impl ArchiveMount for Reopened {
    fn id(&self) -> u64 {
        43
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI performed listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI performed reading")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

/// A file as Find reports it.
fn found(path: PathBuf) -> FindEntry {
    FindEntry {
        path,
        kind: EntryKind::File,
        metadata: EntryMetadata::default(),
        identity: TransferIdentity {
            device: 1,
            object: [7; 16],
        },
        revision: FindRevision([0; 6]),
    }
}

/// `found.emc`, a member at the root of `share.zip`, as Find reports it.
fn found_member() -> (FindEntry, Arc<FindArchiveMember>) {
    let container = test_root().join("share.zip");
    let archive = Arc::new(FindArchiveMember {
        folder: ArchiveAddress {
            format: Some(ArchiveFormat::Zip),
            provider: "midnight.archive".into(),
            container: container.clone(),
            source_version: vec![1],
            components: vec![],
        },
        name: b"found.emc".to_vec(),
        container: found(container.clone()),
    });
    (found(container.join("found.emc")), archive)
}

/// A file in another native folder, as Find reports it.
fn found_elsewhere() -> PathBuf {
    test_root().join("elsewhere").join("found.txt")
}

/// The left panel's Find window, opened while the panel showed its native folder: it
/// found `entry` (a member of `share.zip` when `archive` names it).
fn find_window(
    p: &mut Presenter,
    entry: FindEntry,
    archive: Option<Arc<FindArchiveMember>>,
) -> FindSessionId {
    let id = p.create_find(PaneId::Left).expect("a Find window");
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update: FindUpdate::Batch(
            vec![Arc::new(FindResult {
                id: FindResultId(1),
                entry,
                duplicate_group: None,
                duplicate_group_size: 0,
                archive,
            })]
            .into(),
        ),
    })));
    id
}

/// `share.zip` (session 42) entered by the left panel after its Find window found
/// `entry` (inside `archive` when that names it): an encrypted member, a plain one and
/// an encrypted archive inside it.
fn share_with_find(
    entry: FindEntry,
    archive: Option<Arc<FindArchiveMember>>,
) -> (Presenter, Arc<LockedMount>, FindSessionId) {
    let mut p = native_left();
    let find = find_window(&mut p, entry, archive);
    let mount = Arc::new(LockedMount::default());
    enter_share(
        &mut p,
        mount.clone(),
        vec![
            member(3, "balcony.emc", EntryKind::File, ArchiveEncryption::Aes256),
            member(4, "readme.txt", EntryKind::File, ArchiveEncryption::None),
            member(
                5,
                "inner.zip",
                EntryKind::File,
                ArchiveEncryption::ZipCrypto,
            ),
        ],
    );
    p.apply(Event::ArchiveAvailability(vec![ArchiveFormat::Zip]));
    (p, mount, find)
}

/// Find's Go to file of its one result while the password dialog is open, as its window
/// does it: returns the navigation of the left panel that it sends.
fn go_to_file(p: &mut Presenter, find: FindSessionId) -> Command {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.reveal_find(find, "1:1", &send));
    let Some(Command::ValidateFindResult(request)) = sent.borrow_mut().pop() else {
        panic!("Go to file validates its result first");
    };
    let entry = p.finds.sessions[&find].results[0].entry.clone();
    p.apply_with_sender(
        Event::FindResultValidated(Box::new(FindValidationResult {
            request,
            result: Ok(Box::new(entry)),
        })),
        &send,
    );
    sent.into_inner()
        .into_iter()
        .find(|command| {
            matches!(
                command,
                Command::Navigate {
                    pane: PaneId::Left,
                    ..
                } | Command::ArchiveNavigate {
                    pane: PaneId::Left,
                    ..
                }
            )
        })
        .expect("Go to file moves the left panel")
}

/// The name of the row under the left panel's cursor.
fn cursor_name(p: &Presenter) -> Option<String> {
    let panel = &p.panes[0];
    panel
        .cursor
        .and_then(|id| panel.entry_by_id(id))
        .map(|entry| entry.display_name.clone())
}

/// Go to file of a file in another native folder: the left panel leaves `share.zip`
/// for that folder, its cursor on the found file.
fn leave_for_native_file(p: &mut Presenter, find: FindSessionId) {
    let Command::Navigate {
        pane: PaneId::Left,
        navigation,
        ..
    } = go_to_file(p, find)
    else {
        panic!("Go to file navigates the left panel");
    };
    let path = found_elsewhere();
    finish(
        p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(91), path, EntryKind::File)],
    );
    assert!(p.panes[0].archive.is_none(), "the panel left share.zip");
    assert_eq!(cursor_name(p).as_deref(), Some("found.txt"));
}

/// Go to file of a member of `share.zip`: the left panel lists its folder through a new
/// session of the archive (43), its cursor on that member.
fn reopen_for_member(p: &mut Presenter, find: FindSessionId) {
    let Command::ArchiveNavigate {
        pane: PaneId::Left,
        navigation,
        action: ArchiveAction::List(location),
        ..
    } = go_to_file(p, find)
    else {
        panic!("Go to file lists the member's archive folder");
    };
    let entries: Arc<[Entry]> = vec![
        member_in(
            43,
            3,
            "balcony.emc",
            EntryKind::File,
            ArchiveEncryption::Aes256,
        ),
        member_in(
            43,
            7,
            "found.emc",
            EntryKind::File,
            ArchiveEncryption::Aes256,
        ),
    ]
    .into();
    // As the application applies it: Find then puts the cursor on the member.
    p.apply_with_sender(
        Event::ArchiveListed {
            pane: PaneId::Left,
            navigation,
            result: Ok(ArchiveListing {
                location: ArchiveLocation {
                    mount: Some(Arc::new(Reopened)),
                    address: location.address,
                    directory: 1,
                    trail: vec![(1, "share.zip".into())],
                },
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            }),
        },
        &|_| true,
    );
    let shown = p.panes[0]
        .archive
        .as_ref()
        .and_then(|archive| archive.location.mount_id());
    assert_eq!(shown, Some(43), "a new session of share.zip");
    assert_eq!(cursor_name(p).as_deref(), Some("found.emc"));
}

/// The viewer that the UI opens once the dialog has closed: `take_archive_view`, then
/// the F3 request it makes (`view_command`).
fn view(p: &mut Presenter) -> Option<ViewerLaunch> {
    let (pane, basic) = p.take_archive_view()?;
    Some(
        p.view_command(pane, basic, &|_| true)
            .unwrap_or_else(|reason| panic!("the viewer request failed: {reason:?}")),
    )
}

/// OK with the right password and the worker's answer to what OK sent, as the dialog
/// and the archive's worker do: returns everything they sent and the viewer that the UI
/// opens then.
fn ok(p: &mut Presenter, mount: &LockedMount) -> (Vec<Command>, Option<ViewerLaunch>) {
    let mut sent = submit(p, "Kyrandia");
    let request = sent.iter().find_map(|command| match command {
        Command::UnlockArchive { request, .. } => Some(*request),
        _ => None,
    });
    if let Some(request) = request {
        mount.unlocked.store(true, Ordering::Release);
        sent.extend(answer(p, request, ArchiveUnlock::Unlocked));
    }
    let viewed = view(p);
    (sent, viewed)
}

/// The archive session, the focused member and the number of marked members of the
/// open Copy dialog.
fn copy_target(p: &Presenter) -> Option<(Option<u64>, Option<EntryId>, usize)> {
    let draft = p.copies.draft.as_ref()?;
    let CopyDraftInput::Archive(request) = &draft.input else {
        panic!("a Copy dialog of archive members");
    };
    Some((
        request.location.mount_id(),
        request.fallback,
        request.selected.len(),
    ))
}

/// What a viewer shows: its path, and the member and session of an archive viewer.
fn shown_by(launch: &ViewerLaunch) -> (PathBuf, Option<(u64, u64)>) {
    (
        launch.focused.clone(),
        launch
            .archive
            .as_ref()
            .map(|archive| (archive.mount.id(), archive.focused.entry)),
    )
}

/// The panel had left the archive: the dialog closed, nothing was unlocked, sent or
/// opened, and the status line says so once.
fn assert_left(p: &Presenter, sent: &[Command], viewed: Option<&ViewerLaunch>, message: &str) {
    assert_eq!(viewed.map(shown_by), None, "a viewer opened");
    assert_eq!(copy_target(p), None, "a Copy dialog opened");
    assert!(sent.is_empty(), "sent {sent:?}");
    assert!(!p.archive_password.open, "the dialog closed");
    assert_eq!(p.desktop.status, message);
    assert_eq!(p.desktop.notice, None, "the status line says it, once");
}

/// F3 asked for the password, then Find's Go to file took the panel to a file in another
/// folder: OK views nothing, never the found file now under the cursor.
#[test]
fn f3_whose_panel_went_to_a_found_file_views_nothing_after_ok() {
    let (mut p, mount, find) = share_with_find(found(found_elsewhere()), None);
    focus(&mut p, 3);
    assert!(p.view_needs_archive_password(PaneId::Left, false));
    leave_for_native_file(&mut p, find);
    let (sent, viewed) = ok(&mut p, &mount);
    assert_left(&p, &sent, viewed.as_ref(), LEFT_MEMBER);
}

/// The panel went to the found file while the archive's worker checked the password:
/// the accepted password opens nothing either.
#[test]
fn f3_whose_panel_moved_while_the_password_was_checked_views_nothing() {
    let (mut p, mount, find) = share_with_find(found(found_elsewhere()), None);
    focus(&mut p, 3);
    assert!(p.view_needs_archive_password(PaneId::Left, false));
    let request = unlock_request(&submit(&mut p, "Kyrandia"), "Kyrandia");
    leave_for_native_file(&mut p, find);
    mount.unlocked.store(true, Ordering::Release);
    let sent = answer(&mut p, request, ArchiveUnlock::Unlocked);
    let viewed = view(&mut p);
    assert_left(&p, &sent, viewed.as_ref(), LEFT_MEMBER);
}

/// While the panel still shows the archive, the member that F3 (here Alt+F3) named
/// opens after OK, although the cursor has moved to another row meanwhile.
#[test]
fn the_member_that_f3_named_opens_after_ok_wherever_the_cursor_is() {
    let (mut p, mount) = share();
    focus(&mut p, 3);
    assert!(p.view_needs_archive_password(PaneId::Left, true));
    focus(&mut p, 4);
    let (_, viewed) = ok(&mut p, &mount);
    let launch = viewed.expect("a viewer opens");
    assert_eq!(shown_by(&launch).1, Some((42, 3)), "balcony.emc opens");
    assert!(launch.basic, "as Alt+F3 asked");
}

/// F5 asked for the password, then Find's Go to file listed a member of the same
/// archive through a new session, its cursor on that member: OK copies nothing.
#[test]
fn f5_whose_panel_went_to_a_found_member_copies_nothing_after_ok() {
    let (entry, archive) = found_member();
    let (mut p, mount, find) = share_with_find(entry, Some(archive));
    focus(&mut p, 3);
    p.request_copy_dialog(PaneId::Left, &|_| true);
    assert!(p.archive_password.open);
    reopen_for_member(&mut p, find);
    let (sent, viewed) = ok(&mut p, &mount);
    assert_left(&p, &sent, viewed.as_ref(), LEFT_COPY);
}

/// While the panel still shows the archive, the Copy dialog names what F5 named,
/// although the cursor has moved to another row meanwhile.
#[test]
fn the_selection_that_f5_named_is_copied_after_ok_wherever_the_cursor_is() {
    let (mut p, mount) = share();
    focus(&mut p, 3);
    p.request_copy_dialog(PaneId::Left, &|_| true);
    assert!(p.archive_password.open);
    focus(&mut p, 4);
    ok(&mut p, &mount);
    assert_eq!(copy_target(&p), Some((Some(42), Some(EntryId(3)), 0)));
}

/// Enter on an encrypted archive inside the archive asked for the password, then Find's
/// Go to file took the panel to a file in another folder: OK opens nothing.
#[test]
fn enter_whose_panel_went_to_a_found_file_opens_nothing_after_ok() {
    let (mut p, mount, find) = share_with_find(found(found_elsewhere()), None);
    focus(&mut p, 5);
    assert!(enter(&mut p).is_empty(), "Enter asks first");
    assert!(p.archive_password.open);
    leave_for_native_file(&mut p, find);
    let (sent, viewed) = ok(&mut p, &mount);
    assert_left(&p, &sent, viewed.as_ref(), LEFT_MEMBER);
}

/// While the panel still shows the archive, OK opens the archive that Enter named, from
/// the folder that holds it.
#[test]
fn enter_opens_the_archive_it_named_after_ok() {
    let (mut p, mount) = nested_share();
    focus(&mut p, 5);
    assert!(enter(&mut p).is_empty(), "Enter asks first");
    let (sent, _) = ok(&mut p, &mount);
    let opened = sent.iter().find_map(|command| match command {
        Command::ArchiveNavigate {
            pane: PaneId::Left,
            action: ArchiveAction::OpenMember { parent, .. },
            ..
        } => Some((
            parent.location.mount_id(),
            parent.location.directory,
            parent.member.entry,
        )),
        _ => None,
    });
    assert_eq!(opened, Some((Some(42), 1, 5)));
}

/// `share.zip`'s names are encrypted, so its listing asked for the password; then Find's
/// Go to file took the panel to a file in another folder. OK lists nothing, which would
/// take the panel back from where Go to file left it.
#[test]
fn a_locked_listing_whose_panel_went_to_a_found_file_is_not_listed_after_ok() {
    let mut p = native_left();
    let find = find_window(&mut p, found(found_elsewhere()), None);
    let mount = Arc::new(LockedMount {
        names_locked: true,
        ..LockedMount::default()
    });
    enter_share(&mut p, mount.clone(), vec![]);
    assert!(p.archive_password.open, "the listing asks");
    leave_for_native_file(&mut p, find);
    let (sent, viewed) = ok(&mut p, &mount);
    assert_left(&p, &sent, viewed.as_ref(), LEFT_FOLDER);
}
