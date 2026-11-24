//! The archive password dialog: asked once before F3, Enter or F5 needs it; OK sends
//! the password to a backend worker. Escape, Cancel or the header's × close it without
//! a word: F3 and Enter open nothing, and a copy goes on without the password. A closure
//! the user did not choose says why.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "archive_password_targets.rs"]
mod archive_password_targets;

#[derive(Debug, Default)]
struct LockedMount {
    unlocked: AtomicBool,
    names_locked: bool,
}
impl ArchiveMount for LockedMount {
    fn id(&self) -> u64 {
        42
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
    fn unlocked(&self) -> bool {
        self.unlocked.load(Ordering::Acquire)
    }
    fn listing_locked(&self) -> bool {
        self.names_locked && !self.unlocked()
    }
}

fn member(id: u64, name: &str, kind: EntryKind, encryption: ArchiveEncryption) -> Entry {
    member_in(42, id, name, kind, encryption)
}

/// A member listed through session `mount`.
fn member_in(
    mount: u64,
    id: u64,
    name: &str,
    kind: EntryKind,
    encryption: ArchiveEncryption,
) -> Entry {
    Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId { mount, entry: id },
        parent: 1,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind,
        size: Some(1368),
        readable: kind == EntryKind::File,
        access: ArchiveAccess {
            encryption,
            contains_encrypted: kind == EntryKind::Directory,
            unreadable: None,
        },
    })
}

/// `share.zip`, listing `entries` through `mount`, in the left panel.
fn share_pane(mount: Arc<LockedMount>, entries: Vec<Entry>) -> Presenter {
    let mut p = native_left();
    enter_share(&mut p, mount, entries);
    p
}

/// The left panel in its native folder, with the viewer settings read.
fn native_left() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let path = p.panes[0].path.to_string_lossy().into_owned();
    let nav = begin(&mut p, PaneId::Left, &path);
    finish(&mut p, PaneId::Left, nav, vec![]);
    p
}

