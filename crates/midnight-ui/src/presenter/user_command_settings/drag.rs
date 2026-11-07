//! One captured settings gesture; the existing validated list transaction persists its order.
use super::{super::*, reorder::Drag};
use std::{cell::Cell, rc::Rc, sync::Weak};

#[derive(Default)]
pub(super) struct State {
    configuration: Weak<Vec<UserCommand>>,
    revision: u64,
    serial: u64,
    active: Rc<Cell<u64>>,
    capture: Option<(u64, Drag)>,
}
impl State {
    pub(super) fn sync(&mut self, commands: &Arc<Vec<UserCommand>>) {
        if self.configuration.as_ptr() != Arc::as_ptr(commands) {
            self.configuration = Arc::downgrade(commands);
            self.revision = self.revision.saturating_add(1);
            self.cancel();
        }
    }
    pub(super) fn cancel(&mut self) -> bool {
        self.active.set(0);
        self.capture.take().is_some()
    }
}

impl Presenter {
    pub(crate) fn user_command_drag_cancel_handle(&self) -> Rc<Cell<u64>> {
        self.user_command_settings.drag.active.clone()
    }
    pub(crate) fn user_command_drag_revision(&self) -> u64 {
        self.user_command_settings.drag.revision
    }
    pub(crate) fn user_command_drag_token(&self) -> String {
        let drag = &self.user_command_settings.drag;
        if drag.active.get() == 0 || !self.user_command_settings_available() {
            String::new()
        } else {
            drag.active.get().to_string()
        }
    }
    pub(crate) fn cancel_user_command_drag(&mut self) {
        if self.user_command_settings.drag.cancel() {
            self.user_command_settings.changed(false);
        }
    }
    pub(crate) fn begin_user_command_drag(&mut self, id: &str) -> String {
        self.cancel_user_command_drag();
        if !self.command_settings_ready() {
            return String::new();
        }
        if self.user_command_settings.dirty {
            self.user_command_settings
                .fail("Apply or cancel the current draft before reordering commands.");
            return String::new();
        }
        let commands = &self.preferences.values.user_commands;
        let state = &mut self.user_command_settings;
        state.drag.sync(commands);
        let Some(token) = state.drag.serial.checked_add(1) else {
            state.fail("Command ordering gesture identities are exhausted.");
            return String::new();
        };
        let capture = match Drag::new(commands, state.drag.revision, id) {
            Ok(capture) => capture,
            Err(error) => {
                state.fail(error);
                return String::new();
            }
        };
        state.drag.serial = token;
        state.drag.active.set(token);
        state.drag.capture = Some((token, capture));
        state.error.clear();
        state.changed(false);
        token.to_string()
    }
    pub(crate) fn finish_user_command_drag(
        &mut self,
        token: &str,
        boundary: i32,
        commit: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Ok(token) = token.parse::<u64>() else {
            return false;
        };
        let state = &mut self.user_command_settings;
        if token == 0
            || state.drag.active.get() != token
            || state
                .drag
                .capture
                .as_ref()
                .is_none_or(|capture| capture.0 != token)
        {
            return false;
        }
        let Some((_, capture)) = state.drag.capture.take() else {
            return false;
        };
        state.drag.active.set(0);
        state.changed(false);
        if !self.command_settings_ready() {
            return false;
        }
        let state = &self.user_command_settings;
        let commands = &self.preferences.values.user_commands;
        let result = if commit {
            usize::try_from(boundary)
                .map_err(|_| "This command drop position is unavailable.")
                .and_then(|boundary| {
                    capture.order(commands, state.drag.revision, state.dirty, boundary)
                })
        } else {
            capture
                .validate(commands, state.drag.revision, state.dirty)
                .map(|()| None)
        };
        match result {
            Ok(Some(commands)) => {
                self.commit_user_command_list(commands, Some(capture.id().clone()), send)
            }
            Ok(None) => {
                if commit {
                    self.select_user_command_setting(capture.id().as_str());
                }
                true
            }
            Err(error) => {
                self.user_command_settings.fail(error);
                false
            }
        }
    }
}
