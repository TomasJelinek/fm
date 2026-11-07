//! Options form/model only. Settings encoding, metadata and process work stay in services.
use crate::presenter::user_command_settings::form::Form;
use crate::{
    MainWindow, Presenter, UserCommandSettingsForm, UserCommandSettingsRow, UserCommandSettingsView,
};
use midnight_core::{Command, ShellFlavor, UserCommandInvocation, UserCommandLaunch};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    let view = window.global::<UserCommandSettingsView>();
    let state = &p.user_command_settings;
    let commands = &p.preferences.values.user_commands;
    let revision = format!(
        "{}:{}:{}:{:p}",
        state.revision,
        p.preferences.revision.0,
        p.user_command_settings_available(),
        Arc::as_ptr(commands)
    );
    if view.get_revision() == revision {
        return;
    }
    view.set_ready(p.user_command_settings_available());
    view.set_configuration_revision(p.user_command_drag_revision().to_string().into());
    let drag_token = p.user_command_drag_token();
    view.set_drag_active(!drag_token.is_empty());
    view.set_drag_token(drag_token.into());
    view.set_has_draft(state.selected.is_some());
    view.set_dirty(state.dirty);
    view.set_can_apply(state.can_apply && p.user_command_settings_current());
    view.set_can_preview(state.can_preview && p.user_command_settings_current());
    view.set_token(state.token().into());
    view.set_error(state.error.as_str().into());
    view.set_validation(state.validation.as_str().into());
    view.set_status(state.status.as_str().into());
    let position = state
        .selected
        .as_ref()
        .and_then(|id| commands.iter().position(|command| command.id == *id));
    view.set_selected_index(position.map_or(-1, |row| row as i32));
    view.set_can_up(position.is_some_and(|row| row > 0));
    view.set_can_down(position.is_some_and(|row| row + 1 < commands.len()));
    view.set_can_delete(position.is_some() && !state.new);
    let draft_revision = state.draft_revision.to_string();
    // Ordinary validation never replaces text/cursor state in a live input.
    if view.get_draft_revision() != draft_revision {
        if let Some(form) = p.user_command_settings_form() {
            view.set_draft(to_view(form));
        }
        view.set_draft_revision(draft_revision.into());
    }
    let rows_revision = format!(
        "{:p}:{}",
        Arc::as_ptr(commands),
        state.selected.as_ref().map_or("", |id| id.as_str())
    );
    if view.get_rows_revision() != rows_revision {
        let rows = commands
            .iter()
            .map(|command| {
                let kind = match &command.invocation {
                    UserCommandInvocation::Program { executable, .. } => format!(
                        "Program · {}",
                        executable
                            .file_name()
                            .unwrap_or(executable.as_os_str())
                            .to_string_lossy()
                    ),
                    UserCommandInvocation::Terminal { shell, .. } => match shell {
                        ShellFlavor::PowerShell => "PowerShell".into(),
                        ShellFlavor::Posix => "POSIX terminal".into(),
                        ShellFlavor::Cmd => "Command Prompt · unavailable".into(),
                    },
                };
                UserCommandSettingsRow {
                    id: command.id.as_str().into(),
                    name: command.name.as_str().into(),
                    summary: format!(
                        "{kind} · {}",
                        if command.launch == UserCommandLaunch::Single {
                            "Single"
                        } else {
                            "Per item"
                        }
                    )
                    .into(),
                    enabled: command.enabled,
                    selected: state.selected.as_ref() == Some(&command.id),
                }
            })
            .collect::<Vec<_>>();
        view.set_rows(ModelRc::new(VecModel::from(rows)));
        view.set_rows_revision(rows_revision.into());
    }
    view.set_revision(revision.into());
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let selected = state.clone();
    window
        .global::<UserCommandSettingsView>()
        .on_drag_begin(move |id| {
            let token = selected.borrow_mut().begin_user_command_drag(id.as_str());
            if let Some(window) = weak.upgrade() {
                refresh(&window, &selected.borrow());
            }
            token.into()
        });
    let weak = window.as_weak();
    let selected = state.clone();
    let sender = send.clone();
    window
        .global::<UserCommandSettingsView>()
        .on_drag_end(move |token, boundary, commit| {
            let accepted = selected.borrow_mut().finish_user_command_drag(
                token.as_str(),
                boundary,
                commit,
                sender.as_ref(),
            );
            if let Some(window) = weak.upgrade() {
                if commit {
                    crate::refresh(&window, &selected.borrow());
                } else {
                    refresh(&window, &selected.borrow());
                }
            }
            accepted
        });
    let weak = window.as_weak();
    let selected = state.clone();
    let cancellation = state.borrow().user_command_drag_cancel_handle();
    window
        .global::<UserCommandSettingsView>()
        .on_drag_cancel(move || {
            // Cancellation must also invalidate a drop during an existing Presenter
            // borrow. This small cell is shared with the controller, never its data.
            cancellation.set(0);
            let Some(window) = weak.upgrade() else {
                return;
            };
            window
                .global::<UserCommandSettingsView>()
                .set_drag_active(false);
            let cleared = if let Ok(mut p) = selected.try_borrow_mut() {
                p.cancel_user_command_drag();
                true
            } else {
                false
            };
            if cleared {
                refresh(&window, &selected.borrow());
            }
        });
    macro_rules! bind {
        ($callback:ident, $full:expr, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let weak = window.as_weak(); let state = state.clone(); let sender = send.clone();
            window.global::<UserCommandSettingsView>().$callback(move |$($arg),*| {
                let Some(window) = weak.upgrade() else { return; };
                { let mut state = state.borrow_mut(); let $p = &mut *state; let $send = sender.as_ref(); $body }
                if $full { crate::refresh(&window, &state.borrow()); } else { refresh(&window, &state.borrow()); }
            });
        }};
    }
    bind!(on_select, false, |p, _send, id| {
        p.select_user_command_setting(id.as_str());
    });
    bind!(on_new_command, false, |p, _send| {
        p.new_user_command_setting();
    });
    bind!(on_cancel, false, |p, _send| {
        p.cancel_user_command_setting();
    });
    bind!(on_edit_started, false, |p, _send, token| {
        p.mark_user_command_setting_dirty(token.as_str());
    });
    bind!(on_validate, false, |p, _send, token, form| {
        p.validate_user_command_setting(token.as_str(), &from_view(form));
    });
    bind!(on_apply, true, |p, send, token, form| {
        p.apply_user_command_setting(token.as_str(), &from_view(form), send);
    });
    bind!(on_delete, true, |p, send, token| {
        p.delete_user_command_setting(token.as_str(), send);
    });
    bind!(on_move, true, |p, send, token, step| {
        p.move_user_command_setting(token.as_str(), step, send);
    });
    let weak = window.as_weak();
    let selected = state.clone();
    window
        .global::<UserCommandSettingsView>()
        .on_insert_variable(move |token, form, field, variable| {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            let mut form = from_view(form);
            let accepted = selected.borrow_mut().insert_user_command_setting_variable(
                token.as_str(),
                &mut form,
                field,
                variable,
            );
            if accepted {
                window
                    .global::<UserCommandSettingsView>()
                    .set_draft(to_view(form));
            }
            refresh(&window, &selected.borrow());
            accepted
        });
    let weak = window.as_weak();
    window
        .global::<UserCommandSettingsView>()
        .on_preview(move |token, form| {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            let accepted = state.borrow_mut().preview_user_command_setting(
                token.as_str(),
                &from_view(form),
                send.as_ref(),
            );
            if accepted {
                let weak = window.as_weak();
                let state = Rc::downgrade(&state);
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    if let (Some(window), Some(state)) = (weak.upgrade(), state.upgrade()) {
                        crate::refresh(&window, &state.borrow());
                    }
                });
            } else {
                crate::refresh(&window, &state.borrow());
            }
            accepted
        });
}

