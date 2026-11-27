//! The update check's button, dialog, About entry and Options choices in the real
//! window (docs/developer/updates.md). The update service is the application's; here
//! its state is handed to `show_update` as the application does, and its actions are
//! recorded. With `MIDNIGHT_UI_RENDER_DIR` set, every state is saved as a picture.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    items::AccessibleRole,
};
use midnight_core::Event;
use midnight_ui::{Presenter, UpdatePhase, UpdateState, UpdateView, install, refresh, show_update};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    actions: Rc<RefCell<Vec<&'static str>>>,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        install(&ui.window, presenter.clone(), Rc::new(|_| true));
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        refresh(&ui.window, &presenter.borrow());
        ui.window.set_application_version("0.5.0".into());
        // The application's update service replaces these; record what reaches it.
        let actions = Rc::new(RefCell::new(Vec::new()));
        let state = ui.window.global::<UpdateState>();
        for (name, slot) in [
            ("check-now", 0),
            ("download", 1),
            ("restart", 2),
            ("skip", 3),
        ] {
            let actions = actions.clone();
            let record = move || actions.borrow_mut().push(name);
            match slot {
                0 => state.on_check_now(record),
                1 => state.on_download(record),
                2 => state.on_restart(record),
                _ => state.on_skip(record),
            }
        }
        let mut h = Self {
            ui,
            presenter,
            actions,
        };
        h.ui.draw();
        h
    }

    fn show(&mut self, view: &UpdateView) {
        show_update(&self.ui.window, view);
        self.ui.draw();
    }

    fn state(&self) -> UpdateState<'_> {
        self.ui.window.global::<UpdateState>()
    }

    fn open(&mut self) {
        self.state().invoke_open_dialog(false);
        self.ui.draw();
        assert!(!self.state().get_token().is_empty());
    }

    fn has_button(&self, label: &str) -> bool {
        ui::try_popup_item(&self.ui.window, label, AccessibleRole::Button).is_some()
    }

    fn enabled(&self, label: &str) -> bool {
        ui::popup_item(&self.ui.window, label, AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_none_or(|value| value.as_str() == "true")
    }
}

fn view(phase: UpdatePhase) -> UpdateView {
    UpdateView {
        phase,
        current_version: "0.5.0".into(),
        latest_version: if matches!(
            phase,
            UpdatePhase::Idle | UpdatePhase::UpToDate | UpdatePhase::Checking
        ) {
            String::new()
        } else {
            "0.5.1".into()
        },
        notes: "What's new\n\n• Software update from the toolbar\n• Faster scrolling".into(),
        preview: true,
        published: "2026-11-23T10:00:00Z".into(),
        last_checked: Some(1_795_400_000),
        message: if phase == UpdatePhase::Failed {
            "GitHub cannot be reached (name not resolved).".into()
        } else {
            String::new()
        },
        received: 47 * 1024 * 1024,
        total: 131 * 1024 * 1024,
        page_url: "https://github.com/TomasJelinek/fm/releases/tag/v0.5.1".into(),
        ..UpdateView::default()
    }
}

#[test]
fn the_button_names_every_state_in_its_tooltip_and_accessible_name() {
    let mut h = Harness::new();
    for (phase, name) in [
        (UpdatePhase::Idle, "Check for updates"),
        (UpdatePhase::Checking, "Checking for updates…"),
        (
            UpdatePhase::UpToDate,
            "Midnight Explorer 0.5.0 is up to date",
        ),
        (UpdatePhase::Available, "Version 0.5.1 is available"),
        (UpdatePhase::Downloading, "Downloading version 0.5.1: 36%"),
        (UpdatePhase::Verifying, "Verifying version 0.5.1…"),
        (UpdatePhase::Ready, "Restart to update to version 0.5.1"),
        (
            UpdatePhase::Failed,
            "Update failed: GitHub cannot be reached (name not resolved).",
        ),
    ] {
        h.show(&view(phase));
        assert_eq!(h.state().get_status(), name, "{phase:?}");
        assert!(
            ui::try_root_item(&h.ui.window, name, AccessibleRole::Button).is_some(),
            "no toolbar button named {name:?}"
        );
        render::snapshot(&h.ui, &format!("toolbar-{phase:?}"));
    }
}

#[test]
fn the_dialog_offers_the_action_of_each_state() {
    let mut h = Harness::new();
    h.open();
    assert!(ui::has_popup(&h.ui.window));
    for (phase, primary, enabled) in [
        (UpdatePhase::Idle, "Check now", true),
        (UpdatePhase::Checking, "Checking…", false),
        (UpdatePhase::UpToDate, "Check now", true),
        (UpdatePhase::Available, "Download and install", true),
        (UpdatePhase::Downloading, "Downloading…", false),
        (UpdatePhase::Verifying, "Verifying…", false),
        (UpdatePhase::Ready, "Restart now", true),
        (UpdatePhase::Failed, "Check now", true),
    ] {
        h.show(&view(phase));
        assert!(h.has_button(primary), "{phase:?}: no {primary}");
        assert_eq!(h.enabled(primary), enabled, "{phase:?}: {primary}");
        assert_eq!(
            h.has_button("Skip this version"),
            phase == UpdatePhase::Available,
            "{phase:?}"
        );
        let dismiss = if matches!(
            phase,
            UpdatePhase::Available | UpdatePhase::Downloading | UpdatePhase::Ready
        ) {
            "Later"
        } else {
            "Close"
        };
        assert!(h.has_button(dismiss), "{phase:?}: no {dismiss}");
        render::snapshot(&h.ui, &format!("dialog-{phase:?}"));
    }
    // Notes and versions of an offer are in the dialog.
    h.show(&view(UpdatePhase::Available));
    let texts: Vec<String> = ui::visible_text_values(ui::popup_item(
        &h.ui.window,
        "Software update",
        AccessibleRole::Groupbox,
    ))
    .into_iter()
    .map(|text| text.to_string())
    .collect();
    assert!(
        texts
            .iter()
            .any(|text| text == "Version 0.5.1 is available."),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("• Faster scrolling")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("0.5.1 (preview), published 2026-11-23")),
        "{texts:?}"
    );
}

