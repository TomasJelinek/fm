//! One pending native picker, bound to the original field and draft revision.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Editor,
    EditorWorkingDirectory,
    ComparatorProgram,
    ComparatorWorkingDirectory,
    CommandProgram,
    WorkingDirectory,
    CommandIcon,
    CommandIconResource,
}
impl Target {
    fn comparator(self) -> bool {
        matches!(
            self,
            Self::ComparatorProgram | Self::ComparatorWorkingDirectory
        )
    }
}
struct Pending {
    id: DesktopRequestId,
    target: Target,
    revision: u64,
    token: String,
    configuration: std::sync::Weak<Vec<UserCommand>>,
    comparator_context: Option<(u64, u64, super::comparator_draft::RowId)>,
    cancelled: bool,
}
#[derive(Default)]
pub(crate) struct State {
    pending: Option<Pending>,
    pub(crate) poll_scheduled: bool,
}
impl State {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn invalidate(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.cancelled = true;
        }
    }
}

#[cfg(test)]
#[path = "settings_picker_tests.rs"]
mod tests;

impl Presenter {
    pub(crate) fn request_settings_picker(
        &mut self,
        target: Target,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.settings_picker.busy() || !self.preferences.loaded || self.operations.closing {
            return;
        }
        let comparator_context = if target.comparator() {
            let Some(context) = self.comparator_picker_context() else {
                return;
            };
            Some(context)
        } else {
            None
        };
        let (kind, title, initial, revision, token) = match target {
            Target::ComparatorProgram | Target::ComparatorWorkingDirectory => {
                let Some(rule) = self
                    .comparators
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.selected_rule())
                else {
                    return;
                };
                let directory = target == Target::ComparatorWorkingDirectory;
                let external = &rule.external;
                let initial = if directory {
                    external.native_working_directory.clone().or_else(|| {
                        let path = PathBuf::from(&external.working_directory);
                        path.is_absolute().then_some(path)
                    })
                } else {
                    external
                        .native_executable
                        .as_deref()
                        .and_then(Path::parent)
                        .map(Path::to_path_buf)
                        .or_else(|| {
                            let path = PathBuf::from(&external.executable);
                            path.is_absolute()
                                .then(|| path.parent().map(Path::to_path_buf))
                                .flatten()
                        })
                };
                (
                    if directory {
                        PathPickerKind::Directory
                    } else {
                        PathPickerKind::Program
                    },
                    if directory {
                        "Choose comparator working directory"
                    } else {
                        "Choose comparator program"
                    },
                    initial,
                    self.comparators.edit_revision,
                    String::new(),
                )
            }
            Target::Editor | Target::EditorWorkingDirectory => {
                let draft = self.editor_draft();
                let form = draft.selected_form();
                if target == Target::EditorWorkingDirectory && draft.selected.is_none() {
                    return;
                }
                let initial = if target == Target::EditorWorkingDirectory {
                    form.native_working_directory.clone().or_else(|| {
                        let path = PathBuf::from(&form.working_directory);
                        path.is_absolute().then_some(path)
                    })
                } else {
                    form.native
                        .as_deref()
                        .and_then(Path::parent)
                        .map(Path::to_path_buf)
                };
                self.editor.draft = Some(draft);
                (
                    if target == Target::Editor {
                        PathPickerKind::Program
                    } else {
                        PathPickerKind::Directory
                    },
                    if target == Target::Editor {
                        "Choose editor"
                    } else {
                        "Choose editor working directory"
                    },
                    initial,
                    self.editor.revision,
                    String::new(),
                )
            }
            target => {
                let Some(draft) = self.user_command_settings_form() else {
                    return;
                };
                if !self.user_command_settings_current() {
                    return;
                }
                let original = self
                    .preferences
                    .values
                    .user_commands
                    .iter()
                    .find(|command| command.id.as_str() == draft.id);
                let (kind, title, initial) = match target {
                    Target::CommandProgram => {
                        let path = self
                            .user_command_settings
                            .picked_executable
                            .as_deref()
                            .or_else(|| {
                                original.and_then(|command| match &command.invocation {
                                    UserCommandInvocation::Program { executable, .. } => {
                                        Some(executable.as_path())
                                    }
                                    _ => None,
                                })
                            });
                        (
                            PathPickerKind::Program,
                            "Choose command",
                            path.and_then(Path::parent).map(Path::to_path_buf),
                        )
                    }
                    Target::CommandIcon => {
                        let path = self
                            .user_command_settings
                            .picked_icon
                            .as_deref()
                            .or_else(|| original.and_then(|c| c.icon.as_deref()));
                        (
                            PathPickerKind::CommandIconImage,
                            "Choose toolbar image",
                            path.and_then(Path::parent).map(Path::to_path_buf),
                        )
                    }
                    Target::CommandIconResource => (
                        PathPickerKind::CommandIconResource,
                        "Choose toolbar icon",
                        None,
                    ),
                    Target::WorkingDirectory => (
                        PathPickerKind::Directory,
                        "Choose initial directory",
                        Some(PathBuf::from(&draft.working_directory)),
                    ),
                    Target::Editor
                    | Target::EditorWorkingDirectory
                    | Target::ComparatorProgram
                    | Target::ComparatorWorkingDirectory => unreachable!(),
                };
                (
                    kind,
                    title,
                    initial.filter(|p| p.is_absolute()),
                    self.user_command_settings.edit_revision,
                    self.user_command_settings.token(),
                )
            }
        };
        let id = self.desktop.request();
        if send(Command::PickPath {
            request: id,
            input: PathPickerRequest {
                kind,
                title: title.into(),
                initial_directory: initial,
                owner,
            },
        }) {
            self.settings_picker.pending = Some(Pending {
                id,
                target,
                revision,
                token,
                configuration: Arc::downgrade(&self.preferences.values.user_commands),
                comparator_context,
                cancelled: false,
            });
        } else {
            let error = "The file picker queue is busy. Try Browse again.";
            if target.comparator() {
                self.comparators.error = error.into();
                self.comparators.error_field = -1;
                self.comparators.revision = self.comparators.revision.saturating_add(1);
            } else if matches!(target, Target::Editor | Target::EditorWorkingDirectory) {
                self.editor.error = error.into();
            } else {
                self.user_command_settings.error = error.into();
            }
        }
    }
    pub(crate) fn settings_picker_context(&mut self, page: i32, send: &dyn Fn(Command) -> bool) {
        if let Some(pending) = &mut self.settings_picker.pending {
            let expected = if pending.target.comparator() {
                10
            } else if matches!(
                pending.target,
                Target::Editor | Target::EditorWorkingDirectory
            ) {
                4
            } else {
                9
            };
            if page != expected {
                pending.cancelled = true;
            }
        }
        self.flush_settings_picker(send);
    }
    pub(crate) fn flush_settings_picker(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(pending) = &mut self.settings_picker.pending else {
            return;
        };
        pending.cancelled |= self.operations.closing;
        if pending.cancelled
            && send(Command::CancelPathPicker {
                request: pending.id,
            })
        {
            self.settings_picker.pending = None;
        }
    }
    pub(crate) fn settings_path_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        if self
            .settings_picker
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != request)
        {
            self.retired.push(Box::new(result));
            return;
        }
        let pending = self.settings_picker.pending.take().unwrap();
        let current = !pending.cancelled
            && !self.operations.closing
            && match pending.target {
                Target::ComparatorProgram | Target::ComparatorWorkingDirectory => {
                    pending.comparator_context.is_some()
                        && pending.comparator_context == self.comparator_picker_context()
                }
                Target::Editor | Target::EditorWorkingDirectory => {
                    pending.revision == self.editor.revision
                        && self
                            .editor
                            .draft
                            .as_ref()
                            .is_some_and(|d| d.original == self.preferences.values.editor)
                }
                _ => {
                    pending.revision == self.user_command_settings.edit_revision
                        && pending.token == self.user_command_settings.token()
                        && pending.configuration.as_ptr()
                            == Arc::as_ptr(&self.preferences.values.user_commands)
                }
            };
        if !current {
            self.retired.push(Box::new(result));
            return;
        }
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                let message = crate::error_presentation::describe(&error);
                if pending.target.comparator() {
                    self.comparators.error = message;
                    self.comparators.error_field = -1;
                    self.comparators.revision = self.comparators.revision.saturating_add(1);
                } else if matches!(
                    pending.target,
                    Target::Editor | Target::EditorWorkingDirectory
                ) {
                    self.editor.error = message;
                } else {
                    self.user_command_settings.error = message;
                }
                return;
            }
        };
        if !path.is_absolute() {
            return;
        }
        if pending.target.comparator() {
            self.comparator_path_picked(path, pending.target == Target::ComparatorWorkingDirectory);
            return;
        }
        if matches!(
            pending.target,
            Target::Editor | Target::EditorWorkingDirectory
        ) {
            let mut draft = self.editor_draft();
            draft.picked(path, pending.target == Target::EditorWorkingDirectory);
            self.editor.draft = Some(draft);
            self.editor.dirty = true;
            self.editor.error.clear();
            self.editor.revision = self.editor.revision.saturating_add(1);
            return;
        }
        let Some(mut draft) = self.user_command_settings_form() else {
            return;
        };
        match pending.target {
            Target::CommandProgram => {
                draft.executable = path.to_string_lossy().into_owned();
                draft.executable_edited = true;
                self.user_command_settings.picked_executable = Some(path);
            }
            Target::CommandIcon | Target::CommandIconResource => {
                draft.icon = path.to_string_lossy().into_owned();
                draft.icon_edited = true;
                self.user_command_settings.picked_icon = Some(path);
            }
            Target::WorkingDirectory => {
                let Some(text) = path.to_str() else {
                    self.user_command_settings.error = "This directory cannot be stored as literal text. Choose Active folder, or another directory.".into();
                    return;
                };
                // Dollar signs in a picked literal must not become template substitutions.
                draft.working_directory = text.replace('$', "$$");
            }
            Target::Editor
            | Target::EditorWorkingDirectory
            | Target::ComparatorProgram
            | Target::ComparatorWorkingDirectory => unreachable!(),
        }
        self.remember_user_command_form(&pending.token, draft);
        self.user_command_settings.draft_revision =
            self.user_command_settings.draft_revision.saturating_add(1);
        if let Some(form) = self.user_command_settings_form() {
            let token = self.user_command_settings.token();
            self.validate_user_command_setting(&token, &form);
        }
    }
}
