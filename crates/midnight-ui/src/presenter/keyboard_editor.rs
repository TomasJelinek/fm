//! Bounded keyboard-editor presentation; persistence uses the existing preference transaction.
use super::*;

#[derive(Default)]
pub(crate) struct ShortcutEditor {
    pub(crate) filter: String,
    pub(crate) candidate: Option<ShortcutChord>,
    pub(crate) replacement: Option<ShortcutReplacement>,
    replacement_sequence: u64,
}

pub(crate) struct ShortcutReplacement {
    token: String,
    body: String,
    action: String,
    chord: ShortcutChord,
    map: Arc<keyboard::KeyboardBindings>,
    editor_revision: String,
}

struct Assignment {
    values: UserPreferences,
    owners: Vec<String>,
}

const CONTEXTS: [ShortcutContext; 9] = [
    ShortcutContext::Files,
    ShortcutContext::Search,
    ShortcutContext::Command,
    ShortcutContext::Other,
    ShortcutContext::Volume,
    ShortcutContext::Viewer,
    ShortcutContext::ViewerSearch,
    ShortcutContext::Comparison,
    ShortcutContext::ComparisonSearch,
];

pub(crate) struct ShortcutInspection {
    pub(crate) scope: String,
    pub(crate) used_by: String,
    pub(crate) issue: String,
    pub(crate) can_assign: bool,
}

pub(crate) fn matches_filter(action: &ShortcutDefinition, filter: &str) -> bool {
    let label = action.label.to_lowercase();
    let id = action.id.to_lowercase();
    filter
        .to_lowercase()
        .split_whitespace()
        .all(|part| label.contains(part) || id.contains(part))
}

pub(super) fn replace_aliases(values: &mut UserPreferences, action: &str, keys: Vec<String>) {
    if let Some(binding) = values
        .shortcuts
        .iter_mut()
        .find(|binding| binding.action == action)
    {
        binding.keys = keys;
    } else {
        values.shortcuts.push(ShortcutBinding {
            action: action.into(),
            keys,
        });
    }
}

