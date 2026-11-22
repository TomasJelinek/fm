//! The archive password dialog through the real window: typing, Enter, Escape, Show
//! password and the wrong-password message, at normal and minimum sizes. With the
//! Presenter installed, a dialog that F3 or Enter opened and the user dismissed (Escape,
//! Cancel or the header's ×) closes without a notice, opens nothing and gives the
//! keyboard back to the row it came from.
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{ArchivePasswords, NoticePresentation, Presenter, install, refresh};
use slint::{ComponentHandle, Model, SharedString, platform::Key};
use std::{cell::RefCell, rc::Rc, sync::Arc};

type Calls = Rc<RefCell<Vec<(String, String)>>>;

/// Open the dialog as the Presenter's bridge does, recording its callbacks.
fn open(ui: &ui::TestUi) -> (Calls, Rc<RefCell<Vec<String>>>) {
    let dialog = ui.window.global::<ArchivePasswords>();
    let submitted: Calls = Rc::default();
    let dismissed: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = submitted.clone();
    dialog.on_submit(move |token, password| {
        record
            .borrow_mut()
            .push((token.to_string(), password.to_string()));
    });
    let record = dismissed.clone();
    dialog.on_dismiss(move |token| record.borrow_mut().push(token.to_string()));
    dialog.set_archive("share.zip".into());
    dialog.set_purpose("Enter its password to view balcony.emc.".into());
    dialog.set_token("7".into());
    ui::settle_layout(ui.window.window());
    (submitted, dismissed)
}

fn type_text(ui: &ui::TestUi, text: &str) {
    for character in text.chars() {
        ui::key(&ui.window, SharedString::from(character.to_string()));
    }
}

#[test]
fn typing_and_enter_submit_the_password_and_escape_cancels() {
    let ui = ui::TestUi::new();
    let (submitted, dismissed) = open(&ui);
    assert!(ui::has_popup(&ui.window), "the dialog is a modal popup");
    ui::popup_item(&ui.window, "Password", AccessibleRole::TextInput);
    type_text(&ui, "héslo 🔑");
    ui::key(&ui.window, Key::Return);
    assert_eq!(
        submitted.borrow().as_slice(),
        [("7".to_owned(), "héslo 🔑".to_owned())]
    );
    ui::key(&ui.window, Key::Escape);
    assert_eq!(dismissed.borrow().as_slice(), ["7"]);
}

#[test]
fn a_wrong_password_shows_why_and_ok_waits_while_one_is_checked() {
    let ui = ui::TestUi::new();
    let (submitted, _) = open(&ui);
    let dialog = ui.window.global::<ArchivePasswords>();
    dialog.set_busy(true);
    type_text(&ui, "x");
    ui::key(&ui.window, Key::Return);
    assert!(submitted.borrow().is_empty(), "Enter waits for the answer");
    dialog.set_busy(false);
    dialog.set_error("Wrong password — try again.".into());
    dialog.set_attempt(1);
    ui::settle_layout(ui.window.window());
    assert!(
        ui::try_popup_item(
            &ui.window,
            "Wrong password — try again.",
            AccessibleRole::Text
        )
        .is_some(),
        "the reason is shown under the field"
    );
    // The refused text is selected: typing replaces it.
    type_text(&ui, "right");
    ui::key(&ui.window, Key::Return);
    assert_eq!(
        submitted
            .borrow()
            .last()
            .map(|(_, password)| password.as_str()),
        Some("right")
    );
}

#[test]
fn show_password_and_ok_work_by_pointer_and_the_dialog_fits_the_smallest_window() {
    let mut ui = ui::TestUi::new();
    ui.set_size(320, 200);
    let (submitted, dismissed) = open(&ui);
    type_text(&ui, "abc");
    ui::click_popup(&ui.window, "Show password", AccessibleRole::Checkbox);
    ui::click_popup(&ui.window, "OK", AccessibleRole::Button);
    assert_eq!(
        submitted.borrow().as_slice(),
        [("7".to_owned(), "abc".to_owned())]
    );
    // Dismissal stays reachable at 320 × 200.
    ui::click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert_eq!(dismissed.borrow().as_slice(), ["7"]);
}