/// The left panel enters `share.zip`, which lists `entries` through `mount`.
fn enter_share(p: &mut Presenter, mount: Arc<LockedMount>, entries: Vec<Entry>) {
    let location = ArchiveLocation {
        mount: Some(mount),
        address: ArchiveAddress {
            nested: Vec::new(),
            format: Some(ArchiveFormat::Zip),
            provider: "midnight.archive".into(),
            container: test_root().join("share.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "share.zip".into())],
    };
    let entries: Arc<[Entry]> = entries.into();
    let nav = NavigationId(p.panes[0].generation + 1);
    p.panes[0].generation = nav.0;
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: nav,
        result: Ok(ArchiveListing {
            location,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
}

fn share() -> (Presenter, Arc<LockedMount>) {
    let mount = Arc::new(LockedMount::default());
    let p = share_pane(
        mount.clone(),
        vec![
            member(2, "Kyrandia", EntryKind::Directory, ArchiveEncryption::None),
            member(3, "balcony.emc", EntryKind::File, ArchiveEncryption::Aes256),
            member(4, "readme.txt", EntryKind::File, ArchiveEncryption::None),
        ],
    );
    (p, mount)
}

fn focus(p: &mut Presenter, id: u64) {
    p.panes[0].cursor = Some(EntryId(id));
    p.panes[0].parent_cursor = false;
}

/// OK in the dialog, collecting the commands it sends.
fn submit(p: &mut Presenter, password: &str) -> Vec<Command> {
    let sent = RefCell::new(Vec::new());
    p.submit_archive_password(ArchivePassword::new(password.into()), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
}

fn answer(p: &mut Presenter, request: DesktopRequestId, result: ArchiveUnlock) -> Vec<Command> {
    answer_with(p, request, Ok(result))
}

/// The worker's answer, or its failure, collecting the commands that follow it.
fn answer_with(
    p: &mut Presenter,
    request: DesktopRequestId,
    result: Result<ArchiveUnlock, FsError>,
) -> Vec<Command> {
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(Event::ArchiveUnlocked { request, result }, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
}

/// `share.zip` holding an encrypted member and an encrypted archive, which the installed
/// ZIP provider opens: Enter on it asks for `share.zip`'s password first.
fn nested_share() -> (Presenter, Arc<LockedMount>) {
    let mount = Arc::new(LockedMount::default());
    let mut p = share_pane(
        mount.clone(),
        vec![
            member(3, "balcony.emc", EntryKind::File, ArchiveEncryption::Aes256),
            member(
                5,
                "inner.zip",
                EntryKind::File,
                ArchiveEncryption::ZipCrypto,
            ),
        ],
    );
    p.apply(Event::ArchiveAvailability(vec![ArchiveFormat::Zip]));
    (p, mount)
}

/// Enter on the focused row, collecting the commands it sends.
fn enter(p: &mut Presenter) -> Vec<Command> {
    let sent = RefCell::new(Vec::new());
    p.activate_focused(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
}

/// Everything the window could say once the dialog has gone: a notice, the status,
/// error and background lines, and the panel's own error line.
#[derive(Debug, PartialEq)]
struct Said {
    notice: Option<(String, String)>,
    notices: i32,
    status: String,
    error: String,
    background: Option<String>,
    panel: String,
}

fn said(p: &Presenter) -> Said {
    Said {
        notice: p.desktop.notice.clone(),
        notices: p.desktop.notice_generation,
        status: p.desktop.status.clone(),
        error: p.desktop.error.clone(),
        background: p.desktop.background.clone(),
        panel: p.panes[0].error.clone(),
    }
}

/// The left panel is still the active one, its cursor on member `id`: the row whose list
/// the UI gives the keyboard back to once the dialog has closed.
fn assert_on_row(p: &Presenter, id: u64) {
    assert_eq!(p.active, PaneId::Left);
    assert_eq!(p.panes[0].cursor, Some(EntryId(id)));
    assert!(!p.panes[0].parent_cursor);
}

/// Escape, Cancel or the dialog's × (each reaches `cancel_archive_password`, through
/// `ArchivePasswords.dismiss`; it has no sender, so a dismissal sends nothing).
fn dismiss(p: &mut Presenter) {
    p.cancel_archive_password();
}

fn unlock_request(commands: &[Command], expected: &str) -> DesktopRequestId {
    let [
        Command::UnlockArchive {
            request, password, ..
        },
    ] = commands
    else {
        panic!("one unlock command: {commands:?}");
    };
    assert_eq!(password.as_bytes(), expected.as_bytes());
    let debug = format!("{commands:?}");
    assert!(
        !debug.contains(expected),
        "the password leaked into {debug}"
    );
    *request
}

#[test]
fn f3_on_an_encrypted_member_asks_once_retries_a_wrong_password_and_then_views_it() {
    let (mut p, mount) = share();
    focus(&mut p, 3);
    assert!(p.view_needs_archive_password(PaneId::Left, false));
    let dialog = &p.archive_password;
    assert!(dialog.open);
    assert_eq!(dialog.archive, "share.zip");
    assert!(dialog.purpose.contains("balcony.emc"), "{}", dialog.purpose);
    // An empty field says what is missing and sends nothing.
    assert!(submit(&mut p, "").is_empty());
    assert_eq!(p.archive_password.error, "Type the password first.");
    let first = unlock_request(&submit(&mut p, "špatně"), "špatně");
    // A second OK while the first is checked sends nothing.
    assert!(submit(&mut p, "again").is_empty());
    assert!(answer(&mut p, first, ArchiveUnlock::WrongPassword).is_empty());
    assert!(p.archive_password.open, "a wrong password keeps the dialog");
    assert_eq!(
        p.archive_password.error,
        crate::presenter::archive_password::WRONG_PASSWORD
    );
    assert_eq!(p.archive_password.attempt, 1);
    assert_eq!(p.take_archive_view(), None);
    let second = unlock_request(&submit(&mut p, "Kyrandia"), "Kyrandia");
    // A stale answer to the first password changes nothing.
    answer(&mut p, first, ArchiveUnlock::Unlocked);
    assert!(p.archive_password.open);
    mount.unlocked.store(true, Ordering::Release);
    answer(&mut p, second, ArchiveUnlock::Unlocked);
    assert!(!p.archive_password.open);
    assert_eq!(p.take_archive_view(), Some((PaneId::Left, false)));
    assert_eq!(p.take_archive_view(), None, "the viewer opens once");
    // Unlocked: F3 views at once.
    assert!(!p.view_needs_archive_password(PaneId::Left, false));
}

#[test]
fn f3_on_a_plain_member_or_folder_never_asks() {
    let (mut p, _) = share();
    focus(&mut p, 4);
    assert!(!p.view_needs_archive_password(PaneId::Left, false));
    focus(&mut p, 2);
    assert!(!p.view_needs_archive_password(PaneId::Left, false));
    assert!(!p.archive_password.open);
}

/// The user chose not to enter the password for F3 or Alt+F3: the dialog closes without
/// a word. No viewer opens, the member stays locked, and the cursor stays on its row.
#[test]
fn dismissing_the_password_for_f3_opens_nothing_and_says_nothing() {
    for basic in [false, true] {
        let (mut p, mount) = share();
        focus(&mut p, 3);
        let before = said(&p);
        assert!(p.view_needs_archive_password(PaneId::Left, basic));
        dismiss(&mut p);
        assert!(!p.archive_password.open);
        assert_eq!(p.take_archive_view(), None, "no viewer opens");
        assert_eq!(p.desktop.notice, None, "no notice follows the dismissal");
        assert_eq!(said(&p), before, "nothing else is said either");
        assert_on_row(&p, 3);
        assert!(!mount.unlocked(), "the member stays locked");
        // The next F3 asks again.
        assert!(p.view_needs_archive_password(PaneId::Left, basic));
    }
}

/// The user chose not to enter the password that Enter on an encrypted archive inside
/// the archive needs: the dialog closes without a word, the inner archive does not open
/// and the panel stays where it was, its cursor on that archive.
#[test]
fn dismissing_the_password_for_enter_on_an_inner_archive_opens_nothing_and_says_nothing() {
    let (mut p, mount) = nested_share();
    focus(&mut p, 5);
    let before = said(&p);
    assert!(
        enter(&mut p).is_empty(),
        "Enter asks before it opens anything"
    );
    assert!(p.archive_password.open);
    assert_eq!(
        p.archive_password.purpose,
        "Enter its password to open inner.zip."
    );
    dismiss(&mut p);
    assert!(!p.archive_password.open);
    assert_eq!(p.desktop.notice, None, "no notice follows the dismissal");
    assert_eq!(said(&p), before, "nothing else is said either");
    let shown = p.panes[0]
        .archive
        .as_ref()
        .map(|archive| (archive.location.mount_id(), archive.location.trail.len()));
    assert_eq!(shown, Some((Some(42), 1)), "share.zip stays shown");
    assert_on_row(&p, 5);
    assert!(!mount.unlocked(), "the archive stays locked");
    // The next Enter asks again.
    assert!(enter(&mut p).is_empty());
    assert!(p.archive_password.open);
}

/// A closure the user did not choose still says why, and opens nothing: the archive's
/// worker cannot decrypt the members, or the archive failed while it checked the
/// password (its worker was stopped).
#[test]
fn a_closure_the_user_did_not_choose_says_why_and_opens_nothing() {
    let stopped = || {
        FsError::from_io(
            "archive password",
            Path::new(""),
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Archive worker made no progress for 20 seconds and was stopped; open the \
                 archive again",
            ),
        )
    };
    for id in [3, 5] {
        for (result, reason) in [
            (Ok(ArchiveUnlock::Unsupported), "cannot decrypt"),
            (Err(stopped()), "was stopped; open the archive again"),
        ] {
            let (mut p, _) = nested_share();
            focus(&mut p, id);
            if id == 3 {
                assert!(p.view_needs_archive_password(PaneId::Left, false));
            } else {
                assert!(enter(&mut p).is_empty());
            }
            let request = unlock_request(&submit(&mut p, "Kyrandia"), "Kyrandia");
            let sent = answer_with(&mut p, request, result);
            assert!(!p.archive_password.open, "{reason}: the dialog closes");
            assert!(sent.is_empty(), "{reason}: nothing opens: {sent:?}");
            assert_eq!(p.take_archive_view(), None, "{reason}: no viewer opens");
            let (title, message) = p.desktop.notice.clone().expect("a notice says why");
            assert_eq!(title, "Archive password");
            assert!(message.contains(reason), "{message}");
            assert_on_row(&p, id);
        }
    }
}

#[test]
fn f5_of_a_folder_with_encrypted_members_asks_once_and_cancel_still_copies() {
    let (mut p, _) = share();
    focus(&mut p, 2);
    let send = |_: Command| true;
    p.request_copy_dialog(PaneId::Left, &send);
    assert!(
        p.archive_password.open,
        "the folder holds encrypted members"
    );
    assert!(
        p.copies.draft.is_none(),
        "the Copy dialog waits for the password"
    );
    p.cancel_archive_password();
    assert!(
        p.copies.draft.is_some(),
        "without the password the copy goes on and skips the protected members"
    );
}

#[test]
fn f5_after_unlock_or_of_plain_members_opens_the_copy_dialog_at_once() {
    let (mut p, _) = share();
    focus(&mut p, 4);
    let send = |_: Command| true;
    p.request_copy_dialog(PaneId::Left, &send);
    assert!(!p.archive_password.open);
    assert!(p.copies.draft.is_some());
    let (mut p, mount) = share();
    focus(&mut p, 3);
    p.request_copy_dialog(PaneId::Left, &send);
    assert!(p.archive_password.open);
    let request = unlock_request(&submit(&mut p, "Kyrandia"), "Kyrandia");
    mount.unlocked.store(true, Ordering::Release);
    answer(&mut p, request, ArchiveUnlock::Unlocked);
    assert!(!p.archive_password.open);
    assert!(p.copies.draft.is_some(), "unlocked: the Copy dialog opens");
}

#[test]
fn a_listing_whose_names_are_encrypted_asks_and_lists_again_after_unlock() {
    let mount = Arc::new(LockedMount {
        names_locked: true,
        ..LockedMount::default()
    });
    let mut p = share_pane(mount.clone(), vec![]);
    assert!(p.archive_password.open);
    assert!(p.archive_password.purpose.contains("names are encrypted"));
    let request = unlock_request(&submit(&mut p, "Zanthia"), "Zanthia");
    mount.unlocked.store(true, Ordering::Release);
    let commands = answer(&mut p, request, ArchiveUnlock::Unlocked);
    assert!(
        matches!(
            commands.as_slice(),
            [Command::ArchiveNavigate {
                action: ArchiveAction::List(_),
                ..
            }]
        ),
        "{commands:?}"
    );
}

#[test]
fn unsupported_and_failed_unlocks_close_the_dialog_and_say_why() {
    let (mut p, _) = share();
    p.request_archive_password(PaneId::Left);
    assert!(p.archive_password.open);
    let request = unlock_request(&submit(&mut p, "Malcolm"), "Malcolm");
    answer(&mut p, request, ArchiveUnlock::Unsupported);
    assert!(!p.archive_password.open);
    assert!(
        p.desktop
            .notice
            .clone()
            .unwrap()
            .1
            .contains("cannot decrypt")
    );
    p.request_archive_password(PaneId::Left);
    let request = unlock_request(&submit(&mut p, "Malcolm"), "Malcolm");
    answer(&mut p, request, ArchiveUnlock::NotEncrypted);
    assert!(
        p.desktop
            .notice
            .clone()
            .unwrap()
            .1
            .contains("has no password-protected members")
    );
}

#[test]
fn the_status_line_marks_encrypted_members_and_names_why_one_cannot_be_read() {
    let (mut p, _) = share();
    focus(&mut p, 3);
    let details = p.panel_details(PaneId::Left);
    let encrypted = details
        .iter()
        .find(|detail| detail.label == "Encrypted")
        .expect("an Encrypted detail");
    assert_eq!(encrypted.value, "AES-256");
    assert_eq!(
        details
            .iter()
            .find(|detail| detail.label == "Bytes")
            .unwrap()
            .value,
        p.number_locale.format_u64(1368)
    );
    focus(&mut p, 4);
    assert!(
        p.panel_details(PaneId::Left)
            .iter()
            .all(|detail| detail.label != "Encrypted")
    );
    let mut imploded = member(5, "old.bin", EntryKind::File, ArchiveEncryption::None);
    if let Some(node) = imploded.virtual_node.as_mut() {
        node.readable = false;
        node.access.unreadable = Some(ArchiveUnreadable::Compression("Implode".into()));
    }
    let node = imploded.virtual_node.as_deref().unwrap();
    assert_eq!(
        node.unreadable_reason(),
        "old.bin is compressed with Implode, which the archive plugin cannot decompress."
    );
}
