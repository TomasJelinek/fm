//! Immutable bindings and display strings track overrides and command configuration.
use super::*;
use slint::SharedString;
use std::{cell::RefCell, rc::Rc};

#[path = "function_slots.rs"]
mod function_slots;

pub(crate) struct KeyboardBindings {
    pub(crate) map: ShortcutMap,
    pub(crate) revision: u64,
    pub(crate) revision_text: SharedString,
    pub(crate) help: SharedString,
    overrides: Vec<ShortcutBinding>,
    commands: std::sync::Weak<Vec<UserCommand>>,
    hints: HashMap<String, (SharedString, SharedString)>,
    editor_text: HashMap<String, String>,
    scopes: HashMap<String, String>,
    control_files: [crate::FunctionSlot; 12],
    control_search: [crate::FunctionSlot; 12],
}

impl KeyboardBindings {
    fn new(
        overrides: &[ShortcutBinding],
        commands: &Arc<Vec<UserCommand>>,
        revision: u64,
    ) -> Result<Self, String> {
        let map = ShortcutMap::with_user_commands(overrides, commands)?;
        let mut hints = HashMap::new();
        let mut editor_text = HashMap::new();
        let mut scopes = HashMap::new();
        let mut help = String::new();
        for action in map.definitions() {
            let keys = map.bindings(&action.id).expect("catalog action");
            let primary = keys
                .iter()
                .find(|key| {
                    !key.control
                        && !key.alt
                        && !key.shift
                        && !key.meta
                        && key.key.starts_with('F')
                        && key.key[1..].parse::<u8>().is_ok()
                })
                .or_else(|| keys.first());
            let primary = primary.map(display_key).unwrap_or_default();
            let aliases = keys.iter().map(display_key).collect::<Vec<_>>().join(" / ");
            let canonical = keys
                .iter()
                .map(ShortcutChord::display)
                .collect::<Vec<_>>()
                .join("\n");
            let scope = match keys.first() {
                Some(first)
                    if keys
                        .iter()
                        .any(|key| action.scope_for(key) != action.scope_for(first)) =>
                {
                    keys.iter()
                        .map(|key| {
                            format!("{} — {}", display_key(key), action.scope_for(key).label())
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                Some(first) => action.scope_for(first).label().to_owned(),
                None => action.scope.label().to_owned(),
            };
            help.push_str(&format!(
                "{} — {}\n{}\n\n",
                action.label,
                if aliases.is_empty() {
                    "Unassigned"
                } else {
                    &aliases
                },
                scope
            ));
            hints.insert(action.id.clone(), (primary.into(), aliases.into()));
            editor_text.insert(action.id.clone(), canonical);
            scopes.insert(action.id.clone(), scope);
        }
        Ok(Self {
            control_files: function_slots::build(&map, ShortcutContext::Files),
            control_search: function_slots::build(&map, ShortcutContext::Search),
            map,
            revision,
            help: help.into(),
            revision_text: revision.to_string().into(),
            overrides: overrides.to_vec(),
            commands: Arc::downgrade(commands),
            hints,
            editor_text,
            scopes,
        })
    }

    pub(crate) fn hint(&self, action: &str, all: bool) -> &str {
        self.hints
            .get(action)
            .map_or("", |(primary, aliases)| if all { aliases } else { primary })
    }

    pub(crate) fn editor_text(&self, action: &str) -> &str {
        self.editor_text.get(action).map_or("", String::as_str)
    }

    pub(crate) fn scope(&self, action: &str) -> &str {
        self.scopes.get(action).map_or("", String::as_str)
    }

    pub(crate) fn hint_owned(&self, action: &str, all: bool) -> SharedString {
        self.hints
            .get(action)
            .map_or_else(SharedString::default, |(primary, aliases)| {
                if all {
                    aliases.clone()
                } else {
                    primary.clone()
                }
            })
    }
}

fn display_key(key: &ShortcutChord) -> String {
    let mut key = key.clone();
    if let Some(digit) = key.key.strip_prefix("Digit") {
        key.key = digit.to_owned();
    } else {
        key.key = match key.key.as_str() {
            "Period" => ".",
            "Comma" => ",",
            "Plus" => "+",
            "Minus" => "-",
            "Equal" => "=",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            "Backslash" => "\\",
            "Slash" => "/",
            "Semicolon" => ";",
            "Quote" => "'",
            "Backquote" => "`",
            other => other,
        }
        .to_owned();
    }
    key.display()
}

pub(crate) struct KeyboardState {
    // UI callbacks clone this small cell, so layout never re-borrows Presenter.
    pub(crate) bindings: Rc<RefCell<Arc<KeyboardBindings>>>,
    pub(crate) snapshot: Arc<KeyboardBindings>,
    pub(crate) selected: usize,
    pub(crate) error: String,
    pub(crate) editor: super::keyboard_editor::ShortcutEditor,
    editor_revision: u64,
    pub(crate) editor_revision_text: SharedString,
}

impl Default for KeyboardState {
    fn default() -> Self {
        let snapshot = Arc::new(
            KeyboardBindings::new(&[], &Arc::default(), 0).expect("validated default shortcuts"),
        );
        Self {
            bindings: Rc::new(RefCell::new(snapshot.clone())),
            snapshot,
            selected: 0,
            error: String::new(),
            editor: Default::default(),
            editor_revision: 0,
            editor_revision_text: "0".into(),
        }
    }
}

impl KeyboardState {
    pub(super) fn sync(
        &mut self,
        overrides: &[ShortcutBinding],
        commands: &Arc<Vec<UserCommand>>,
    ) -> Result<(), String> {
        let same_commands = self.snapshot.commands.ptr_eq(&Arc::downgrade(commands))
            || (commands.is_empty()
                && self.snapshot.map.definitions().len() == shortcut_catalog().len());
        if overrides == self.snapshot.overrides && same_commands {
            return Ok(());
        }
        let selected_id = self.snapshot.map.definitions()[self.selected].id.clone();
        let snapshot = Arc::new(KeyboardBindings::new(
            overrides,
            commands,
            self.snapshot.revision.saturating_add(1),
        )?);
        self.selected = snapshot
            .map
            .definitions()
            .iter()
            .position(|action| action.id == selected_id)
            .unwrap_or_else(|| {
                self.editor.candidate = None;
                0
            });
        self.snapshot = snapshot.clone();
        *self.bindings.borrow_mut() = snapshot;
        self.reset_editor();
        Ok(())
    }

    pub(super) fn reset_editor(&mut self) {
        self.editor_revision = self.editor_revision.saturating_add(1);
        self.editor_revision_text = self.editor_revision.to_string().into();
    }
}

impl Presenter {
    pub(crate) fn shortcut_hint(&self, action: &str) -> &str {
        self.keyboard.snapshot.hint(action, true)
    }

    pub(crate) fn shortcut_key(&self, action: &str) -> &str {
        self.keyboard.snapshot.hint(action, false)
    }

    pub(crate) fn select_shortcut(&mut self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        if index < self.keyboard.snapshot.map.definitions().len() {
            self.keyboard.selected = index;
            self.keyboard.error.clear();
        }
    }

    pub(crate) fn change_shortcuts(
        &mut self,
        action: &str,
        keys: Vec<String>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self
            .keyboard
            .snapshot
            .map
            .definitions()
            .iter()
            .any(|definition| definition.id == action)
        {
            self.keyboard.error = "Choose an existing shortcut action.".into();
            return false;
        }
        let mut values = self.preferences.values.clone();
        super::keyboard_editor::replace_aliases(&mut values, action, keys);
        self.apply_shortcut_values(values, send)
    }

    pub(crate) fn restore_shortcut(
        &mut self,
        action: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self
            .keyboard
            .snapshot
            .map
            .definitions()
            .iter()
            .any(|definition| definition.id == action)
        {
            self.keyboard.error = "Choose an existing shortcut action.".into();
            return false;
        }
        let mut values = self.preferences.values.clone();
        values.shortcuts.retain(|entry| entry.action != action);
        self.apply_shortcut_values(values, send)
    }

    pub(crate) fn restore_shortcuts(&mut self, send: &dyn Fn(Command) -> bool) {
        let mut values = self.preferences.values.clone();
        values.shortcuts.clear();
        self.apply_shortcut_values(values, send);
    }

    fn apply_shortcut_values(
        &mut self,
        values: UserPreferences,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let previous_revision = self.keyboard.editor_revision;
        if let Err(error) = self.keyboard.sync(&values.shortcuts, &values.user_commands) {
            self.keyboard.error = error;
            return false;
        }
        // A successful restore must discard the draft even when defaults were
        // already effective. Keep that invalidation separate from the map cache.
        if self.keyboard.editor_revision == previous_revision {
            self.keyboard.reset_editor();
        }
        self.preferences.edited.shortcuts = true;
        self.keyboard.error.clear();
        self.keyboard.editor.candidate = None;
        self.update_preferences(values, send);
        true
    }
}
