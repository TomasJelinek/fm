//! A refused open offers to close an archive that a tab or viewer shows, then opens the
//! refused archive once more in its place (`archive_prompt`).
use super::*;
use std::rc::Rc;

/// An archive session that a test panel shows: only its identity matters.
#[derive(Debug)]
struct Shown(u64);
impl ArchiveMount for Shown {
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

/// The session of sample.zip, which the left panel shows (`archives::archive_pane`).
const LEFT: u64 = 42;
/// The session of x.7z (`show_right`).
const RIGHT: u64 = 77;

fn address(name: &str) -> ArchiveAddress {
    ArchiveAddress {
        format: Some(ArchiveFormat::SevenZip),
        provider: "midnight.archive".into(),
        container: test_root().join(name),
        source_version: vec![7],
        components: vec![],
    }
}

fn recorder() -> (Rc<RefCell<Vec<Command>>>, impl Fn(Command) -> bool) {
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    (commands, move |command| {
        sink.borrow_mut().push(command);
        true
    })
}

/// The right panel shows x.7z (session 77).
fn show_right(p: &mut Presenter) {
    let location = ArchiveLocation {
        mount: Some(Arc::new(Shown(RIGHT))),
        address: address("x.7z"),
        directory: 1,
        trail: vec![(1, "x.7z".into())],
    };
    assert!(p.archive_navigate(
        PaneId::Right,
        ArchiveAction::List(location.clone()),
        &|_| true
    ));
    p.apply(Event::ArchiveListed {
        pane: PaneId::Right,
        navigation: NavigationId(p.panes[1].generation),
        result: Ok(ArchiveListing {
            location,
            entries: Arc::from([]),
            index: Arc::default(),
        }),
    });
    assert!(p.panes[1].archive.is_some());
}

/// The left panel's `action` is refused: `sessions` are the archives that count.
fn refuse(p: &mut Presenter, action: ArchiveAction, sessions: &[(u64, &str)]) -> NavigationId {
    assert!(p.archive_navigate(PaneId::Left, action.clone(), &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ArchiveSessionsFull {
        pane: PaneId::Left,
        navigation,
        sessions: sessions
            .iter()
            .map(|(id, name)| ArchiveSession {
                id: *id,
                container: test_root().join(name),
            })
            .collect(),
        retry: action,
    });
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation,
        result: Err(FsError::from_io(
            "open archive",
            &test_root().join("b.7z"),
            io::Error::other("Two archives are already open; close one of them first"),
        )),
    });
    navigation
}

fn open_b() -> ArchiveAction {
    ArchiveAction::List(ArchiveLocation::unavailable(address("b.7z")))
}

/// The archive navigations sent: pane, replaced session and the archive's file name.
fn archive_navigations(commands: &RefCell<Vec<Command>>) -> Vec<(PaneId, Option<u64>, String)> {
    commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::ArchiveNavigate {
                pane,
                replaces,
                action,
                ..
            } => {
                let path = match action {
                    ArchiveAction::Open { path, .. } => path.clone(),
                    ArchiveAction::List(location) => location.address.container.clone(),
                    ArchiveAction::OpenMember { .. } => PathBuf::new(),
                };
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                Some((*pane, *replaces, name.into_owned()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_refused_open_offers_to_close_what_a_tab_or_viewer_shows() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    // The left tab let go of sample.zip for b.7z, but a job still reads sample.zip.
    refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    let (title, body) = p.desktop.notice.clone().expect("the offer is shown");
    assert_eq!(title, "Two archives are already open");
    assert_eq!(
        body,
        "Close one of them to open b.7z.\nsample.zip is in use by a running job."
    );
    assert_eq!(p.desktop.notice_choices, ["Close x.7z (right panel)"]);
    assert!(p.desktop.notice_cancel, "Close cancels the offer");
    assert!(!p.desktop.notice_error, "a question, not an error");
    assert_eq!(
        p.panel_error_text(PaneId::Left),
        "",
        "the offer replaces the panel's error line"
    );
}

#[test]
fn choosing_a_panel_takes_it_out_of_its_archive_then_opens_the_refused_one() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    let refused = refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    let (commands, send) = recorder();
    assert!(p.choose_notice(p.desktop.notice_generation, 0, &send));
    // The right panel goes to the folder that contains x.7z, with the cursor on it.
    let (navigation, folder) = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Right,
                navigation,
                path,
                ..
            } => Some((*navigation, path.clone())),
            _ => None,
        })
        .expect("the right panel leaves its archive");
    assert_eq!(folder, test_root());
    assert_eq!(
        p.panes[1].pending.as_ref().unwrap().wanted_cursor,
        Some(test_root().join("x.7z"))
    );
    assert!(
        archive_navigations(&commands).is_empty(),
        "b.7z waits until the right panel let go of x.7z"
    );
    finish(&mut p, PaneId::Right, navigation, vec![]);
    p.flush_selection(&send);
    assert_eq!(
        archive_navigations(&commands),
        [(PaneId::Left, Some(RIGHT), "b.7z".to_owned())],
        "b.7z opens once more, in x.7z's place"
    );
    assert_eq!(p.panes[0].generation, refused.0 + 1);
    p.flush_selection(&send);
    assert_eq!(
        archive_navigations(&commands).len(),
        1,
        "it is retried once"
    );
}

