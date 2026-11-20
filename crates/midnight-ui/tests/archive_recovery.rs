//! A panel inside an archive whose provider session was stopped still leaves it:
//! Backspace through the focused panel, events applied as the application does.
use crate::support::ui;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::platform::Key;
use std::{cell::RefCell, io, path::Path, rc::Rc, sync::Arc};

/// A provider session the UI only passes along.
#[derive(Debug)]
struct Session(u64);

impl ArchiveMount for Session {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("the UI listed an archive")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("the UI read an archive")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("the UI validated an archive")
    }
}

fn file(mount: u64, entry: u64, name: &str) -> Entry {
    Entry::from_archive(ArchiveNode {
        id: VirtualEntryId { mount, entry },
        parent: 1,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: Some(5),
        modified: None,
        readable: true,
    })
}

fn listing(location: ArchiveLocation, entries: Vec<Entry>) -> ArchiveListing {
    let entries: Arc<[Entry]> = entries.into();
    ArchiveListing {
        location,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    }
}

#[test]
fn backspace_leaves_an_archive_folder_whose_session_was_stopped() {
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
    let last_navigation = || {
        sent.borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                }
                | Command::ArchiveNavigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap()
    };
    let last_listing = || {
        sent.borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ArchiveNavigate {
                    action: ArchiveAction::List(location),
                    ..
                } => Some(location.clone()),
                _ => None,
            })
            .expect("an archive listing")
    };
    // The panel shows bundle.zip/bundle through session 42.
    let address = ArchiveAddress {
        format: None,
        provider: "midnight.archive".into(),
        container: root.join("bundle.zip"),
        source_version: vec![1],
        components: vec![b"bundle".to_vec()],
    };
    let apply = |event| {
        presenter
            .borrow_mut()
            .apply_with_sender(event, send.as_ref());
    };
    apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: last_navigation(),
        result: Ok(listing(
            ArchiveLocation {
                mount: Some(Arc::new(Session(42))),
                address: address.clone(),
                directory: 2,
                trail: vec![(1, "bundle.zip".into()), (2, "bundle".into())],
            },
            vec![file(42, 3, "readme.txt")],
        )),
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    ui.window.invoke_focus_files();
    // Backspace lists the archive root through session 42, which the provider's
    // watchdog has stopped meanwhile: the listing fails.
    ui::key(&ui.window, Key::Backspace);
    let parent = last_listing();
    assert_eq!(parent.mount_id(), Some(42));
    assert!(parent.address.components.is_empty());
    apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: last_navigation(),
        result: Err(FsError::from_io(
            "archive navigation",
            Path::new(""),
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Archive worker made no progress for 20 seconds and was stopped; open the archive again",
            ),
        )),
    });
    // The root is listed again through a fresh session, which opens.
    let reopened = last_listing();
    assert!(reopened.mount.is_none() && reopened.address.components.is_empty());
    apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: last_navigation(),
        result: Ok(listing(
            ArchiveLocation {
                mount: Some(Arc::new(Session(43))),
                address: reopened.address,
                directory: 1,
                trail: vec![(1, "bundle.zip".into())],
            },
            vec![file(43, 2, "notes.txt")],
        )),
    });
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    // The next Backspace returns to the native folder that contains the archive.
    let before = sent.borrow().len();
    ui::key(&ui.window, Key::Backspace);
    let native = sent.borrow()[before..]
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                path,
                ..
            } => Some(path.clone()),
            _ => None,
        });
    assert_eq!(native, Some(root.clone()));
    presenter.borrow_mut().retire_all();
    let retired = presenter.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}
