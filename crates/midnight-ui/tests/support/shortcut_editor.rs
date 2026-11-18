//! Shortcut edits through the Options keyboard editor (`KeyboardEditorView`), the only
//! production path that changes key bindings.
//!
//! Select, Remove, Restore and Reset all act without an Options session, so they can
//! change bindings while a menu stays open. Assigning a new chord needs Options: it is
//! confirmed with OK and the settings save completes as the settings worker reports it.
#![allow(dead_code)]
use midnight_core::{Command, Event, PreferencesRevision};
use midnight_ui::{KeyboardEditorView, MainWindow, Presenter, refresh};
use slint::{ComponentHandle, Model, SharedString};
use std::{cell::RefCell, rc::Rc};

/// An installed Presenter with the command log an Options commit needs.
pub struct Shortcuts {
    pub presenter: Rc<RefCell<Presenter>>,
    pub commands: Rc<RefCell<Vec<Command>>>,
}

impl Shortcuts {
    /// Install `presenter` on `window`, recording every command it sends.
    pub fn install(window: &MainWindow, presenter: Presenter) -> Self {
        let presenter = Rc::new(RefCell::new(presenter));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        midnight_ui::install(
            window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        refresh(window, &presenter.borrow());
        Self {
            presenter,
            commands,
        }
    }

    /// Give `id` exactly `keys` in an Options session accepted with OK, confirming each
    /// replacement prompt, then complete that save.
    pub fn assign(&self, window: &MainWindow, id: &str, keys: &[&str]) {
        assert!(
            !window.get_options_open(),
            "assign opens its own Options session"
        );
        window.invoke_show_options();
        assert!(window.get_options_open());
        unassign(window, id);
        let view = window.global::<KeyboardEditorView>();
        for key in keys {
            view.invoke_capture_chord((*key).into());
            let candidate = view.get_candidate();
            assert!(!candidate.is_empty(), "{key} is not a shortcut");
            view.invoke_assign(id.into(), candidate.clone());
            let token = view.get_replacement_token();
            if !token.is_empty() {
                view.invoke_decide_replacement(token, true);
            }
            assert!(
                aliases(window).contains(&candidate),
                "{id} did not gain {candidate}: {}",
                view.get_issue()
            );
        }
        let sent = self.commands.borrow().len();
        // The callback of the Options OK button.
        window.invoke_options_accepted();
        let revision = self
            .saved_since(sent)
            .expect("OK saves the edited shortcuts");
        self.presenter.borrow_mut().apply(Event::PreferencesSaved {
            revision,
            result: Ok(()),
        });
        refresh(window, &self.presenter.borrow());
        // Run the change handlers that close the Options popup, as a frame does.
        slint::platform::update_timers_and_animations();
        assert!(
            !window.get_options_open(),
            "the saved Options session closes"
        );
    }

    fn saved_since(&self, sent: usize) -> Option<PreferencesRevision> {
        self.commands.borrow()[sent..]
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::SavePreferences { revision, .. } => Some(*revision),
                _ => None,
            })
    }
}

/// The aliases the keyboard editor shows for its selected command.
pub fn aliases(window: &MainWindow) -> Vec<SharedString> {
    window
        .global::<KeyboardEditorView>()
        .get_aliases()
        .iter()
        .collect()
}

/// Select `id` in the keyboard editor.
pub fn select(window: &MainWindow, id: &str) {
    let view = window.global::<KeyboardEditorView>();
    view.invoke_select(id.into());
    assert_eq!(
        view.get_selected_id(),
        id,
        "the keyboard editor selects {id}"
    );
}

/// Remove every alias of `id` with the editor's per-alias Remove.
pub fn unassign(window: &MainWindow, id: &str) {
    select(window, id);
    let view = window.global::<KeyboardEditorView>();
    for alias in aliases(window) {
        view.invoke_remove(id.into(), alias);
    }
    assert!(
        aliases(window).is_empty(),
        "{id} keeps an alias: {}",
        view.get_issue()
    );
}

/// The editor's Restore for one command.
pub fn restore(window: &MainWindow, id: &str) {
    let view = window.global::<KeyboardEditorView>();
    view.invoke_restore(id.into());
    assert!(view.get_issue().is_empty(), "{}", view.get_issue());
}

/// The editor's Reset all.
pub fn reset_all(window: &MainWindow) {
    let view = window.global::<KeyboardEditorView>();
    view.invoke_reset_all();
    assert!(view.get_issue().is_empty(), "{}", view.get_issue());
}
