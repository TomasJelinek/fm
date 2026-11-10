//! Stable configuration drafts. Persistence and process work stay behind backend commands.
use super::*;

mod drag;
pub(crate) mod form;
mod reorder;
#[cfg(test)]
mod reorder_tests;
use form::Form;

#[derive(Default)]
pub(crate) struct State {
    drag: drag::State,
    pub selected: Option<UserCommandId>,
    source: std::sync::Weak<Vec<UserCommand>>,
    pub revision: u64,
    pub draft_revision: u64,
    pub dirty: bool,
    pub new: bool,
    pub can_apply: bool,
    pub can_preview: bool,
    pub error: String,
    pub validation: String,
    pub status: String,
    pub(crate) draft: Option<Form>,
    pub(crate) picked_executable: Option<PathBuf>,
    pub(crate) picked_icon: Option<PathBuf>,
    pub(crate) edit_revision: u64,
    pub(crate) preview_revision: u64,
    next_id: u64,
}
impl State {
    pub(crate) fn token(&self) -> String {
        format!(
            "{}:{}",
            self.draft_revision,
            self.selected.as_ref().map_or("", UserCommandId::as_str)
        )
    }
    fn changed(&mut self, draft: bool) {
        self.revision = self.revision.saturating_add(1);
        if draft {
            self.status.clear();
            self.draft_revision = self.draft_revision.saturating_add(1);
        }
    }
    pub(super) fn clear_status(&mut self) {
        if !self.status.is_empty() {
            self.status.clear();
            self.changed(false);
        }
    }
    pub(super) fn fail(&mut self, error: impl Into<String>) {
        self.error = error.into();
        self.changed(false);
    }
}