/// A session of `share.zip` without its password yet; the UI only passes it along.
#[derive(Debug)]
struct Locked;

impl ArchiveMount for Locked {
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
        panic!("the UI validated an archive")
    }
}

fn member(entry: u64, name: &str, encryption: ArchiveEncryption) -> Entry {
    Entry::from_archive(ArchiveNode {
        id: VirtualEntryId { mount: 42, entry },
        parent: 1,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: Some(1368),
        modified: None,
        readable: true,
        access: ArchiveAccess {
            encryption,
            contains_encrypted: false,
            unreadable: None,
        },
    })
}

/// What asks for the archive's password.
#[derive(Clone, Copy, Debug)]
enum Asking {
    /// F3 on the encrypted member `balcony.emc`.
    View,
    /// Enter on `inner.zip`, an encrypted archive inside `share.zip`.
    OpenMember,
}

/// How the user dismisses the dialog.
#[derive(Clone, Copy, Debug)]
enum Dismissal {
    Escape,
    /// The footer's Cancel button.
    Cancel,
    /// The header's × button.
    Close,
}

/// The installed window, its left panel in `share.zip` (session 42): an encrypted member,
/// a plain one and an encrypted archive inside it, which the ZIP provider opens.
struct Share {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
    /// The viewers that F3 opened, as (pane, basic).
    viewed: Rc<RefCell<Vec<(i32, bool)>>>,
}

impl Share {
    fn new() -> Self {
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
        // F3 as the application wires it: the password first, the viewer once unlocked.
        let viewed = Rc::new(RefCell::new(Vec::new()));
        let (state, record, weak) = (presenter.clone(), viewed.clone(), ui.window.as_weak());
        ui.window.on_view_file(move |pane, basic| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let id = if pane == 0 {
                PaneId::Left
            } else {
                PaneId::Right
            };
            if state.borrow_mut().view_needs_archive_password(id, basic) {
                refresh(&window, &state.borrow());
            } else {
                record.borrow_mut().push((pane, basic));
            }
        });
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        presenter
            .borrow_mut()
            .apply(Event::ArchiveAvailability(vec![ArchiveFormat::Zip]));
        let navigation = sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .expect("the left panel is listed at start");
        // In name order, as the backend lists them: each member has a row below it.
        let entries: Arc<[Entry]> = vec![
            member(3, "balcony.emc", ArchiveEncryption::Aes256),
            member(5, "inner.zip", ArchiveEncryption::ZipCrypto),
            member(4, "readme.txt", ArchiveEncryption::None),
        ]
        .into();
        presenter.borrow_mut().apply_with_sender(
            Event::ArchiveListed {
                pane: PaneId::Left,
                navigation,
                result: Ok(ArchiveListing {
                    location: ArchiveLocation {
                        mount: Some(Arc::new(Locked)),
                        address: ArchiveAddress {
                            format: Some(ArchiveFormat::Zip),
                            provider: "midnight.archive".into(),
                            container: root.join("share.zip"),
                            source_version: vec![1],
                            components: vec![],
                        },
                        directory: 1,
                        trail: vec![(1, "share.zip".into())],
                    },
                    index: Arc::new(DirectoryIndex::from_entries(&entries)),
                    entries,
                }),
            },
            send.as_ref(),
        );
        refresh(&ui.window, &presenter.borrow());
        ui.draw();
        Self {
            ui,
            presenter,
            sent,
            viewed,
        }
    }

    /// Run due timers, change handlers and deferred focus restoration, then lay out.
    fn settle(&self) {
        for _ in 0..3 {
            ui::settle_layout(self.ui.window.window());
        }
    }

    /// The name in the left list's cursor row, as the window shows it.
    fn cursor(&self) -> Option<String> {
        let row = usize::try_from(self.ui.window.get_left_cursor()).ok()?;
        self.ui
            .window
            .get_left_rows()
            .row_data(row)
            .map(|row| row.name.to_string())
    }

    /// Click `name` in the left list, whose list then has the keyboard.
    fn focus(&self, name: &str) {
        let rows = self.ui.window.get_left_rows();
        let row = (0..rows.row_count())
            .find(|row| rows.row_data(*row).is_some_and(|data| data.name == name))
            .unwrap_or_else(|| panic!("no row {name}"));
        self.ui.window.invoke_activate(0);
        self.ui
            .window
            .invoke_row_click(0, i32::try_from(row).unwrap(), false, false);
        self.ui.window.invoke_focus_files();
        self.settle();
        assert_eq!(self.cursor().as_deref(), Some(name));
        assert_eq!(self.ui.window.get_focused_pane(), 0);
    }

    fn dismiss(&self, how: Dismissal) {
        let window = &self.ui.window;
        match how {
            Dismissal::Escape => ui::key(window, Key::Escape),
            Dismissal::Cancel => ui::click_popup(window, "Cancel", AccessibleRole::Button),
            Dismissal::Close => ui::click_popup(window, "Cancel dialog", AccessibleRole::Button),
        }
        self.settle();
    }

    /// Retire the archive session off the UI thread, as the application does.
    fn retire(self) {
        self.presenter.borrow_mut().retire_all();
        let retired = self.presenter.borrow_mut().take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }
}

