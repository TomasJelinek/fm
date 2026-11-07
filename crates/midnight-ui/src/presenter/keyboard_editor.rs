//! Bounded keyboard-editor presentation; persistence uses the existing preference transaction.
use super::*;

#[derive(Default)]
pub(crate) struct ShortcutEditor {
    pub(crate) filter: String,
    pub(crate) candidate: Option<ShortcutChord>,
}

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
        let rows = shortcut_catalog()
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
        let action = &shortcut_catalog()[self.keyboard.selected];
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
        let contexts = [
            ShortcutContext::Files,
            ShortcutContext::Search,
            ShortcutContext::Command,
            ShortcutContext::Other,
            ShortcutContext::Volume,
            ShortcutContext::Viewer,
            ShortcutContext::ViewerSearch,
        ];
        let mut seen = HashSet::new();
        let mut owners = Vec::new();
        let mut conflict = false;
        for context in contexts {
            if let Some(owner) = map.resolve(chord, context) {
                conflict |= owner.id != action.id && scope.includes(context);
                if seen.insert(owner.id.as_str()) {
                    owners.push(format!(
                        "{} — {}",
                        owner.label,
                        owner.scope_for(chord).label()
                    ));
                }
            }
        }
        inspection.used_by = if owners.is_empty() {
            "Unassigned".into()
        } else {
            owners.join("\n")
        };
        if !matches_filter(action, &self.keyboard.editor.filter) {
            inspection.issue = "Select a command from the matching results.".into();
            return inspection;
        }
        let aliases = map
            .bindings(&action.id)
            .expect("catalog action has bindings");
        if aliases.contains(chord) {
            inspection.issue = "Already assigned to this command.".into();
            return inspection;
        }
        if conflict {
            inspection.issue = "This shortcut is used by another command in the same context. Remove that assignment first.".into();
            return inspection;
        }
        let mut keys = aliases
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>();
        keys.push(chord.display());
        let mut values = self.preferences.values.clone();
        replace_aliases(&mut values, &action.id, keys);
        match ShortcutMap::new(&values.shortcuts) {
            Ok(_) => inspection.can_assign = true,
            Err(error) => inspection.issue = error,
        }
        inspection
    }

    pub(crate) fn assign_editor_shortcut(
        &mut self,
        action: &str,
        candidate: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let definition = &shortcut_catalog()[self.keyboard.selected];
        let Some(chord) = &self.keyboard.editor.candidate else {
            return false;
        };
        if action != definition.id || candidate != chord.display() {
            return false;
        }
        let inspection = self.inspect_editor_shortcut();
        if !inspection.can_assign {
            self.keyboard.error = inspection.issue;
            return false;
        }
        let mut aliases = self
            .keyboard
            .snapshot
            .map
            .bindings(action)
            .expect("catalog action")
            .iter()
            .map(ShortcutChord::display)
            .collect::<Vec<_>>();
        aliases.push(chord.display());
        self.change_shortcuts(action, aliases, send)
    }

    pub(crate) fn remove_editor_shortcut(
        &mut self,
        action: &str,
        key: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if action != shortcut_catalog()[self.keyboard.selected].id {
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
        assert!(!inspection.can_assign);
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
