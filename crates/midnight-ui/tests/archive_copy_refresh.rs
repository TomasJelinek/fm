//! Manual measurement (WIN-8): what one UI refresh costs while a panel shows an archive
//! folder of 100,001 selected members, as when a selection of them is copied out. The
//! application refreshes once per drained run of engine events.
//! `cargo test -p midnight-ui --test integration archive_copy_refresh -- --ignored --nocapture`
use crate::support::ui;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Session;

impl ArchiveMount for Session {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("the UI listed an archive")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("the UI read an archive")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[test]
#[ignore = "manual measurement"]
fn refreshing_a_panel_of_100001_selected_archive_members() {
    const COUNT: u64 = 100_001;
    let mut ui = ui::TestUi::new();
    let root = std::env::temp_dir();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
        let sent = sent.clone();
        move |command| {
            sent.borrow_mut().push(command);
            true
        }
    });
    install(&ui.window, presenter.clone(), send.clone());
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let navigation = sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .expect("the left panel's first navigation");
    let entries: Arc<[Entry]> = (0..COUNT)
        .map(|index| {
            Entry::from_archive(ArchiveNode {
                id: VirtualEntryId {
                    mount: 42,
                    entry: index + 2,
                },
                parent: 1,
                raw_name: format!("{index:06}.txt").into_bytes().into(),
                display_name: format!("{index:06}.txt"),
                kind: EntryKind::File,
                size: Some(1),
                modified: None,
                readable: true,
                access: Default::default(),
            })
        })
        .collect::<Vec<_>>()
        .into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply_with_sender(
        Event::ArchiveListed {
            pane: PaneId::Left,
            navigation,
            result: Ok(ArchiveListing {
                location: ArchiveLocation {
                    mount: Some(Arc::new(Session)),
                    address: ArchiveAddress {
                        format: None,
                        provider: "midnight.archive".into(),
                        container: root.join("flat.tar"),
                        source_version: vec![1],
                        components: vec![],
                    },
                    directory: 1,
                    trail: vec![(1, "flat.tar".into())],
                },
                index: index.clone(),
                entries: entries.clone(),
            }),
        },
        send.as_ref(),
    );
    refresh(&ui.window, &presenter.borrow());
    // Ctrl+A, answered as the backend answers it: every member selected.
    ui.window.invoke_focus_files();
    ui::chord(&ui.window, &[slint::platform::Key::Control], "a");
    let (navigation, revision, view) = sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::ArchiveSelect {
                navigation,
                revision,
                view,
                ..
            } => Some((*navigation, *revision, view.clone())),
            _ => None,
        })
        .expect("Ctrl+A selects through the backend");
    let selected: HashSet<EntryId> = entries.iter().map(|entry| entry.id).collect();
    let mut totals = ArchiveSelectionTotals::default();
    for entry in entries.iter() {
        totals.change(entry.kind, entry.metadata.size, true);
    }
    presenter.borrow_mut().apply_with_sender(
        Event::ArchiveSelection {
            pane: PaneId::Left,
            navigation,
            revision,
            view,
            result: Ok(ArchiveSelectionResult {
                selected: Arc::new(selected),
                totals,
            }),
        },
        send.as_ref(),
    );
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let mut refreshes = Duration::ZERO;
    let mut draws = Duration::ZERO;
    const ROUNDS: u32 = 20;
    for _ in 0..ROUNDS {
        let started = Instant::now();
        refresh(&ui.window, &presenter.borrow());
        refreshes += started.elapsed();
        let started = Instant::now();
        ui.draw();
        draws += started.elapsed();
    }
    println!(
        "PERF UI: one refresh of a panel of {COUNT} selected archive members takes {:.1} ms, \
         one draw {:.1} ms",
        refreshes.as_secs_f64() * 1000.0 / f64::from(ROUNDS),
        draws.as_secs_f64() * 1000.0 / f64::from(ROUNDS)
    );
}