#[test]
fn the_actions_reach_the_update_service_and_escape_closes() {
    let mut h = Harness::new();
    h.show(&view(UpdatePhase::Available));
    h.open();
    ui::click_popup(&h.ui.window, "Download and install", AccessibleRole::Button);
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Skip this version", AccessibleRole::Button);
    h.ui.draw();
    h.show(&view(UpdatePhase::Ready));
    // The keyboard follows the new primary action: Enter restarts.
    ui::key(&h.ui.window, slint::platform::Key::Return);
    h.ui.draw();
    h.show(&view(UpdatePhase::Failed));
    ui::click_popup(&h.ui.window, "Check now", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(
        *h.actions.borrow(),
        ["download", "skip", "restart", "check-now"]
    );
    ui::key(&h.ui.window, slint::platform::Key::Escape);
    h.ui.draw();
    assert!(h.state().get_token().is_empty());
    assert!(!h.ui.window.get_embedded_modal_open());
}

#[test]
fn a_copy_that_cannot_install_offers_the_release_page() {
    let mut h = Harness::new();
    h.show(&UpdateView {
        blocked: "This build has no update signing key, so it cannot install updates. Download new versions from the release page.".into(),
        ..view(UpdatePhase::Available)
    });
    h.open();
    assert!(h.has_button("Open release page"));
    assert!(!h.has_button("Download and install"));
    render::snapshot(&h.ui, "dialog-blocked");
}

#[test]
fn a_windows_installer_copy_installs_with_install_now() {
    let mut h = Harness::new();
    h.open();
    for needs_approval in [false, true] {
        h.show(&UpdateView {
            windows_installer: true,
            needs_approval,
            ..view(UpdatePhase::Ready)
        });
        assert_eq!(h.state().get_status(), "Install version 0.5.1");
        assert!(h.has_button("Install now"));
        assert!(!h.has_button("Restart now"));
        let texts: Vec<String> = ui::visible_text_values(ui::popup_item(
            &h.ui.window,
            "Software update",
            AccessibleRole::Groupbox,
        ))
        .into_iter()
        .map(|text| text.to_string())
        .collect();
        let headline = texts
            .iter()
            .find(|text| text.starts_with("Version 0.5.1 is ready."))
            .unwrap_or_else(|| panic!("{texts:?}"));
        assert!(headline.contains("Windows Installer"), "{headline}");
        assert_eq!(
            headline.contains("administrator approval"),
            needs_approval,
            "{headline}"
        );
        render::snapshot(&h.ui, &format!("dialog-installer-{needs_approval}"));
    }
    ui::click_popup(&h.ui.window, "Install now", AccessibleRole::Button);
    h.ui.draw();
    assert_eq!(*h.actions.borrow(), ["restart"]);
    // Approval is asked only of a Windows Installer copy.
    h.show(&UpdateView {
        needs_approval: true,
        ..view(UpdatePhase::Ready)
    });
    assert!(h.has_button("Restart now"));
    assert!(!h.state().get_needs_approval());
}

#[test]
fn about_opens_the_update_dialog() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_about();
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Check for updates", AccessibleRole::Button);
    h.ui.draw();
    assert!(!h.ui.window.get_about_open());
    assert!(!h.state().get_token().is_empty());
}

#[test]
fn options_has_the_update_choices_on_by_default_in_its_draft() {
    let mut h = Harness::new();
    h.ui.window.invoke_show_options();
    h.ui.draw();
    h.ui.options_topic(0);
    for label in ["Check for updates at startup", "Include preview releases"] {
        let item = ui::popup_item(&h.ui.window, label, AccessibleRole::Checkbox);
        assert_eq!(
            item.accessible_string_property(AccessibleStringProperty::Checked)
                .unwrap()
                .as_str(),
            "true",
            "{label}"
        );
    }
    // The page scrolls; the check box's own action stands for a click on it.
    ui::popup_item(
        &h.ui.window,
        "Check for updates at startup",
        AccessibleRole::Checkbox,
    )
    .accessible_action(&AccessibilityAction::Default);
    h.ui.draw();
    assert!(
        !h.presenter
            .borrow()
            .loaded_update_preferences()
            .unwrap()
            .check_at_startup
    );
    render::snapshot(&h.ui, "options-updates");
    // Cancel restores the choice.
    ui::key(&h.ui.window, slint::platform::Key::Escape);
    h.ui.draw();
    assert!(
        h.presenter
            .borrow()
            .loaded_update_preferences()
            .unwrap()
            .check_at_startup
    );
    assert!(h.state().get_check_at_startup());
}
