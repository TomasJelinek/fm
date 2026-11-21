//! The path bar takes the path of a file, typed or pasted as Windows "Copy as path"
//! writes it: Enter lists the file's folder with the cursor on the file. The first test
//! answers the real path editor's requests with scripted backend events; the second
//! runs the production Engine on native files.
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{Model, platform::Key};
use std::{
    cell::{Cell, RefCell},
    path::{MAIN_SEPARATOR, Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const NAME: &str = "ünï 文档 🧪 e\u{301}.txt";

struct Scripted {
    ui: ui::TestUi,
    /// The 25 ms paste timer (src/path_paste.rs) follows this clock only.
    clock: Rc<Cell<Duration>>,
    p: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Scripted {
    fn new(start: PathBuf) -> Self {
        let (ui, clock) = ui::TestUi::with_virtual_clock();
        let p = Rc::new(RefCell::new(Presenter::new(
            start.clone(),
            start.join("right"),
        )));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let commands = sent.clone();
        install(
            &ui.window,
            p.clone(),
            Rc::new(move |command| {
                commands.borrow_mut().push(command);
                true
            }),
        );
        p.borrow_mut().apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences: UserPreferences::default(),
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
        refresh(&ui.window, &p.borrow());
        Self { ui, clock, p, sent }
    }

    /// Run due timers in 10 ms steps past the paste timer, as frames would.
    fn settle(&mut self) {
        for _ in 0..5 {
            self.clock.set(self.clock.get() + Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
            self.ui.draw();
        }
    }

    /// Deliver a backend event as the application does.
    fn answer(&mut self, event: Event) {
        let sent = self.sent.clone();
        self.p.borrow_mut().apply_with_sender(event, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        refresh(&self.ui.window, &self.p.borrow());
        self.settle();
    }

    /// The latest left request for `path`.
    fn navigation(&self, path: &Path) -> NavigationId {
        self.sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    path: requested,
                    ..
                } if requested == path => Some(*navigation),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no request for {}: {:?}", path.display(), self.sent))
    }
}

#[test]
fn a_pasted_file_path_lists_its_folder_with_the_cursor_on_the_file() {
    let base = std::env::temp_dir().join("path bar file 资料");
    let folder = base.join("Příliš 文件夹");
    let file = folder.join(NAME);
    let mut h = Scripted::new(base);
    // Ctrl+G opens the left path editor; Ctrl+A and Ctrl+V replace its text.
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], "g");
    h.settle();
    ui::chord(&h.ui.window, &[Key::Control], "a");
    h.sent.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "v");
    let request = h
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ReadPathClipboardText { request } => Some(*request),
            _ => None,
        })
        .expect("Paste reads the clipboard through the backend");
    // "Copy as path" text, ended with the separator that the backend's Paste adds.
    let pasted = format!("\"{}\"{MAIN_SEPARATOR}", file.to_str().unwrap());
    h.p.borrow_mut().apply(Event::PathClipboardTextRead {
        request,
        result: Ok(Arc::from(pasted.as_str())),
    });
    refresh(&h.ui.window, &h.p.borrow());
    h.settle();
    assert_eq!(h.ui.window.get_left_path_draft(), pasted.as_str());

    ui::key(&h.ui.window, Key::Return);
    h.settle();
    let typed = h.navigation(&file);
    h.answer(Event::ListingNotAFolder {
        pane: PaneId::Left,
        navigation: typed,
        parent: folder.clone(),
        entry: file.clone(),
        hidden: false,
    });
    h.answer(Event::ListingFallback {
        pane: PaneId::Left,
        navigation: typed,
        path: folder.clone(),
        error: FsError::from_io(
            "read directory",
            &file,
            std::io::ErrorKind::NotADirectory.into(),
        ),
    });
    let listed = h.navigation(&folder);
    assert!(listed > typed);
    // 150 files before the named one: its row 151 (after `..`) starts out of view.
    let names: Vec<String> = (0..150)
        .map(|index| format!("a-{index:03}.txt"))
        .chain([NAME.to_owned(), "z.txt".to_owned()])
        .collect();
    let entries: Vec<Entry> = names
        .iter()
        .zip(1..)
        .map(|(name, id)| Entry::new(EntryId(id), folder.join(name), EntryKind::File))
        .collect();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    h.answer(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: listed,
        entries: entries.into(),
        index,
    });

    assert_eq!(h.ui.window.get_left_path(), folder.to_str().unwrap());
    assert_eq!(h.ui.window.get_left_cursor(), 151);
    let row = h.ui.window.get_left_rows().row_data(151).unwrap();
    assert_eq!(row.name, NAME);
    assert!(
        !row.selected,
        "the cursor focuses the file without selecting it"
    );
    let (start, end) = (
        h.ui.window.get_left_visible_row_start(),
        h.ui.window.get_left_visible_row_end(),
    );
    assert!(
        start <= 151 && 151 < end,
        "row 151 scrolled into {start}..{end}"
    );
    assert!(h.ui.window.get_left_error().is_empty());
    let notice = h.ui.window.get_message_title();
    assert!(notice.is_empty(), "{notice}");
}

/// The name on the left panel's cursor row.
fn cursor_name(h: &native::Harness) -> Option<String> {
    let cursor = usize::try_from(h.cursor(PaneId::Left)).ok()?;
    let row = h.ui.window.get_left_rows().row_data(cursor)?;
    Some(row.name.to_string())
}

#[test]
fn a_typed_file_path_lists_its_folder_with_the_cursor_on_the_file_through_the_engine() {
    // Key input composes a combining sequence (e + U+0301 becomes é), which would
    // name another file; Paste inserts it unchanged (the scripted test above).
    const TYPED: &str = "ünï 文档 🧪.txt";
    let mut h = native::Harness::new();
    let folder = h.root.join("Příliš 文件夹");
    std::fs::create_dir(&folder).unwrap();
    for name in ["a.txt", TYPED, "z.txt"] {
        std::fs::write(folder.join(name), name).unwrap();
    }
    let file = folder.join(TYPED);
    h.ui.window.invoke_activate(0);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], "g");
    h.settle();
    ui::chord(&h.ui.window, &[Key::Control], "a");
    // As Windows "Copy as path" writes it, with stray spaces around.
    let typed = format!(" \"{}\" ", file.to_str().unwrap());
    ui::key(&h.ui.window, typed.as_str());
    assert_eq!(h.ui.window.get_left_path_draft(), typed.as_str());
    ui::key(&h.ui.window, Key::Return);
    let shown = folder.to_str().unwrap().to_owned();
    // A failure notice ends the wait at once.
    h.until(|h| {
        !h.ui.window.get_message_title().is_empty()
            || (h.ui.window.get_left_path().as_str() == shown
                && !h.ui.window.get_left_loading()
                && cursor_name(h).as_deref() == Some(TYPED))
    });
    let notice = h.ui.window.get_message_title();
    assert!(
        notice.is_empty(),
        "{notice}: {} {}",
        h.ui.window.get_message_body(),
        h.ui.window.get_message_path()
    );
    assert_eq!(h.ui.window.get_left_path().as_str(), shown);
    assert_eq!(cursor_name(&h).as_deref(), Some(TYPED));
    assert!(h.ui.window.get_left_error().is_empty());
}
