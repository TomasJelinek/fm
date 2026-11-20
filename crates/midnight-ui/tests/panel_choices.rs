//! Show hidden files and Select/Deselect by mask through the real window: keypad
//! keys, Ctrl+H, the F10 menu, each panel's View menu, Options and the dialog.
use crate::support::ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{PanelChoices, Presenter, ShortcutHints, install, refresh};
use slint::{ComponentHandle, Model, platform::Key};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let captured = sent.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        let mut h = Self {
            ui,
            presenter,
            sent,
        };
        h.event(Event::PreferencesLoaded(Ok(Box::default())));
        for (pane, navigation) in h.navigations() {
            let root = PathBuf::from(if pane == PaneId::Left {
                "left"
            } else {
                "right"
            });
            h.finish(pane, navigation, &root, 0);
        }
        h.sent.borrow_mut().clear();
        h
    }

    fn navigations(&self) -> Vec<(PaneId, NavigationId)> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane, navigation, ..
                }
                | Command::Refresh {
                    pane, navigation, ..
                } => Some((*pane, *navigation)),
                _ => None,
            })
            .collect()
    }

    fn finish(&mut self, pane: PaneId, navigation: NavigationId, root: &Path, hidden: usize) {
        let entries: Vec<_> = [
            ("a.txt", EntryKind::File),
            ("b.md", EntryKind::File),
            ("c.log", EntryKind::File),
            ("docs", EntryKind::Directory),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (name, kind))| Entry::new(EntryId(id as u64 + 1), root.join(name), kind))
        .collect();
        let index = Arc::new(DirectoryIndex {
            hidden,
            ..DirectoryIndex::from_entries(&entries)
        });
        self.event(Event::ListingFinished {
            pane,
            navigation,
            entries: entries.into(),
            index,
        });
    }

    fn event(&mut self, event: Event) {
        let sent = self.sent.clone();
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, &|command| {
                sent.borrow_mut().push(command);
                true
            });
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }

    fn take(&self) -> Vec<Command> {
        self.sent.take()
    }

    fn told(&self) -> Vec<bool> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::ShowHiddenEntries(show) => Some(*show),
                _ => None,
            })
            .collect()
    }
}

fn mask_requests(commands: &[Command]) -> Vec<(PaneId, MaskSelection)> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::Selection(request) => match &request.action {
                SelectionAction::Mask(mask) => Some((request.pane, (**mask).clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn keypad_plus_and_minus_open_the_mask_dialog_and_enter_marks_matching_files() {
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    let choices = window.global::<PanelChoices>();
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false));
    h.ui.draw();
    assert!(
        !choices.get_mask_token().is_empty(),
        "Num + opens Select files"
    );
    assert!(choices.get_mask_select());
    assert_eq!(choices.get_mask_pattern(), "*");
    assert!(ui::has_popup(&window));
    ui::popup_item(&window, "Selection mask", AccessibleRole::TextInput);
    // The mask field has focus with its text selected: typing replaces it.
    ui::key(&window, "*");
    ui::key(&window, ".");
    ui::key(&window, "t");
    ui::key(&window, "x");
    ui::key(&window, "t");
    ui::key(&window, Key::Return);
    h.ui.draw();
    assert!(
        choices.get_mask_token().is_empty(),
        "Enter selects and closes"
    );
    assert_eq!(
        mask_requests(&h.take()),
        [(
            PaneId::Left,
            MaskSelection {
                pattern: "*.txt".into(),
                select: true,
                folders: false,
            }
        )]
    );

    // Num − deselects with the remembered mask; Include folders is a check box.
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadSubtract".into(), false, false, false, false));
    h.ui.draw();
    assert!(!choices.get_mask_select());
    assert_eq!(choices.get_mask_pattern(), "*.txt");
    ui::click_popup(&window, "Include folders", AccessibleRole::Checkbox);
    ui::click_popup(&window, "OK", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        mask_requests(&h.take()),
        [(
            PaneId::Left,
            MaskSelection {
                pattern: "*.txt".into(),
                select: false,
                folders: true,
            }
        )]
    );

    // Escape cancels without selecting anything.
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false));
    h.ui.draw();
    ui::key(&window, Key::Escape);
    h.ui.draw();
    assert!(choices.get_mask_token().is_empty());
    assert!(mask_requests(&h.take()).is_empty());

    // F10 → Selection offers the same commands to the mouse.
    window.invoke_show_app_menu(2);
    h.ui.draw();
    ui::click_popup(&window, "Menu category", AccessibleRole::Combobox);
    for _ in 0..2 {
        ui::key(&window, Key::DownArrow);
    }
    ui::key(&window, Key::Return);
    h.ui.draw();
    for label in [
        "Select by mask…",
        "Deselect by mask…",
        "Invert, folders as in Select by mask",
    ] {
        ui::popup_item(&window, label, AccessibleRole::Button);
    }
    ui::click_popup(&window, "Deselect by mask…", AccessibleRole::Button);
    h.ui.draw();
    assert!(!choices.get_mask_token().is_empty());
    assert!(!choices.get_mask_select());
}

