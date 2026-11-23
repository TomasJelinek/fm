//! Output windows of Run directly commands: which sessions have one and what the user
//! does in it. The windows read their session themselves (`command_output_windows`);
//! nothing here reads a pipe or waits for a program.
use super::*;

#[derive(Default)]
pub(crate) struct CommandOutputs {
    pub(crate) windows: Vec<OutputWindow>,
}

pub(crate) struct OutputWindow {
    pub(crate) session: Arc<UserCommandOutputSession>,
    /// Changes with every notification and action; a window reads the session again
    /// when it differs from the revision it showed.
    pub(crate) revision: u64,
    /// The result of the last Copy all or Save as, shown in the window's footer.
    pub(crate) status: String,
    pub(crate) save: Option<OutputSave>,
}

/// Save as… of one window, from the picker to the saved file.
pub(crate) enum OutputSave {
    Picking(DesktopRequestId),
    /// Picked (or confirmed): waits for the next sender.
    Ready {
        path: PathBuf,
        overwrite: bool,
    },
    Saving {
        request: DesktopRequestId,
        path: PathBuf,
        overwrite: bool,
    },
    /// The file exists: the window asks whether to replace it.
    Confirm(PathBuf),
}

impl OutputWindow {
    /// The question the window asks, if any.
    pub(crate) fn save_prompt(&self) -> Option<String> {
        match &self.save {
            Some(OutputSave::Confirm(path)) => Some(format!(
                "{} already exists. Replace it?",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            )),
            _ => None,
        }
    }
    pub(crate) fn saving(&self) -> bool {
        self.save.is_some()
    }
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Presenter {
    pub(super) fn apply_command_output(&mut self, event: UserCommandOutputEvent) {
        let outputs = &mut self.command_launches.outputs;
        if let Some(window) = outputs
            .windows
            .iter_mut()
            .find(|window| Arc::ptr_eq(&window.session, &event.session))
        {
            window.revision = window.revision.wrapping_add(1);
        } else if event.opened {
            if outputs.windows.len() >= MAX_OUTPUT_SESSIONS {
                // No window could show it, so nothing may run unseen.
                event.session.request_stop();
                let name = event.session.name.clone();
                self.desktop.report(
                    "Custom Commands",
                    format!(
                        "{name} was not run: {MAX_OUTPUT_SESSIONS} output windows are open. \
                         Close one of them and run the command again."
                    ),
                );
            } else {
                outputs.windows.push(OutputWindow {
                    session: event.session.clone(),
                    revision: 1,
                    status: String::new(),
                    save: None,
                });
            }
        }
        // A closed window's late news: the session may hold megabytes of output.
        self.retired.push(Box::new(event));
    }

    pub(crate) fn command_output(&self, request: UserCommandRequestId) -> Option<&OutputWindow> {
        self.command_launches
            .outputs
            .windows
            .iter()
            .find(|window| window.session.request == request)
    }

    fn command_output_mut(&mut self, request: UserCommandRequestId) -> Option<&mut OutputWindow> {
        self.command_launches
            .outputs
            .windows
            .iter_mut()
            .find(|window| window.session.request == request)
    }

    /// Stop: end the running program's process tree and start no further run.
    pub(crate) fn stop_command_output(
        &mut self,
        request: UserCommandRequestId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(window) = self.command_output_mut(request) else {
            return;
        };
        if window.session.is_finished() {
            return;
        }
        window.session.request_stop();
        window.revision = window.revision.wrapping_add(1);
        // The session's own Stop is enough; this also ends the Jobs entry's launches.
        let _ = send(Command::CancelUserCommand(request));
    }

    /// The window closed: a program that still runs is stopped with it.
    pub(crate) fn close_command_output(
        &mut self,
        request: UserCommandRequestId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let outputs = &mut self.command_launches.outputs.windows;
        let Some(index) = outputs
            .iter()
            .position(|window| window.session.request == request)
        else {
            return;
        };
        let window = outputs.remove(index);
        if !window.session.is_finished() {
            window.session.request_stop();
            let _ = send(Command::CancelUserCommand(request));
        }
        self.retired.push(Box::new(window.session));
    }

    /// Clear: forget the shown lines (Copy all then holds only later ones).
    pub(crate) fn clear_command_output(&mut self, request: UserCommandRequestId) {
        if let Some(window) = self.command_output_mut(request) {
            window.session.clear();
            window.status.clear();
            window.revision = window.revision.wrapping_add(1);
        }
    }

    /// Copy all: every retained line as text, on the backend's clipboard worker.
    pub(crate) fn copy_command_output(
        &mut self,
        request: UserCommandRequestId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(window) = self.command_output(request) else {
            return;
        };
        let text = window.session.text();
        let lines = text.lines().count();
        let status = if lines == 0 {
            "Nothing to copy.".to_owned()
        } else {
            self.copy_viewer_text(text, send);
            format!(
                "Copied {} {}.",
                self.number_locale.format_u64(lines as u64),
                if lines == 1 { "line" } else { "lines" }
            )
        };
        if let Some(window) = self.command_output_mut(request) {
            window.status = status;
            window.revision = window.revision.wrapping_add(1);
        }
    }

    /// The command's "Close when finished successfully" choice: None when the command
    /// no longer exists or no longer uses an output window.
    pub(crate) fn command_output_closes_on_success(
        &self,
        request: UserCommandRequestId,
    ) -> Option<bool> {
        let window = self.command_output(request)?;
        self.preferences
            .values
            .user_commands
            .iter()
            .find(|command| command.id == window.session.command)
            .filter(|command| command.output.mode == UserCommandOutputMode::Window)
            .map(|command| command.output.close_on_success)
    }

    /// Remember the choice with the command, as Options › Custom Commands would.
    pub(crate) fn set_command_output_close_on_success(
        &mut self,
        request: UserCommandRequestId,
        close: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.command_output_closes_on_success(request) != Some(!close) {
            return;
        }
        let Some(id) = self
            .command_output(request)
            .map(|window| window.session.command.clone())
        else {
            return;
        };
        let mut values = self.preferences.values.clone();
        let mut commands = (*values.user_commands).clone();
        if let Some(command) = commands.iter_mut().find(|command| command.id == id) {
            command.output.close_on_success = close;
        }
        values.user_commands = Arc::new(commands);
        self.update_preferences(values, send);
        if let Some(window) = self.command_output_mut(request) {
            window.revision = window.revision.wrapping_add(1);
        }
    }
}

impl Presenter {
    /// Save as…: ask for a destination with the native picker.
    pub(crate) fn save_command_output(
        &mut self,
        id: UserCommandRequestId,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.command_output(id).is_none_or(OutputWindow::saving) {
            return;
        }
        let request = self.desktop.request();
        let sent = send(Command::PickPath {
            request,
            input: PathPickerRequest {
                kind: PathPickerKind::CommandOutputSave,
                title: "Save command output".into(),
                initial_directory: None,
                owner,
            },
        });
        if let Some(window) = self.command_output_mut(id) {
            if sent {
                window.save = Some(OutputSave::Picking(request));
                window.status.clear();
            } else {
                window.status = "The request queue is busy. Try Save as again.".into();
            }
            window.touch();
        }
    }

