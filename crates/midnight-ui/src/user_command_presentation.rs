//! Configured command toolbar and bounded launch details. Native work stays in services.
use crate::*;
use std::{
    collections::HashSet,
    rc::Weak,
    sync::{Arc, Weak as SyncWeak},
};

#[cfg(test)]
#[path = "user_command_presentation_tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct ToolbarState {
    configuration: RefCell<SyncWeak<Vec<UserCommand>>>,
    pub(crate) revision: Rc<Cell<u64>>,
    pub(crate) image_revision: Cell<u64>,
    visible: Rc<RefCell<HashSet<(String, String)>>>,
    pending_preview: Rc<Cell<u64>>,
}
impl ToolbarState {
    pub(crate) fn invalidate_preview(&self) {
        self.pending_preview.set(0);
    }
    fn sync(&self, configuration: &Arc<Vec<UserCommand>>) -> bool {
        if self
            .configuration
            .borrow()
            .upgrade()
            .is_some_and(|old| Arc::ptr_eq(&old, configuration))
        {
            return false;
        }
        self.revision.set(self.revision.get().saturating_add(1));
        *self.configuration.borrow_mut() = Arc::downgrade(configuration);
        self.visible.borrow_mut().clear();
        true
    }
}

struct Rows {
    state: Weak<RefCell<Presenter>>,
    notify: ModelNotify,
    structure: Cell<(u64, u64, u64)>,
}
impl Rows {
    fn refresh(&self, p: &Presenter) {
        let current = (
            p.command_toolbar.revision.get(),
            p.command_toolbar.image_revision.get(),
            p.keyboard.snapshot.revision,
        );
        let old = self.structure.replace(current);
        if old.0 != current.0 {
            self.notify.reset();
        } else if old.1 != current.1 || old.2 != current.2 {
            for index in 0..p.preferences.values.user_commands.len() {
                self.notify.row_changed(index);
            }
        }
    }
}
impl Model for Rows {
    type Data = UserCommandRow;
    fn row_count(&self) -> usize {
        self.state
            .upgrade()
            .map_or(0, |p| p.borrow().preferences.values.user_commands.len())
    }
    fn row_data(&self, index: usize) -> Option<Self::Data> {
        let state = self.state.upgrade()?;
        let p = state.borrow();
        let command = p.preferences.values.user_commands.get(index)?;
        let revision = p.command_toolbar.revision.get().to_string();
        Some(UserCommandRow {
            identity: command.id.as_str().into(),
            revision: revision.clone().into(),
            name: command.name.as_str().into(),
            label: command.toolbar_label.as_deref().unwrap_or("").into(),
            enabled: command.enabled,
            hint: command_hint(command, p.shortcut_hint(&command.id.action())).into(),
            icon: p
                .custom_command_icon(&revision, command.id.as_str())
                .unwrap_or_else(|| p.desktop.image(&IconKey::action(ActionIcon::Run))),
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn command_hint(command: &UserCommand, shortcut: &str) -> String {
    let mode = match &command.invocation {
        UserCommandInvocation::Program { .. } => "Program",
        UserCommandInvocation::Terminal { shell, .. } => match shell {
            ShellFlavor::PowerShell => "PowerShell",
            ShellFlavor::Cmd => "Cmd",
            ShellFlavor::Posix => "Terminal",
        },
    };
    let launches = match command.launch {
        UserCommandLaunch::Single => "Once",
        UserCommandLaunch::PerItem => "Once per selected item",
    };
    let mut hint = format!(
        "{}{}\n{mode} · {launches}\nWorking folder: {}",
        command.name,
        if command.enabled { "" } else { " (disabled)" },
        shortened(&command.working_directory),
    );
    if !shortcut.is_empty() {
        hint.push('\n');
        hint.push_str(shortcut);
    }
    hint.push_str("\nRight-click for Custom Commands settings");
    hint
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    window.set_user_commands(
        Rc::new(Rows {
            state: Rc::downgrade(&state),
            notify: ModelNotify::default(),
            structure: Cell::new((0, 0, 0)),
        })
        .into(),
    );
    // Layout can invoke this callback while Presenter is immutably borrowed for
    // refresh. This small independent cell never reenters that borrow.
    let visible = state.borrow().command_toolbar.visible.clone();
    let revision = state.borrow().command_toolbar.revision.clone();
    window.on_user_command_icon_visible(move |id, token, showing| {
        let key = (token.to_string(), id.to_string());
        if showing && token.parse::<u64>().ok() == Some(revision.get()) {
            if visible.borrow().len() < MAX_USER_COMMANDS {
                visible.borrow_mut().insert(key);
            }
        } else {
            visible.borrow_mut().remove(&key);
        }
    });
    let weak = window.as_weak();
    window.on_execute_user_command(move |id, revision| {
        let Some(window) = weak.upgrade() else { return };
        {
            let mut p = state.borrow_mut();
            if !revision.is_empty()
                && revision.parse::<u64>().ok() != Some(p.command_toolbar.revision.get())
            {
                return;
            }
            p.request_user_command(
                pane(window.get_active_pane()),
                id.as_str(),
                false,
                send.as_ref(),
            );
        }
        crate::refresh(&window, &state.borrow());
    });
}

pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    let model = window.get_user_commands();
    if let Some(rows) = model.as_any().downcast_ref::<Rows>() {
        rows.refresh(p);
    }
    // Update this after ordinary job rows in the host refresh, using its deferred
    // show callback so a settings editor can finish closing before Jobs appears.
    if let Some(id) = p
        .command_launches
        .preview_ready
        .filter(|_| !p.operations.closing)
    {
        let token: SharedString = format!("command-job:{}", id.0).into();
        if p.command_toolbar.pending_preview.replace(id.0) != id.0 {
            let weak = window.as_weak();
            let pending = p.command_toolbar.pending_preview.clone();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(window) = weak
                    .upgrade()
                    .filter(|w| w.window().is_visible() && pending.get() == id.0)
                {
                    window.set_command_preview_token(token);
                }
            });
        }
    }
}

impl Presenter {
    pub(crate) fn command_icon_targets(&self) -> Vec<ThumbnailTarget> {
        let revision = self.command_toolbar.revision.get().to_string();
        let visible = self.command_toolbar.visible.borrow();
        self.preferences
            .values
            .user_commands
            .iter()
            .filter_map(|command| {
                let path = command.icon.as_ref()?;
                if !visible.contains(&(revision.clone(), command.id.as_str().to_owned())) {
                    return None;
                }
                Some(ThumbnailTarget {
                    identity: command_icon_identity(&revision, command.id.as_str()),
                    path: path.clone(),
                    selected: false,
                    hovered: false,
                    confirmed_ssd: false,
                })
            })
            .take(MAX_USER_COMMANDS)
            .collect()
    }
}
pub(crate) fn command_icon_identity(revision: &str, id: &str) -> String {
    format!("command-icon:{revision}:{id}")
}

pub(crate) fn job_rows(p: &Presenter) -> Vec<OperationRow> {
    p.command_launches
        .jobs
        .iter()
        .rev()
        .map(|job| {
            let started = p.number_locale.format_u64(job.started);
            let mut detail = format!(
                "{}\n{}\n\n{started} processes started.",
                job.name, job.state
            );
            detail.push_str(concat!(
                " Cancel prevents remaining launches;",
                " it does not terminate programs already started."
            ));
            if let Some(summary) = &job.summary {
                detail.push_str(&format!(
                    "\n{} planned · {} not started",
                    p.number_locale.format_u64(summary.planned),
                    p.number_locale.format_u64(summary.not_started)
                ));
            }
            if let Some(error) = &job.error {
                detail.push_str(&format!("\n\n{}", error_presentation::describe(error)));
            }
            if let Some(preview) = &job.preview {
                append_preview(&mut detail, preview);
            }
            let action = if job.preview_only {
                "Preview"
            } else {
                "Launch"
            };
            OperationRow {
                identity: format!("command-job:{}", job.id.0).into(),
                title: format!("{action}: {}", job.name).into(),
                state: job.state.into(),
                summary: format!("{started} processes started").into(),
                detail: detail.into(),
                cancellable: !job.finished && !job.stopping,
                attention: false,
                failed: job.error.is_some(),
                active: !job.finished,
                determinate: job.finished,
                progress: if job.finished { 1.0 } else { 0.0 },
                icon: p.desktop.image(&IconKey::action(ActionIcon::Run)),
                kind: 6,
            }
        })
        .collect()
}

fn append_preview(detail: &mut String, preview: &UserCommandPreview) {
    detail.push_str(concat!(
        "\n\nPreview only — no process was started by this request.",
        "\nArguments are separate native values, not a shell command string."
    ));
    for (index, row) in preview.processes.iter().take(4).enumerate() {
        let process = &row.process;
        detail.push_str(&format!(
            "\n\nProcess {}\nWorking folder: {}",
            index + 1,
            shortened(&process.working_directory.to_string_lossy())
        ));
        match &process.invocation {
            UserCommandProcessInvocation::Program {
                executable,
                arguments,
            } => {
                detail.push_str(&format!(
                    "\nProgram: {}",
                    shortened(&executable.to_string_lossy())
                ));
                for (index, value) in arguments.iter().take(16).enumerate() {
                    detail.push_str(&format!(
                        "\nArgument {index}: {:?}",
                        shortened(&value.to_string_lossy())
                    ));
                }
                if arguments.len() > 16 {
                    detail.push_str("\n… Additional arguments omitted from this display.");
                }
            }
            UserCommandProcessInvocation::Terminal {
                shell,
                script,
                environment,
            } => {
                detail.push_str(&format!(
                    "\nTerminal: {shell:?}\nLiteral script:\n{}",
                    shortened(script)
                ));
                for (key, value) in environment {
                    detail.push_str(&format!(
                        "\n{key}: {:?}",
                        shortened(&value.to_string_lossy())
                    ));
                }
            }
        }
    }
    if preview.processes.len() > 4 {
        detail.push_str("\n\n… Additional processes omitted from this display.");
    }
}
fn shortened(value: &str) -> std::borrow::Cow<'_, str> {
    if value.len() <= 256 {
        value.into()
    } else {
        format!(
            "{}… [display truncated]",
            &value[..value.floor_char_boundary(256)]
        )
        .into()
    }
}