#[test]
fn control_equal_and_minus_open_the_mask_dialog_without_a_keypad() {
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    let choices = window.global::<PanelChoices>();
    // Menus show the key every keyboard has; their tooltips list all of them.
    let hints = window.global::<ShortcutHints>();
    assert_eq!(hints.invoke_key("selection.mask-select".into()), "Ctrl+=");
    assert_eq!(
        hints.invoke_keys("selection.mask-select".into()),
        "Ctrl+= / Num + / Ctrl++"
    );
    assert_eq!(hints.invoke_key("selection.mask-deselect".into()), "Ctrl+-");
    assert_eq!(
        hints.invoke_keys("selection.mask-deselect".into()),
        "Ctrl+- / Num -"
    );
    assert_eq!(hints.invoke_key("selection.mask-invert".into()), "Num *");

    window.invoke_focus_files();
    ui::chord(&window, &[Key::Control], "=");
    h.ui.draw();
    assert!(
        !choices.get_mask_token().is_empty(),
        "Ctrl+= opens Select files"
    );
    assert!(choices.get_mask_select());
    ui::key(&window, Key::Return);
    h.ui.draw();
    assert_eq!(
        mask_requests(&h.take()),
        [(
            PaneId::Left,
            MaskSelection {
                pattern: "*".into(),
                select: true,
                folders: false,
            }
        )]
    );

    window.invoke_focus_files();
    ui::chord(&window, &[Key::Control], "-");
    h.ui.draw();
    assert!(
        !choices.get_mask_token().is_empty(),
        "Ctrl+- opens Deselect files"
    );
    assert!(!choices.get_mask_select());
    ui::key(&window, Key::Escape);
    h.ui.draw();
    assert!(mask_requests(&h.take()).is_empty());
    assert!(
        !window.get_left_search_active(),
        "no filename search started"
    );
}

#[test]
fn keypad_star_inverts_with_the_mask_folder_choice_and_number_row_keys_search() {
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadMultiply".into(), false, false, false, false));
    let inverted = h.take().into_iter().find_map(|command| match command {
        Command::Selection(request) => Some(request.action),
        _ => None,
    });
    assert!(matches!(
        inverted,
        Some(SelectionAction::Bulk(SelectionOperation::Invert))
    ));
    // Select by mask with Include folders remembers the folder choice for Num *.
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false));
    h.ui.draw();
    ui::click_popup(&window, "Include folders", AccessibleRole::Checkbox);
    ui::click_popup(&window, "OK", AccessibleRole::Button);
    h.ui.draw();
    h.take();
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadMultiply".into(), false, false, false, false));
    let inverted = h.take().into_iter().find_map(|command| match command {
        Command::Selection(request) => Some(request.action),
        _ => None,
    });
    assert!(matches!(
        inverted,
        Some(SelectionAction::Bulk(SelectionOperation::InvertAll))
    ));

    // The number row's + reaches filename search as typed text.
    window.invoke_focus_files();
    ui::key(&window, "+");
    h.ui.draw();
    assert!(window.get_left_search_active());
    assert_eq!(window.get_left_search_query(), "+");
    assert!(window.global::<PanelChoices>().get_mask_token().is_empty());
}

#[test]
fn control_h_menus_and_options_switch_hidden_files_and_the_status_counts_them() {
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    let choices = window.global::<PanelChoices>();
    assert!(choices.get_show_hidden_files());
    window.invoke_focus_files();
    ui::chord(&window, &[Key::Control], "h");
    h.ui.draw();
    assert_eq!(h.told(), [false]);
    assert!(!choices.get_show_hidden_files());
    let rereads: Vec<_> = h.navigations();
    assert_eq!(rereads.len(), 2, "both panels are read again in place");
    for (pane, navigation) in rereads {
        let root = PathBuf::from(if pane == PaneId::Left {
            "left"
        } else {
            "right"
        });
        h.finish(pane, navigation, &root, 3);
    }
    assert!(
        window.get_left_status().contains("3 hidden"),
        "{}",
        window.get_left_status()
    );
    // The visible status line under the list carries the count as a field.
    let details = window.get_left_details();
    assert!(
        (0..details.row_count())
            .filter_map(|row| details.row_data(row))
            .any(|field| field.label == "Hidden" && field.value == "3 hidden"),
        "the status line shows 3 hidden"
    );
    h.take();

    // The View menu offers the opposite choice and reaches the same command.
    ui::click_root(&window, "Left panel view mode", AccessibleRole::Combobox);
    h.ui.draw();
    ui::click_popup(&window, "Show hidden files", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.told(), [true]);
    assert!(choices.get_show_hidden_files());
    h.take();

    // F10 → Panels and navigation.
    window.invoke_show_app_menu(2);
    h.ui.draw();
    ui::click_popup(&window, "Menu category", AccessibleRole::Combobox);
    for _ in 0..3 {
        ui::key(&window, Key::DownArrow);
    }
    ui::key(&window, Key::Return);
    h.ui.draw();
    ui::click_popup(&window, "Hide hidden files", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.told(), [false]);
    h.take();

    // Options › General previews the check box; Cancel restores the saved choice.
    window.set_options_topic_index(0);
    window.invoke_show_options();
    h.ui.draw();
    ui::click_popup(&window, "Show hidden files", AccessibleRole::Checkbox);
    h.ui.draw();
    assert_eq!(h.told(), [true]);
    assert!(
        !h.take()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. })),
        "Options previews until OK"
    );
    ui::click_popup(&window, "Cancel", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(h.told(), [false]);
    assert!(
        !choices.get_show_hidden_files(),
        "Cancel keeps the saved choice"
    );
}