    pub(crate) fn owns_output_picker(&self, request: DesktopRequestId) -> bool {
        self.command_launches.outputs.windows.iter().any(
            |window| matches!(window.save, Some(OutputSave::Picking(picker)) if picker == request),
        )
    }

    pub(crate) fn output_path_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        let Some(window) = self.command_launches.outputs.windows.iter_mut().find(
            |window| matches!(window.save, Some(OutputSave::Picking(picker)) if picker == request),
        ) else {
            return;
        };
        match result {
            Ok(Some(path)) => {
                window.save = Some(OutputSave::Ready {
                    path,
                    overwrite: false,
                });
            }
            Ok(None) => window.save = None,
            Err(error) => {
                window.save = None;
                window.status = crate::error_presentation::describe(&error);
            }
        }
        window.touch();
    }

    /// Send the saves that wait for a sender (picked or confirmed destinations).
    pub(crate) fn flush_output_saves(&mut self, send: &dyn Fn(Command) -> bool) {
        let mut requests = Vec::new();
        for (index, window) in self.command_launches.outputs.windows.iter().enumerate() {
            if let Some(OutputSave::Ready { path, overwrite }) = &window.save {
                requests.push((index, path.clone(), *overwrite));
            }
        }
        for (index, path, overwrite) in requests {
            let request = self.desktop.request();
            let window = &mut self.command_launches.outputs.windows[index];
            let sent = send(Command::SaveUserCommandOutput(Box::new(
                UserCommandOutputSaveRequest {
                    request,
                    session: window.session.clone(),
                    destination: path.clone(),
                    overwrite,
                },
            )));
            window.save = sent.then_some(OutputSave::Saving {
                request,
                path,
                overwrite,
            });
            if !sent {
                window.status = "The request queue is busy. Try Save as again.".into();
            }
            window.touch();
        }
    }

    /// The answer to "Replace it?".
    pub(crate) fn confirm_command_output_save(
        &mut self,
        id: UserCommandRequestId,
        replace: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(window) = self.command_output_mut(id) else {
            return;
        };
        let Some(OutputSave::Confirm(path)) = window.save.take() else {
            return;
        };
        if replace {
            window.save = Some(OutputSave::Ready {
                path,
                overwrite: true,
            });
        } else {
            window.status = "Not saved.".into();
        }
        window.touch();
        self.flush_output_saves(send);
    }

    pub(super) fn apply_output_saved(
        &mut self,
        request: DesktopRequestId,
        result: Result<UserCommandOutputSaveReceipt, FsError>,
    ) {
        let locale = self.number_locale.clone();
        let Some(window) = self.command_launches.outputs.windows.iter_mut().find(|window| {
            matches!(&window.save, Some(OutputSave::Saving { request: saving, .. }) if *saving == request)
        }) else {
            return;
        };
        let Some(OutputSave::Saving {
            path, overwrite, ..
        }) = window.save.take()
        else {
            return;
        };
        match result {
            Ok(receipt) => {
                window.status = format!(
                    "Saved {} bytes to {}.",
                    locale.format_u64(receipt.bytes),
                    receipt.path.display()
                );
                if let Some(kept) = receipt.recovery.first() {
                    window.status.push_str(&format!(
                        " The replaced file is kept as {}.",
                        kept.display()
                    ));
                }
            }
            Err(error) if error.kind == std::io::ErrorKind::AlreadyExists && !overwrite => {
                window.save = Some(OutputSave::Confirm(path));
            }
            Err(error) => window.status = crate::error_presentation::describe(&error),
        }
        window.touch();
    }
}

#[cfg(test)]
#[path = "command_outputs_tests.rs"]
mod tests;
