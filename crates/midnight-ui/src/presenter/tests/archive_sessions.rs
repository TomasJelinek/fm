//! A tab that leaves its archive for another one lets go of the old session and names it
//! as the session that the new one replaces (`Command::ArchiveNavigate::replaces`).
use super::*;
use std::rc::Rc;

/// The session of the archive that the left panel shows (`archives::archive_pane`).
const SHOWN: u64 = 42;

fn saved(name: &str) -> ArchiveAddress {
    ArchiveAddress {
        nested: Vec::new(),
        format: Some(ArchiveFormat::SevenZip),
        provider: "midnight.archive".into(),
        container: test_root().join(name),
        source_version: vec![7],
        components: vec![],
    }
}

/// The last archive navigation sent: its navigation, replaced session and action.
fn sent(commands: &RefCell<Vec<Command>>) -> (NavigationId, Option<u64>, ArchiveAction) {
    commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::ArchiveNavigate {
                navigation,
                replaces,
                action,
                ..
            } => Some((*navigation, *replaces, action.clone())),
            _ => None,
        })
        .expect("an archive navigation")
}

fn refuse(p: &mut Presenter, navigation: NavigationId, name: &str) {
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation,
        result: Err(FsError::from_io(
            "open archive",
            &test_root().join(name),
            io::Error::other("Two archives are already open; close one of them first"),
        )),
    });
}

#[test]
fn a_refused_archive_does_not_keep_the_tabs_old_archive_open() {
    let mut p = super::archives::archive_pane();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    // Go to file in another archive, as Find does.
    let other = ArchiveLocation::unavailable(saved("other.7z"));
    assert!(p.archive_navigate(PaneId::Left, ArchiveAction::List(other), &send));
    let (navigation, replaces, _) = sent(&commands);
    assert_eq!(
        replaces,
        Some(SHOWN),
        "the new archive takes the old one's place"
    );
    refuse(&mut p, navigation, "other.7z");
    assert!(p.panes[0].error.contains("Two archives are already open"));
    assert!(
        !p.panes[0].holds_archive_session(SHOWN),
        "the tab's history kept its previous archive open behind the refused one"
    );
    // The tab still knows its previous archive by its address: it opens it again.
    let history = &p.panes[0].history;
    assert!(super::history::archives(history).all(|location| location.mount.is_none()));
    assert!(
        super::history::archives(history)
            .any(|location| location.address.container == test_root().join("sample.zip"))
    );
}

#[test]
fn an_archive_that_a_tab_panel_or_viewer_still_shows_is_not_replaced() {
    for holder in ["tab", "panel", "viewer"] {
        let mut p = super::archives::archive_pane();
        match holder {
            // A duplicate of the tab stays in the background.
            "tab" => assert!(p.new_tab(PaneId::Left, &|_| true)),
            "panel" => p.panes[1].archive = p.panes[0].archive.clone(),
            _ => p.set_viewer_archives(Rc::new(|| vec![7, SHOWN])),
        }
        let commands = RefCell::new(Vec::new());
        let other = ArchiveLocation::unavailable(saved("other.7z"));
        assert!(
            p.archive_navigate(PaneId::Left, ArchiveAction::List(other), &|command| {
                commands.borrow_mut().push(command);
                true
            })
        );
        assert_eq!(sent(&commands).1, None, "{holder} still shows it");
    }
}

#[test]
fn a_hot_path_to_another_archive_replaces_the_tabs_archive() {
    let mut p = super::archives::archive_pane();
    let commands = RefCell::new(Vec::new());
    assert!(
        p.open_archive_address(PaneId::Left, saved("hot.7z"), &|command| {
            commands.borrow_mut().push(command);
            true
        })
    );
    let (_, replaces, action) = sent(&commands);
    assert_eq!(replaces, Some(SHOWN));
    assert!(matches!(action, ArchiveAction::List(location) if location.mount.is_none()));
}

#[test]
fn the_archive_opened_after_leaving_one_replaces_the_one_left() {
    let mut p = super::archives::archive_pane();
    p.apply(Event::ArchiveAvailability(vec![ArchiveFormat::SevenZip]));
    let directory = test_root().to_string_lossy().into_owned();
    let navigation = begin(&mut p, PaneId::Left, &directory);
    let next = test_root().join("next.7z");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(5), next.clone(), EntryKind::File)],
    );
    assert!(p.panes[0].archive.is_none(), "the tab left the archive");
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.open(PaneId::Left, p.panes[0].parent_offset(), &send));
    let (navigation, replaces, action) = sent(&commands);
    assert!(matches!(action, ArchiveAction::Open { path, .. } if path == next));
    assert_eq!(replaces, Some(SHOWN), "its slot may still be in teardown");
    refuse(&mut p, navigation, "next.7z");
    assert!(
        p.panes[0].archive.is_none(),
        "a refused open keeps the folder"
    );
    // Named once: a later archive does not wait for that session again.
    assert!(p.open(PaneId::Left, p.panes[0].parent_offset(), &send));
    assert_eq!(sent(&commands).1, None);
}

#[test]
fn listing_inside_the_same_session_replaces_nothing() {
    let mut p = super::archives::archive_pane();
    let commands = RefCell::new(Vec::new());
    assert!(p.open(PaneId::Left, 1, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    let (_, replaces, action) = sent(&commands);
    assert!(matches!(action, ArchiveAction::List(location) if location.mount_id() == Some(SHOWN)));
    assert_eq!(replaces, None);
    assert!(p.panes[0].holds_archive_session(SHOWN));
}