/// `asking` asks for the archive's password and `how` dismisses the dialog: it closes
/// without a notice, no viewer opens, the inner archive does not open, no password is
/// sent, and the keyboard is back in the list with its cursor on the same row.
fn dismissed(asking: Asking, how: Dismissal) {
    let (name, key, purpose) = match asking {
        Asking::View => (
            "balcony.emc",
            Key::F3,
            "Enter its password to view balcony.emc.",
        ),
        Asking::OpenMember => (
            "inner.zip",
            Key::Return,
            "Enter its password to open inner.zip.",
        ),
    };
    let share = Share::new();
    {
        let window = &share.ui.window;
        let dialog = window.global::<ArchivePasswords>();
        let notice = window.global::<NoticePresentation>();
        share.focus(name);
        share.sent.borrow_mut().clear();
        ui::key(window, key);
        share.settle();
        assert!(
            !dialog.get_token().is_empty(),
            "{asking:?} asked for nothing"
        );
        assert_eq!(dialog.get_purpose(), purpose);
        share.dismiss(how);
        assert!(
            dialog.get_token().is_empty(),
            "{how:?} left the dialog open"
        );
        assert!(
            notice.get_token().is_empty(),
            "a notice followed {how:?}: {}: {}",
            notice.get_title(),
            notice.get_body()
        );
        assert!(!ui::has_popup(window), "a popup stayed after {how:?}");
        assert!(share.viewed.borrow().is_empty(), "a viewer opened");
        assert!(
            !share.sent.borrow().iter().any(|command| matches!(
                command,
                Command::ArchiveNavigate { .. } | Command::UnlockArchive { .. }
            )),
            "sent {:?}",
            share.sent.borrow()
        );
        assert_eq!(
            window.get_focused_pane(),
            0,
            "the keyboard is not back in the file list"
        );
        assert_eq!(share.cursor().as_deref(), Some(name), "the cursor moved");
        // The keyboard really is in the list: Down moves its cursor.
        ui::key(window, Key::DownArrow);
        share.settle();
        assert_ne!(share.cursor().as_deref(), Some(name), "Down");
    }
    share.retire();
}

#[test]
fn escape_closes_the_password_dialog_of_f3_silently() {
    dismissed(Asking::View, Dismissal::Escape);
}

#[test]
fn cancel_closes_the_password_dialog_of_f3_silently() {
    dismissed(Asking::View, Dismissal::Cancel);
}

#[test]
fn the_close_button_closes_the_password_dialog_of_f3_silently() {
    dismissed(Asking::View, Dismissal::Close);
}

#[test]
fn escape_closes_the_password_dialog_of_enter_silently() {
    dismissed(Asking::OpenMember, Dismissal::Escape);
}

#[test]
fn cancel_closes_the_password_dialog_of_enter_silently() {
    dismissed(Asking::OpenMember, Dismissal::Cancel);
}

#[test]
fn the_close_button_closes_the_password_dialog_of_enter_silently() {
    dismissed(Asking::OpenMember, Dismissal::Close);
}