fn to_view(form: Form) -> UserCommandSettingsForm {
    UserCommandSettingsForm {
        id: form.id.into(),
        name: form.name.into(),
        enabled: form.enabled,
        toolbar_label: form.toolbar_label.into(),
        icon: form.icon.into(),
        icon_edited: form.icon_edited,
        working_directory: form.working_directory.into(),
        kind: form.kind,
        launch: form.launch,
        executable: form.executable.into(),
        executable_edited: form.executable_edited,
        arguments: form.arguments.into(),
        script: form.script.into(),
        shell: form.shell,
        active_path: form.active_path,
        other_path: form.other_path,
        file_path: form.file_path,
        file_name: form.file_name,
        file_stem: form.file_stem,
        file_extension: form.file_extension,
    }
}
fn from_view(form: UserCommandSettingsForm) -> Form {
    Form {
        id: form.id.to_string(),
        name: form.name.to_string(),
        enabled: form.enabled,
        toolbar_label: form.toolbar_label.to_string(),
        icon: form.icon.to_string(),
        icon_edited: form.icon_edited,
        working_directory: form.working_directory.to_string(),
        kind: form.kind,
        launch: form.launch,
        executable: form.executable.to_string(),
        executable_edited: form.executable_edited,
        arguments: form.arguments.to_string(),
        script: form.script.to_string(),
        shell: form.shell,
        active_path: form.active_path,
        other_path: form.other_path,
        file_path: form.file_path,
        file_name: form.file_name,
        file_stem: form.file_stem,
        file_extension: form.file_extension,
    }
}
