//! Show hidden files and Select/Deselect by mask through the real window: keypad
//! keys, Ctrl+H, the F10 menu, each panel's View menu, Options and the dialog.
#[path = "support/render.rs"]
mod render;
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

    /// The backend's answer to the mask dialog's last OK, which asked it to compile
    /// the mask (`Command::ValidateSelectionMask`).
    fn answer_mask_check(&mut self, result: Result<(), &str>) {
        let request = self
            .sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ValidateSelectionMask(request) => Some(request.clone()),
                _ => None,
            })
            .expect("OK has the backend check the mask");
        self.event(Event::SelectionMaskValidated {
            request,
            result: result.map_err(str::to_owned),
        });
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
    // The mask is the common edit: with the keyboard and text it shows its inline ×.
    h.ui.draw();
    render::snapshot(&h.ui, "select-mask-common-edit");
    ui::key(&window, Key::Return);
    h.ui.draw();
    assert!(
        !choices.get_mask_token().is_empty() && mask_requests(&h.sent.borrow()).is_empty(),
        "Enter has the backend check the mask first"
    );
    h.answer_mask_check(Ok(()));
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
    h.answer_mask_check(Ok(()));
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
fn a_mask_the_backend_refuses_keeps_the_dialog_open_with_its_reason() {
    const REASON: &str =
        "This mask is too large to match names with. Use fewer ? wildcards or shorter masks.";
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    let choices = window.global::<PanelChoices>();
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false));
    h.ui.draw();
    ui::key(&window, "?");
    ui::key(&window, Key::Return);
    h.ui.draw();
    h.answer_mask_check(Err(REASON));
    assert!(
        !choices.get_mask_token().is_empty() && ui::has_popup(&window),
        "the dialog stays open"
    );
    assert_eq!(choices.get_mask_error(), REASON, "and says why");
    let sent = h.take();
    assert!(mask_requests(&sent).is_empty(), "nothing is selected");
    assert!(
        !sent
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. })),
        "nor remembered"
    );
    // Escape still closes it without selecting anything, and the dialog opens again
    // with the mask remembered before.
    ui::key(&window, Key::Escape);
    h.ui.draw();
    assert!(choices.get_mask_token().is_empty());
    assert!(mask_requests(&h.take()).is_empty());
    window.invoke_focus_files();
    assert!(window.invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false));
    h.ui.draw();
    assert_eq!(choices.get_mask_pattern(), "*");
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
    h.answer_mask_check(Ok(()));
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
    h.answer_mask_check(Ok(()));
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

/// The View menu lists the views in shortcut order (Alt+2 Brief, Alt+3 Detailed, Alt+5
/// Thumbnails), and each still selects its own view.
#[test]
fn the_view_menu_lists_brief_detailed_thumbnails_and_each_selects_its_view() {
    let mut h = Harness::new();
    let window = h.ui.window.clone_strong();
    let open_menu = |h: &mut Harness| {
        ui::click_root(&window, "Left panel view mode", AccessibleRole::Combobox);
        h.ui.draw();
    };
    open_menu(&mut h);
    let top = |label: &str| {
        let item = ui::popup_item(&window, label, AccessibleRole::Button);
        item.map_to_native_window(item.geometry().origin).y
    };
    let (brief, detailed, thumbnails) = (top("Brief"), top("Detailed"), top("Thumbnails"));
    assert!(
        brief < detailed && detailed < thumbnails,
        "Brief {brief}, Detailed {detailed}, Thumbnails {thumbnails}"
    );
    ui::key(&window, Key::Escape);
    h.ui.draw();
    for (label, mode) in [("Brief", 1), ("Thumbnails", 2), ("Detailed", 0)] {
        open_menu(&mut h);
        ui::click_popup(&window, label, AccessibleRole::Button);
        h.ui.draw();
        assert_eq!(window.get_left_view_mode(), mode, "{label}");
    }
}