impl Presenter {
    pub(crate) fn move_shortcut_command(&mut self, step: i32) {
        let rows = self
            .keyboard
            .snapshot
            .map
            .definitions()
            .iter()
            .enumerate()
            .filter(|(_, action)| matches_filter(action, &self.keyboard.editor.filter))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|index| *index == self.keyboard.selected)
            .map_or(if step >= 0 { -1 } else { rows.len() as i32 }, |index| {
                index as i32
            });
        let next = current.saturating_add(step).clamp(0, rows.len() as i32 - 1);
        self.select_shortcut(rows[next as usize] as i32);
    }

    pub(crate) fn filter_shortcut_commands(&mut self, mut query: String) {
        query.truncate(query.floor_char_boundary(4096));
        if self.keyboard.editor.filter != query {
            self.keyboard.editor.filter = query;
            self.keyboard.reset_editor();
        }
    }

    pub(crate) fn capture_editor_shortcut(&mut self, chord: ShortcutChord) {
        if self.keyboard.editor.candidate.as_ref() != Some(&chord) {
            self.keyboard.editor.candidate = Some(chord);
            self.keyboard.error.clear();
            self.keyboard.reset_editor();
        }
    }

    pub(crate) fn clear_editor_shortcut(&mut self) {
        if self.keyboard.editor.candidate.take().is_some() {
            self.keyboard.error.clear();
            self.keyboard.reset_editor();
        }
    }

    pub(crate) fn inspect_editor_shortcut(&self) -> ShortcutInspection {
        let action = &self.keyboard.snapshot.map.definitions()[self.keyboard.selected];
        let mut inspection = ShortcutInspection {
            scope: action.scope.label().into(),
            used_by: String::new(),
            issue: String::new(),
            can_assign: false,
        };
        let Some(chord) = &self.keyboard.editor.candidate else {
            return inspection;
        };
        let scope = action.scope_for(chord);
        inspection.scope = scope.label().into();
        let map = &self.keyboard.snapshot.map;
        let mut seen = HashSet::new();
        let mut owners = Vec::new();
        for context in CONTEXTS {
            if let Some(owner) = map.resolve(chord, context)
                && seen.insert(owner.id.as_str())
            {
                owners.push(format!(
                    "{} — {}",
                    owner.label,
                    owner.scope_for(chord).label()
                ));
            }
        }
        inspection.used_by = if owners.is_empty() {
            "Unassigned".into()
        } else {
            owners.join("\n")
        };
        match self.shortcut_assignment() {
            Ok(_) => inspection.can_assign = true,
            Err(error) => inspection.issue = error,
        }
        inspection
    }

    fn shortcut_assignment(&self) -> Result<Assignment, String> {
        let map = &self.keyboard.snapshot.map;
        let action = &map.definitions()[self.keyboard.selected];
        let chord = self
            .keyboard
            .editor
            .candidate
            .as_ref()
            .ok_or("Capture a shortcut first.")?;
        if !matches_filter(action, &self.keyboard.editor.filter) {
            return Err("Select a command from the matching results.".into());
        }
        let aliases = map
            .bindings(&action.id)
            .expect("catalog action has bindings");
        if aliases.contains(chord) {
            return Err("Already assigned to this command.".into());
        }
        let mut keys = aliases
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>();
        keys.push(chord.display());
        let mut values = self.preferences.values.clone();
        let scope = action.scope_for(chord);
        let mut owners = Vec::new();
        for owner in map.definitions() {
            if owner.id == action.id
                || !CONTEXTS.into_iter().any(|context| {
                    scope.includes(context) && owner.scope_for(chord).includes(context)
                })
            {
                continue;
            }
            let aliases = map
                .bindings(&owner.id)
                .expect("catalog action has bindings");
            if !aliases.contains(chord) {
                continue;
            }
            owners.push(format!(
                "{} — {}",
                owner.label,
                owner.scope_for(chord).label()
            ));
            replace_aliases(
                &mut values,
                &owner.id,
                aliases
                    .iter()
                    .filter(|alias| *alias != chord)
                    .map(ShortcutChord::display)
                    .collect(),
            );
        }
        replace_aliases(&mut values, &action.id, keys);
        // Validate the complete proposed map before offering confirmation. This
        // retains alias/override limits and native editor/IME reservations.
        ShortcutMap::with_user_commands(&values.shortcuts, &values.user_commands)?;
        Ok(Assignment { values, owners })
    }

    pub(crate) fn assign_editor_shortcut(
        &mut self,
        action: &str,
        candidate: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let definition = &self.keyboard.snapshot.map.definitions()[self.keyboard.selected];
        let Some(chord) = &self.keyboard.editor.candidate else {
            return false;
        };
        if action != definition.id || candidate != chord.display() {
            return false;
        }
        let assignment = match self.shortcut_assignment() {
            Ok(assignment) => assignment,
            Err(error) => {
                self.keyboard.error = error;
                return false;
            }
        };
        if assignment.owners.is_empty() {
            return self.apply_shortcut_values(assignment.values, send);
        }
        if !self.replacement_options_ready() {
            return false;
        }
        let Some(sequence) = self.keyboard.editor.replacement_sequence.checked_add(1) else {
            self.keyboard.error =
                "Shortcut confirmation identities are exhausted. Restart the application.".into();
            return false;
        };
        let chord = chord.clone();
        let body = format!(
            "Assign {} to {}?\n\nThis removes this shortcut from:\n{}\n\nEach listed assignment is removed for its full scope. Other shortcuts and commands remain unchanged.",
            chord.display(),
            definition.label,
            assignment.owners.join("\n")
        );
        self.keyboard.reset_editor();
        self.keyboard.editor.replacement_sequence = sequence;
        self.keyboard.editor.replacement = Some(ShortcutReplacement {
            token: format!("shortcut-replace:{sequence}"),
            body,
            action: action.to_owned(),
            chord,
            map: self.keyboard.snapshot.clone(),
            editor_revision: self.keyboard.editor_revision_text.to_string(),
        });
        true
    }

    fn replacement_options_ready(&self) -> bool {
        self.preferences.loaded
            && self.preferences.options.open
            && !self.preferences.options.accepting
            && !self.preferences.options.saving
    }

    fn replacement_current(&self, replacement: &ShortcutReplacement) -> bool {
        self.replacement_options_ready()
            && Arc::ptr_eq(&self.keyboard.snapshot, &replacement.map)
            && self.keyboard.editor_revision_text.as_str() == replacement.editor_revision.as_str()
            && self.keyboard.snapshot.map.definitions()[self.keyboard.selected].id
                == replacement.action
            && self.keyboard.editor.candidate.as_ref() == Some(&replacement.chord)
    }

    pub(crate) fn shortcut_replacement(&self) -> Option<(&str, &str)> {
        self.keyboard
            .editor
            .replacement
            .as_ref()
            .and_then(|replacement| {
                self.replacement_current(replacement)
                    .then_some((replacement.token.as_str(), replacement.body.as_str()))
            })
    }

    pub(crate) fn decide_shortcut_replacement(
        &mut self,
        token: &str,
        replace: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(replacement) = &self.keyboard.editor.replacement else {
            return false;
        };
        if replacement.token != token {
            return false;
        }
        let current = self.replacement_current(replacement);
        self.keyboard.reset_editor();
        if !current {
            return false;
        }
        if !replace {
            return true;
        }
        let assignment = match self.shortcut_assignment() {
            Ok(assignment) => assignment,
            Err(error) => {
                self.keyboard.error = error;
                return false;
            }
        };
        self.apply_shortcut_values(assignment.values, send)
    }

    pub(crate) fn remove_editor_shortcut(
        &mut self,
        action: &str,
        key: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if action != self.keyboard.snapshot.map.definitions()[self.keyboard.selected].id {
            return false;
        }
        let aliases = self
            .keyboard
            .snapshot
            .map
            .bindings(action)
            .expect("catalog action");
        let keys = aliases
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>();
        if !keys.iter().any(|alias| alias == key) {
            return false;
        }
        self.change_shortcuts(
            action,
            keys.into_iter().filter(|alias| alias != key).collect(),
            send,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn editor(action: &str) -> Presenter {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.apply(Event::PreferencesLoaded(Ok(Box::default())));
        presenter.keyboard.selected = shortcut_catalog()
            .iter()
            .position(|item| item.id == action)
            .unwrap();
        presenter
    }

    #[test]
    fn assign_and_remove_one_alias_preserves_the_other_alias_and_persists() {
        let mut presenter = editor("files.copy");
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        presenter.capture_editor_shortcut(ShortcutChord::parse("ctrl+alt+y").unwrap());
        assert!(presenter.inspect_editor_shortcut().can_assign);
        assert!(sent.borrow().is_empty());
        assert!(presenter.assign_editor_shortcut("files.copy", "Ctrl+Alt+Y", &send));
        assert_eq!(
            presenter.keyboard.snapshot.editor_text("files.copy"),
            "Ctrl+C\nCtrl+Alt+Y"
        );
        assert!(presenter.remove_editor_shortcut("files.copy", "Ctrl+C", &send));
        assert_eq!(
            presenter.keyboard.snapshot.editor_text("files.copy"),
            "Ctrl+Alt+Y"
        );
        assert!(matches!(
            sent.borrow().last(),
            Some(Command::SavePreferences { .. })
        ));
        assert!(!presenter.remove_editor_shortcut("files.paste", "Ctrl+V", &send));
    }

    #[test]
    fn conflicts_duplicates_and_native_editing_reservations_are_previewed_before_save() {
        let mut presenter = editor("files.copy");
        presenter.capture_editor_shortcut(ShortcutChord::parse("Ctrl+V").unwrap());
        let inspection = presenter.inspect_editor_shortcut();
        assert!(inspection.can_assign);
        assert!(inspection.used_by.contains("Paste files"));
        assert!(
            !presenter
                .assign_editor_shortcut("files.copy", "Ctrl+V", &|_| panic!("conflict saved"))
        );
        presenter.capture_editor_shortcut(ShortcutChord::parse("Ctrl+C").unwrap());
        assert!(
            presenter
                .inspect_editor_shortcut()
                .issue
                .contains("Already assigned")
        );
        let mut presenter = editor("app.help");
        presenter.capture_editor_shortcut(ShortcutChord::parse("Ctrl+Space").unwrap());
        assert!(
            presenter
                .inspect_editor_shortcut()
                .issue
                .contains("input method")
        );
    }

    #[test]
    fn filtering_preserves_identity_and_never_assigns_to_a_hidden_command() {
        let mut presenter = editor("files.copy");
        presenter.capture_editor_shortcut(ShortcutChord::parse("Ctrl+Alt+Y").unwrap());
        presenter.filter_shortcut_commands("FILES.COPY".into());
        assert!(presenter.inspect_editor_shortcut().can_assign);
        let selected = presenter.keyboard.selected;
        presenter.filter_shortcut_commands("no matching command 🦀".into());
        assert_eq!(presenter.keyboard.selected, selected);
        assert!(!presenter.inspect_editor_shortcut().can_assign);
        assert!(
            !presenter.assign_editor_shortcut("files.copy", "Ctrl+Alt+Y", &|_| panic!(
                "hidden command saved"
            ))
        );
    }
}
