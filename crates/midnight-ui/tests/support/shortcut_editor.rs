//! Shortcut edits through the Options keyboard editor (`KeyboardEditorView`), the only
//! production path that changes key bindings.
//!
//! Select, Remove, Restore and Reset all act without an Options session, so they can
//! change bindings while a menu stays open; each such edit saves at once, and these
//! helpers complete that save as the settings worker reports it. Assigning a new chord
//! needs Options: it is confirmed with OK and that save is completed the same way.
#![allow(dead_code)]
use midnight_core::{Command, Event, PreferencesRevision};
use midnight_ui::{KeyboardEditorView, MainWindow, Presenter, refresh};
use slint::{ComponentHandle, Model, SharedString};
use std::{cell::RefCell, rc::Rc};

/// An installed Presenter with the command log its settings saves need.
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

    /// Give `id` exactly `keys` in an Options session accepted with OK, then complete
    /// that save. Each key must be free: a chord that another command uses would open
    /// the replacement prompt, and silently confirming it would strip that command.
    pub fn assign(&self, window: &MainWindow, id: &str, keys: &[&str]) {
        assert!(
            !window.get_options_open(),
            "assign opens its own Options session"
        );
        window.invoke_show_options();
        assert!(window.get_options_open());
        self.unassign(window, id);
        let view = window.global::<KeyboardEditorView>();
        for key in keys {
            view.invoke_capture_chord((*key).into());
            let candidate = view.get_candidate();
            assert!(!candidate.is_empty(), "{key} is not a shortcut");
            view.invoke_assign(id.into(), candidate.clone());
            assert!(
                view.get_replacement_token().is_empty(),
                "{candidate} is not free for {id}: {}",
                view.get_replacement_body()
            );
            assert!(
                aliases(window).contains(&candidate),
                "{id} did not gain {candidate}: {}",
                view.get_issue()
            );
        }
        let sent = self.commands.borrow().len();
        // The callback of the Options OK button.
        window.invoke_options_accepted();
        let [revision] = self.saves_since(sent)[..] else {
            panic!("OK saves the edited shortcuts once");
        };
        self.saved(window, revision);
        // Run the change handlers that close the Options popup, as a frame does.
        slint::platform::update_timers_and_animations();
        assert!(
            !window.get_options_open(),
            "the saved Options session closes"
        );
    }

    /// Remove every alias of `id` with the editor's per-alias Remove.
    pub fn unassign(&self, window: &MainWindow, id: &str) {
        select(window, id);
        let view = window.global::<KeyboardEditorView>();
        for alias in aliases(window) {
            self.edit(window, || view.invoke_remove(id.into(), alias));
        }
        assert!(
            aliases(window).is_empty(),
            "{id} keeps an alias: {}",
            view.get_issue()
        );
    }

    /// The editor's Restore for one command.
    pub fn restore(&self, window: &MainWindow, id: &str) {
        let view = window.global::<KeyboardEditorView>();
        self.edit(window, || view.invoke_restore(id.into()));
        assert!(view.get_issue().is_empty(), "{}", view.get_issue());
    }

    /// The editor's Reset all.
    pub fn reset_all(&self, window: &MainWindow) {
        let view = window.global::<KeyboardEditorView>();
        self.edit(window, || view.invoke_reset_all());
        assert!(view.get_issue().is_empty(), "{}", view.get_issue());
    }

    /// Run one editor edit. In an Options session it changes only the draft that OK
    /// saves; otherwise it saves at once, and that save completes here.
    fn edit(&self, window: &MainWindow, edit: impl FnOnce()) {
        let sent = self.commands.borrow().len();
        edit();
        let saves = self.saves_since(sent);
        if window.get_options_open() {
            assert!(saves.is_empty(), "an Options session saves only on OK");
        } else {
            let [revision] = saves[..] else {
                panic!(
                    "an edit outside Options saves once, not {} times",
                    saves.len()
                );
            };
            self.saved(window, revision);
        }
    }

    fn saves_since(&self, sent: usize) -> Vec<PreferencesRevision> {
        self.commands.borrow()[sent..]
            .iter()
            .filter_map(|command| match command {
                Command::SavePreferences { revision, .. } => Some(*revision),
                _ => None,
            })
            .collect()
    }

    /// Report a completed save as the settings worker does, then refresh.
    fn saved(&self, window: &MainWindow, revision: PreferencesRevision) {
        self.presenter.borrow_mut().apply(Event::PreferencesSaved {
            revision,
            result: Ok(midnight_core::FileComparatorSnapshot::default()),
        });
        refresh(window, &self.presenter.borrow());
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