impl Presenter {
    pub(crate) fn remember_user_command_form(&mut self, token: &str, form: Form) {
        if token != self.user_command_settings.token()
            || self
                .user_command_settings
                .selected
                .as_ref()
                .is_none_or(|id| id.as_str() != form.id)
        {
            return;
        }
        let state = &mut self.user_command_settings;
        if state
            .picked_executable
            .as_ref()
            .is_some_and(|p| p.to_string_lossy() != form.executable)
        {
            state.picked_executable = None;
        }
        if state
            .picked_icon
            .as_ref()
            .is_some_and(|p| p.to_string_lossy() != form.icon)
        {
            state.picked_icon = None;
        }
        if let Some(old) = state.draft.replace(form) {
            self.retired.push(Box::new(old));
        }
        self.user_command_settings.edit_revision =
            self.user_command_settings.edit_revision.saturating_add(1);
        self.mark_user_command_setting_dirty(token);
    }
    pub(crate) fn edit_user_command_argument(
        &mut self,
        token: &str,
        operation: i32,
        row: i32,
        text: &str,
    ) {
        let Some(mut form) = self.user_command_settings_form() else {
            return;
        };
        let Ok(mut values) = serde_json::from_str::<Vec<String>>(&form.arguments) else {
            return;
        };
        if !crate::argument_rows::change(&mut values, operation, row, text) {
            return;
        }
        form.arguments = serde_json::to_string(&values).unwrap_or_default();
        self.remember_user_command_form(token, form);
    }
    pub(crate) fn user_command_settings_available(&self) -> bool {
        self.preferences.loaded && !self.operations.closing
    }
    pub(crate) fn user_command_settings_current(&self) -> bool {
        self.user_command_settings.source.as_ptr()
            == Arc::as_ptr(&self.preferences.values.user_commands)
    }
    pub(crate) fn insert_user_command_setting_variable(
        &mut self,
        token: &str,
        form: &mut Form,
        field: i32,
        variable: i32,
    ) -> bool {
        if token != self.user_command_settings.token() || !self.user_command_settings_current() {
            return false;
        }
        let variable = usize::try_from(variable).ok().and_then(|index| {
            [
                UserCommandVariable::ActivePath,
                UserCommandVariable::OtherPath,
                UserCommandVariable::FilePath,
                UserCommandVariable::FileName,
                UserCommandVariable::FileStem,
                UserCommandVariable::FileExtension,
                UserCommandVariable::SelectedPaths,
            ]
            .get(index)
            .copied()
        });
        let Some(variable) = variable else {
            self.user_command_settings
                .fail("Choose a context variable.");
            return false;
        };
        if let Err(error) = form.insert_variable(field, variable) {
            self.user_command_settings.fail(error);
            return false;
        }
        self.mark_user_command_setting_dirty(token);
        true
    }
    pub(crate) fn mark_user_command_setting_dirty(&mut self, token: &str) {
        if token != self.user_command_settings.token() {
            return;
        }
        self.settings_picker.invalidate();
        let state = &mut self.user_command_settings;
        state.drag.cancel();
        state.dirty = true;
        state.can_apply = false;
        state.can_preview = false;
        state.error.clear();
        state.validation.clear();
        state.status.clear();
        state.changed(false);
    }
    pub(crate) fn sync_user_command_settings(&mut self) {
        let commands = &self.preferences.values.user_commands;
        let state = &mut self.user_command_settings;
        state.drag.sync(commands);
        state.next_id = state.next_id.max(
            commands
                .iter()
                .filter_map(|command| {
                    command
                        .id
                        .as_str()
                        .strip_prefix("custom-")?
                        .parse::<u64>()
                        .ok()
                })
                .max()
                .unwrap_or(0),
        );
        if state.source.as_ptr() == Arc::as_ptr(commands) {
            return;
        }
        if state.dirty {
            state.can_apply = false;
            state.can_preview = false;
            state.fail("Custom Commands changed. Cancel the draft to reload current settings.");
            return;
        }
        state.source = Arc::downgrade(commands);
        state.selected = state
            .selected
            .take()
            .filter(|id| commands.iter().any(|command| command.id == *id))
            .or_else(|| commands.first().map(|command| command.id.clone()));
        state.new = false;
        state.draft = None;
        state.picked_executable = None;
        state.picked_icon = None;
        state.error.clear();
        state.validation.clear();
        state.can_apply = false;
        state.can_preview = state.selected.is_some();
        state.changed(true);
    }
    pub(crate) fn user_command_settings_form(&self) -> Option<Form> {
        let state = &self.user_command_settings;
        if let Some(form) = &state.draft {
            return Some(form.clone());
        }
        let id = state.selected.as_ref()?;
        if state.new {
            let mut form = Form::new(id.as_str().into());
            if self.desktop.shell == Some(ShellFlavor::Posix) {
                form.shell = 1;
            }
            Some(form)
        } else {
            self.preferences
                .values
                .user_commands
                .iter()
                .find(|command| command.id == *id)
                .map(Form::from_command)
        }
    }
    fn command_settings_ready(&mut self) -> bool {
        if !self.preferences.loaded || self.operations.closing {
            self.user_command_settings
                .fail("Wait for saved options to load before editing Custom Commands.");
            return false;
        }
        true
    }
    pub(crate) fn select_user_command_setting(&mut self, id: &str) {
        if !self.command_settings_ready() {
            return;
        }
        if self.user_command_settings.dirty {
            self.user_command_settings
                .fail("Apply or cancel the current draft before selecting another command.");
            return;
        }
        let Some(id) = self
            .preferences
            .values
            .user_commands
            .iter()
            .find(|command| command.id.as_str() == id)
            .map(|command| command.id.clone())
        else {
            return;
        };
        self.user_command_settings.drag.cancel();
        self.user_command_settings.selected = Some(id);
        self.user_command_settings.source = Arc::downgrade(&self.preferences.values.user_commands);
        self.user_command_settings.new = false;
        self.user_command_settings.draft = None;
        self.user_command_settings.picked_executable = None;
        self.user_command_settings.picked_icon = None;
        self.user_command_settings.can_apply = false;
        self.user_command_settings.can_preview = true;
        self.user_command_settings.error.clear();
        self.user_command_settings.validation.clear();
        self.user_command_settings.changed(true);
    }
    pub(crate) fn new_user_command_setting(&mut self) {
        if !self.command_settings_ready() {
            return;
        }
        if self.user_command_settings.dirty {
            self.user_command_settings
                .fail("Apply or cancel the current draft before adding a command.");
            return;
        }
        if self.preferences.values.user_commands.len() == MAX_USER_COMMANDS {
            self.user_command_settings
                .fail("At most 128 Custom Commands can be configured.");
            return;
        }
        self.sync_user_command_settings();
        for _ in 0..=MAX_USER_COMMANDS {
            let Some(next) = self.user_command_settings.next_id.checked_add(1) else {
                self.user_command_settings
                    .fail("Custom Command identities are exhausted.");
                return;
            };
            self.user_command_settings.next_id = next;
            let Ok(id) = UserCommandId::new(format!("custom-{next}")) else {
                continue;
            };
            if self
                .preferences
                .values
                .user_commands
                .iter()
                .any(|command| command.id == id)
            {
                continue;
            }
            let state = &mut self.user_command_settings;
            state.drag.cancel();
            state.selected = Some(id);
            state.new = true;
            state.draft = None;
            state.picked_executable = None;
            state.picked_icon = None;
            state.dirty = true;
            state.can_apply = false;
            state.can_preview = false;
            state.error.clear();
            state.validation =
                "Enter an absolute executable path, or choose Terminal and enter a script.".into();
            state.status.clear();
            state.changed(true);
            return;
        }
        self.user_command_settings
            .fail("No available Custom Command identity.");
    }
    pub(crate) fn cancel_user_command_setting(&mut self) {
        let state = &mut self.user_command_settings;
        state.status.clear();
        state.drag.cancel();
        state.dirty = false;
        state.new = false;
        state.source = Default::default();
        state.error.clear();
        state.validation.clear();
        state.status.clear();
        self.sync_user_command_settings();
    }
    fn command_settings_candidate(
        &mut self,
        token: &str,
        form: &Form,
    ) -> Result<UserPreferences, String> {
        let state = &self.user_command_settings;
        if !self.preferences.loaded || self.operations.closing {
            return Err("Custom Commands settings are not available yet.".into());
        }
        if token != state.token()
            || state
                .selected
                .as_ref()
                .is_none_or(|id| id.as_str() != form.id)
        {
            return Err("This command draft is obsolete. Select the command again.".into());
        }
        let source = state
            .source
            .upgrade()
            .filter(|source| Arc::ptr_eq(source, &self.preferences.values.user_commands))
            .ok_or("Custom Commands changed. Cancel the draft to reload current settings.")?;
        let position = source
            .iter()
            .position(|command| command.id.as_str() == form.id);
        if !state.new && position.is_none() || state.new && position.is_some() {
            return Err("This command no longer matches its configuration.".into());
        }
        let command = form.command_with_native(
            position.and_then(|row| source.get(row)),
            state.picked_executable.as_deref(),
            state.picked_icon.as_deref(),
        )?;
        let mut commands = source.as_ref().clone();
        if let Some(row) = position {
            commands[row] = command;
        } else {
            commands.push(command);
        }
        let mut values = self.preferences.values.clone();
        let commands = Arc::new(commands);
        if let Err(error) = values.replace_user_commands(commands.clone()) {
            self.retired.push(Box::new(commands));
            return Err(error.to_string());
        }
        Ok(values)
    }
    pub(crate) fn validate_user_command_setting(&mut self, token: &str, form: &Form) {
        if token != self.user_command_settings.token() {
            return;
        }
        match self.command_settings_candidate(token, form) {
            Ok(values) => {
                let dirty = values.user_commands != self.preferences.values.user_commands;
                self.retired.push(Box::new(values));
                let state = &mut self.user_command_settings;
                if dirty {
                    state.drag.cancel();
                }
                state.dirty = dirty;
                state.can_apply = dirty;
                state.can_preview = true;
                state.validation.clear();
                state.error.clear();
                state.changed(false);
            }
            Err(error) => {
                let state = &mut self.user_command_settings;
                state.drag.cancel();
                state.dirty = true;
                state.can_apply = false;
                state.can_preview = false;
                state.validation = error;
                state.error.clear();
                state.changed(false);
            }
        }
    }
    pub(crate) fn apply_user_command_setting(
        &mut self,
        token: &str,
        form: &Form,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let values = match self.command_settings_candidate(token, form) {
            Ok(values) => values,
            Err(error) => {
                self.user_command_settings.fail(error);
                return false;
            }
        };
        let accepted = values.user_commands.clone();
        if values != self.preferences.values {
            self.update_preferences(values, send);
        } else {
            self.retired.push(Box::new(values));
        }
        if *accepted != *self.preferences.values.user_commands {
            self.retired.push(Box::new(accepted));
            self.user_command_settings
                .fail("Custom Commands could not be applied. Check the shortcut assignments.");
            return false;
        }
        self.user_command_settings.dirty = false;
        self.user_command_settings.new = false;
        self.user_command_settings.source = Default::default();
        self.sync_user_command_settings();
        self.user_command_settings.status =
            "Command applied. Options save status appears below.".into();
        true
    }
    pub(crate) fn delete_user_command_setting(
        &mut self,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.command_settings_ready()
            || token != self.user_command_settings.token()
            || self.user_command_settings.dirty
        {
            return false;
        }
        if self.user_command_settings.source.as_ptr()
            != Arc::as_ptr(&self.preferences.values.user_commands)
        {
            return false;
        }
        let Some(id) = &self.user_command_settings.selected else {
            return false;
        };
        let mut commands = self.preferences.values.user_commands.as_ref().clone();
        let Some(row) = commands.iter().position(|command| command.id == *id) else {
            return false;
        };
        let removed = commands.remove(row);
        self.retired.push(Box::new(removed));
        let selected = commands
            .get(row.min(commands.len().saturating_sub(1)))
            .map(|command| command.id.clone());
        self.commit_user_command_list(commands, selected, send)
    }
    pub(crate) fn move_user_command_setting(
        &mut self,
        token: &str,
        step: i32,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.command_settings_ready()
            || token != self.user_command_settings.token()
            || self.user_command_settings.dirty
            || !matches!(step, -1 | 1)
        {
            return false;
        }
        if self.user_command_settings.source.as_ptr()
            != Arc::as_ptr(&self.preferences.values.user_commands)
        {
            return false;
        }
        let Some(id) = &self.user_command_settings.selected else {
            return false;
        };
        let mut commands = self.preferences.values.user_commands.as_ref().clone();
        let Some(row) = commands.iter().position(|command| command.id == *id) else {
            return false;
        };
        let Some(next) = row
            .checked_add_signed(step as isize)
            .filter(|next| *next < commands.len())
        else {
            self.retired.push(Box::new(commands));
            return false;
        };
        commands.swap(row, next);
        let selected = self.user_command_settings.selected.clone();
        self.commit_user_command_list(commands, selected, send)
    }
    fn commit_user_command_list(
        &mut self,
        commands: Vec<UserCommand>,
        selected: Option<UserCommandId>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let mut values = self.preferences.values.clone();
        let commands = Arc::new(commands);
        if let Err(error) = values.replace_user_commands(commands.clone()) {
            self.retired.push(Box::new(commands));
            self.user_command_settings.fail(error.to_string());
            return false;
        }
        self.update_preferences(values, send);
        if *commands != *self.preferences.values.user_commands {
            self.retired.push(Box::new(commands));
            self.user_command_settings
                .fail("Custom Commands could not be applied. Check the shortcut assignments.");
            return false;
        }
        self.user_command_settings.selected = selected;
        self.user_command_settings.source = Default::default();
        self.sync_user_command_settings();
        true
    }
    pub(crate) fn preview_user_command_setting(
        &mut self,
        token: &str,
        form: &Form,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let values = match self.command_settings_candidate(token, form) {
            Ok(values) => values,
            Err(error) => {
                self.user_command_settings.fail(error);
                return false;
            }
        };
        let accepted = self.preview_user_command_configuration(
            self.active,
            values.user_commands.clone(),
            &form.id,
            send,
        );
        self.retired.push(Box::new(values));
        if accepted {
            self.user_command_settings.error.clear();
            self.user_command_settings.status = if self.current_settings_command_preview().is_none()
            {
                "Waiting for panel selection...".into()
            } else {
                String::new()
            };
            self.user_command_settings.changed(false);
        } else if self.user_command_settings.error.is_empty() {
            self.user_command_settings
                .fail("Preview could not be queued. Try again.");
        }
        accepted
    }
}