#[test]
fn choosing_a_background_tab_and_its_viewer_closes_both_and_retries_at_once() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    // The tab with sample.zip goes to the background behind a tab with a folder.
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let directory = test_root().to_string_lossy().into_owned();
    let navigation = begin(&mut p, PaneId::Left, &directory);
    finish(&mut p, PaneId::Left, navigation, vec![]);
    assert!(p.panes[0].archive.is_none());
    let closed = Rc::new(RefCell::new(Vec::new()));
    let observed = closed.clone();
    p.set_viewer_archives(Rc::new(|| vec![LEFT]));
    p.set_archive_viewer_closer(Rc::new(move |session| observed.borrow_mut().push(session)));
    let open = ArchiveAction::Open {
        path: test_root().join("b.7z"),
        format: ArchiveFormat::SevenZip,
    };
    refuse(&mut p, open, &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    assert_eq!(
        p.desktop.notice_choices,
        [
            "Close sample.zip (left panel, tab 1 and a viewer)",
            "Close x.7z (right panel)"
        ]
    );
    let (commands, send) = recorder();
    assert!(p.choose_notice(p.desktop.notice_generation, 0, &send));
    assert_eq!(*closed.borrow(), [LEFT], "its viewer closes");
    assert!(
        !p.parked_tabs_hold(LEFT),
        "the background tab left sample.zip"
    );
    let tabs = p.tabs(PaneId::Left);
    assert_eq!(
        tabs[0].path,
        test_root(),
        "it shows the folder of sample.zip"
    );
    assert!(p.panes[1].archive.is_some(), "the right panel keeps x.7z");
    assert_eq!(
        archive_navigations(&commands),
        [(PaneId::Left, Some(LEFT), "b.7z".to_owned())]
    );
}

#[test]
fn cancel_closes_nothing_and_opens_nothing() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    let generation = p.desktop.notice_generation;
    p.dismiss_diagnostic_notice(generation);
    let (commands, send) = recorder();
    assert!(!p.choose_notice(generation, 0, &send), "the offer is gone");
    p.flush_selection(&send);
    assert!(commands.borrow().is_empty());
    assert!(p.panes[1].archive.as_ref().unwrap().holds_session(RIGHT));
}

#[test]
fn archives_that_only_jobs_hold_are_named_without_an_action() {
    let mut p = super::archives::archive_pane();
    refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (99, "z.7z")]);
    let (_, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(
        body,
        "sample.zip and z.7z are in use by running jobs. Wait for them to finish or stop \
         them, then open b.7z again."
    );
    assert!(p.desktop.notice_choices.is_empty());
    assert!(!p.desktop.notice_cancel, "Close is the only answer");
    let (commands, send) = recorder();
    assert!(!p.choose_notice(p.desktop.notice_generation, 0, &send));
    assert!(commands.borrow().is_empty());
}

#[test]
fn a_retry_waiting_for_a_panel_is_dropped_when_its_tab_moves_on() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    let (commands, send) = recorder();
    assert!(p.choose_notice(p.desktop.notice_generation, 0, &send));
    let right = NavigationId(p.panes[1].generation);
    // The left panel goes elsewhere before the right one let go of x.7z.
    let directory = test_root().to_string_lossy().into_owned();
    let elsewhere = begin(&mut p, PaneId::Left, &directory);
    finish(&mut p, PaneId::Left, elsewhere, vec![]);
    finish(&mut p, PaneId::Right, right, vec![]);
    p.flush_selection(&send);
    assert!(archive_navigations(&commands).is_empty());
}

#[test]
fn an_old_offer_cannot_be_chosen() {
    let mut p = super::archives::archive_pane();
    show_right(&mut p);
    refuse(&mut p, open_b(), &[(LEFT, "sample.zip"), (RIGHT, "x.7z")]);
    let old = p.desktop.notice_generation;
    p.desktop.report("Another notice", "Later.".into());
    let (commands, send) = recorder();
    assert!(!p.choose_notice(old, 0, &send));
    assert!(commands.borrow().is_empty());
    assert!(p.panes[1].archive.is_some());
}
