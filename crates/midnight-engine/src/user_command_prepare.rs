//! Bounded immutable expansion and metadata pins. No process is launched here.
use midnight_core::*;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_PLAN_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Prepared {
    pub(crate) processes: Arc<Vec<UserCommandProcess>>,
    pins: Arc<Pins>,
    scopes: Arc<Vec<Scope>>,
}
struct Scope {
    directories: Vec<Arc<dyn TransferDirectory>>,
    entries: Vec<Arc<dyn TransferTarget>>,
}
impl Prepared {
    pub(crate) fn revalidate_for(
        &self,
        index: usize,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let scope = self
            .scopes
            .get(index)
            .ok_or_else(|| invalid("Command process index is unavailable."))?;
        for directory in &scope.directories {
            with_io(cancel, directory.path(), || directory.revalidate(cancel))?;
        }
        for target in &scope.entries {
            with_io(cancel, &target.entry().item.path, || {
                target.revalidate(cancel)
            })?;
        }
        Ok(())
    }
    pub(crate) fn into_preview(
        self,
        retention: Arc<UserCommandRetention>,
    ) -> Result<Arc<UserCommandPreview>, FsError> {
        let Self {
            processes,
            pins,
            scopes,
        } = self;
        drop(scopes);
        drop(pins);
        let processes = Arc::try_unwrap(processes)
            .map_err(|_| invalid("Preview process ownership changed unexpectedly."))?;
        let rows = processes
            .into_iter()
            .map(|process| {
                Arc::new(UserCommandPreviewRow {
                    process,
                    retention: retention.clone(),
                })
            })
            .collect();
        Ok(Arc::new(UserCommandPreview {
            processes: rows,
            retention,
        }))
    }
    pub(crate) fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        for directory in self.pins.directories.values() {
            with_io(cancel, directory.path(), || directory.revalidate(cancel))?;
        }
        for (path, target) in &self.pins.entries {
            with_io(cancel, path, || target.revalidate(cancel))?;
        }
        Ok(())
    }
}
struct Pins {
    directories: HashMap<PathBuf, Arc<dyn TransferDirectory>>,
    entries: HashMap<PathBuf, Arc<dyn TransferTarget>>,
    bytes: usize,
}
impl Pins {
    fn new() -> Self {
        Self {
            directories: HashMap::new(),
            entries: HashMap::new(),
            bytes: 0,
        }
    }
    fn charge(&mut self, path: &Path) -> Result<(), FsError> {
        self.bytes = self
            .bytes
            .saturating_add(path.as_os_str().as_encoded_bytes().len().saturating_mul(4))
            .saturating_add(1024);
        if self.bytes > MAX_PLAN_BYTES {
            return Err(fail(
                path,
                io::ErrorKind::InvalidInput,
                "Command metadata pins exceed the 8 MiB workload budget.",
            ));
        }
        Ok(())
    }
    fn directory(
        &mut self,
        path: &Path,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        if let Some(directory) = self.directories.get(path) {
            return Ok(directory.clone());
        }
        native_path(path)?;
        self.charge(path)?;
        let directory = with_io(cancel, path, || {
            service.prepare_transfer_destination(path, cancel)
        })?;
        self.directories.insert(path.to_owned(), directory.clone());
        Ok(directory)
    }
    fn entry(
        &mut self,
        path: &Path,
        expected: Option<&Entry>,
        size: Option<u64>,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        native_path(path)?;
        if !self.entries.contains_key(path) {
            self.charge(path)?;
            let parent = path.parent().ok_or_else(|| {
                fail(
                    path,
                    io::ErrorKind::InvalidInput,
                    "A command target requires a parent.",
                )
            })?;
            let name = path.file_name().ok_or_else(|| {
                fail(
                    path,
                    io::ErrorKind::InvalidInput,
                    "Parent/root navigation is not a command entry.",
                )
            })?;
            let directory = self.directory(parent, service, cancel)?;
            let target = with_io(cancel, path, || directory.prepare_target(name, cancel))?
                .ok_or_else(|| {
                    fail(
                        path,
                        io::ErrorKind::NotFound,
                        "Captured command entry disappeared.",
                    )
                })?;
            self.entries.insert(path.to_owned(), target);
        }
        let captured = self
            .entries
            .get(path)
            .ok_or_else(|| fail(path, io::ErrorKind::Other, "Command pin was not retained."))?
            .entry();
        let link = |kind| matches!(kind, EntryKind::Symlink | EntryKind::DirectoryLink);
        if size.is_some_and(|expected| captured.item.metadata.size != Some(expected))
            || expected.is_some_and(|expected| {
                (expected.kind != captured.item.kind
                    && !(link(expected.kind) && link(captured.item.kind)))
                    || expected
                        .metadata
                        .size
                        .is_some_and(|size| captured.item.metadata.size != Some(size))
                    || expected
                        .metadata
                        .modified
                        .is_some_and(|modified| captured.item.metadata.modified != Some(modified))
            })
        {
            return Err(fail(
                path,
                io::ErrorKind::InvalidData,
                "Captured command entry metadata changed.",
            ));
        }
        Ok(())
    }
}

pub(crate) fn prepare(
    request: &UserCommandRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<Prepared, FsError> {
    prepare_inner(request, service, cancel, true)
}
pub(crate) fn prepare_preview(
    request: &UserCommandRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<Prepared, FsError> {
    prepare_inner(request, service, cancel, false)
}
fn prepare_inner(
    request: &UserCommandRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
    execute: bool,
) -> Result<Prepared, FsError> {
    request
        .validate()
        .map_err(|message| fail(Path::new(""), io::ErrorKind::InvalidInput, message))?;
    if cancel.is_cancelled() {
        return Err(fail(
            Path::new(""),
            io::ErrorKind::Interrupted,
            "Command preparation cancelled.",
        ));
    }
    let command = request
        .configuration
        .iter()
        .find(|command| command.id == request.command)
        .ok_or_else(|| {
            fail(
                Path::new(""),
                io::ErrorKind::NotFound,
                "The configured command no longer exists.",
            )
        })?;
    if matches!(command.invocation, UserCommandInvocation::Separator) {
        return Err(invalid("Separators cannot be previewed or executed."));
    }
    if execute && !command.enabled {
        return Err(fail(
            Path::new(""),
            io::ErrorKind::PermissionDenied,
            "This Custom Command is disabled.",
        ));
    }
    let directory = UserCommandTemplate::parse(&command.working_directory).map_err(invalid)?;
    let arguments = match &command.invocation {
        UserCommandInvocation::Program { arguments, .. } => arguments
            .iter()
            .map(|argument| UserCommandTemplate::parse(argument))
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid)?,
        _ => Vec::new(),
    };
    let mut variables = directory.variables().collect::<HashSet<_>>();
    for argument in &arguments {
        variables.extend(argument.variables());
    }
    if let UserCommandInvocation::Terminal {
        variables: declared,
        ..
    } = &command.invocation
    {
        variables.extend(declared.iter().copied());
    }
    let grouped = arguments.iter().any(UserCommandTemplate::is_group);
    let selected = if grouped || command.launch == UserCommandLaunch::PerItem {
        selected(request, cancel)?
    } else {
        Vec::new()
    };
    let mut pins = Pins::new();
    let mut common_directories = Vec::new();
    if variables.contains(&UserCommandVariable::ActivePath) {
        common_directories.push(
            pins.directory(
                request
                    .context
                    .active
                    .as_deref()
                    .ok_or_else(|| invalid("Active native path is unavailable."))?,
                service,
                cancel,
            )?,
        );
    }
    if variables.contains(&UserCommandVariable::OtherPath) {
        common_directories.push(
            pins.directory(
                request
                    .context
                    .other
                    .as_deref()
                    .ok_or_else(|| invalid("Other native path is unavailable."))?,
                service,
                cancel,
            )?,
        );
    }
    for path in &selected {
        let expected = request
            .context
            .focus
            .as_ref()
            .filter(|entry| entry.path == *path);
        let size = request.context.files.as_ref().and_then(|files| {
            files
                .view
                .by_path
                .get(path)
                .and_then(|id| files.view.by_id.get(id))
                .and_then(|position| files.view.file_sizes.get(*position))
                .copied()
                .flatten()
        });
        pins.entry(path, expected, size, service, cancel)?;
    }
    let needs_file = variables.iter().any(|variable| {
        matches!(
            variable,
            UserCommandVariable::FilePath
                | UserCommandVariable::FileName
                | UserCommandVariable::FileStem
                | UserCommandVariable::FileExtension
        )
    });
    let files = if command.launch == UserCommandLaunch::PerItem {
        selected
            .iter()
            .map(|path| Some(path.as_path()))
            .collect::<Vec<_>>()
    } else {
        let focus = request.context.focus.as_ref();
        if needs_file {
            let entry = focus
                .ok_or_else(|| invalid("A captured focused entry is required by this command."))?;
            pins.entry(&entry.path, Some(entry), None, service, cancel)?;
        }
        vec![focus.map(|entry| entry.path.as_path())]
    };
    let mut processes = Vec::with_capacity(files.len());
    let mut scopes = Vec::with_capacity(files.len());
    let mut bytes = 0usize;
    for file in files {
        if cancel.is_cancelled() {
            return Err(fail(
                Path::new(""),
                io::ErrorKind::Interrupted,
                "Command preparation cancelled.",
            ));
        }
        let lookup = |variable| value(variable, request, file);
        let cwd = PathBuf::from(directory.expand(lookup).map_err(invalid)?);
        let cwd_guard = pins.directory(&cwd, service, cancel)?;
        let mut scope = Scope {
            directories: common_directories.clone(),
            entries: Vec::new(),
        };
        if !scope
            .directories
            .iter()
            .any(|directory| directory.path() == cwd)
        {
            scope.directories.push(cwd_guard);
        }
        if grouped {
            scope.entries = selected
                .iter()
                .filter_map(|path| pins.entries.get(path).cloned())
                .collect();
        }
        if (command.launch == UserCommandLaunch::PerItem || needs_file)
            && let Some(file) = file
            && !scope
                .entries
                .iter()
                .any(|entry| entry.entry().item.path == file)
        {
            scope.entries.push(
                pins.entries
                    .get(file)
                    .cloned()
                    .ok_or_else(|| invalid("Command source guard was not retained."))?,
            );
        }
        let invocation = match &command.invocation {
            UserCommandInvocation::Separator => {
                return Err(invalid("Separators cannot be previewed or executed."));
            }
            UserCommandInvocation::Program { executable, .. } => {
                let mut argv = Vec::new();
                for argument in &arguments {
                    if argument.is_group() {
                        if argv.len().saturating_add(selected.len())
                            > MAX_USER_COMMAND_EXPANDED_ARGUMENTS
                        {
                            return Err(invalid(
                                "Grouped command exceeds the 4160 expanded-argument budget; no launch occurred.",
                            ));
                        }
                        argv.extend(selected.iter().map(|path| path.as_os_str().to_owned()));
                    } else {
                        argv.push(argument.expand(lookup).map_err(invalid)?);
                    }
                    if argv.len() > MAX_USER_COMMAND_EXPANDED_ARGUMENTS {
                        return Err(invalid(
                            "Expanded command exceeds 4160 native arguments; no launch occurred.",
                        ));
                    }
                }
                UserCommandProcessInvocation::Program {
                    executable: executable.clone(),
                    arguments: argv,
                }
            }
            UserCommandInvocation::Terminal {
                shell,
                script,
                variables,
            } => {
                let mut environment = Vec::with_capacity(variables.len());
                for variable in variables {
                    let key = variable.environment_key().ok_or_else(|| {
                        invalid("Grouped variables cannot be terminal environment scalars.")
                    })?;
                    let value = lookup(*variable).ok_or_else(|| {
                        invalid("A declared terminal context value is unavailable.")
                    })?;
                    environment.push((key.to_owned(), value.to_owned()));
                }
                UserCommandProcessInvocation::Terminal {
                    shell: *shell,
                    script: script.clone(),
                    environment,
                }
            }
        };
        let process = UserCommandProcess {
            working_directory: cwd,
            invocation,
        };
        process.validate().map_err(invalid)?;
        bytes = bytes.saturating_add(process_bytes(&process));
        if bytes > MAX_PLAN_BYTES {
            return Err(invalid(
                "Prepared commands exceed the 8 MiB aggregate expanded-plan budget; no launch occurred.",
            ));
        }
        processes.push(process);
        scopes.push(scope);
    }
    let prepared = Prepared {
        processes: Arc::new(processes),
        pins: Arc::new(pins),
        scopes: Arc::new(scopes),
    };
    prepared.revalidate(cancel)?;
    Ok(prepared)
}

fn selected(request: &UserCommandRequest, cancel: &Cancellation) -> Result<Vec<PathBuf>, FsError> {
    if request.context.active.is_none() {
        return Err(invalid(
            "Archive/provider command arguments require a future explicit export policy.",
        ));
    }
    let files = request.context.files.as_ref();
    if let Some(files) = files.filter(|files| !files.selection.is_empty()) {
        let paths = crate::file_targets::resolve(&files.selection, &files.view, None, cancel)?;
        if paths.len() != files.selection.len() {
            return Err(invalid(
                "Captured command selection no longer matches its visible directory.",
            ));
        }
        return Ok(paths);
    }
    let focus = request
        .context
        .focus
        .as_ref()
        .ok_or_else(|| invalid("No selected or focused native entries are available."))?;
    if files.is_some_and(|files| {
        files
            .fallback
            .as_ref()
            .is_some_and(|path| *path != focus.path)
    }) {
        return Err(invalid("Captured focus and selection fallback disagree."));
    }
    Ok(vec![focus.path.clone()])
}
fn value<'a>(
    variable: UserCommandVariable,
    request: &'a UserCommandRequest,
    file: Option<&'a Path>,
) -> Option<&'a OsStr> {
    match variable {
        UserCommandVariable::ActivePath => request.context.active.as_deref().map(Path::as_os_str),
        UserCommandVariable::OtherPath => request.context.other.as_deref().map(Path::as_os_str),
        UserCommandVariable::FilePath => file.map(Path::as_os_str),
        UserCommandVariable::FileName => file.and_then(Path::file_name),
        UserCommandVariable::FileStem => file.and_then(Path::file_stem),
        UserCommandVariable::FileExtension => {
            file.map(|path| path.extension().unwrap_or(OsStr::new("")))
        }
        UserCommandVariable::SelectedPaths => None,
    }
}
fn process_bytes(process: &UserCommandProcess) -> usize {
    let bytes = process
        .working_directory
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .saturating_add(1024);
    bytes.saturating_add(match &process.invocation {
        UserCommandProcessInvocation::Program {
            executable,
            arguments,
        } => arguments.iter().fold(
            executable.as_os_str().as_encoded_bytes().len(),
            |sum, argument| {
                sum.saturating_add(argument.as_encoded_bytes().len())
                    .saturating_add(64)
            },
        ),
        UserCommandProcessInvocation::Terminal {
            script,
            environment,
            ..
        } => environment.iter().fold(script.len(), |sum, (key, value)| {
            sum.saturating_add(key.len())
                .saturating_add(value.as_encoded_bytes().len())
                .saturating_add(64)
        }),
    })
}
fn native_path(path: &Path) -> Result<(), FsError> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path.components().next_back().is_some_and(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(fail(
            path,
            io::ErrorKind::InvalidInput,
            "Command targets must be absolute native entries without NUL or parent navigation.",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> FsError {
    fail(Path::new(""), io::ErrorKind::InvalidInput, message)
}
fn fail(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "Custom Command preparation",
        path,
        io::Error::new(kind, message),
    )
}
fn with_io<T>(
    cancel: &Cancellation,
    path: &Path,
    action: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(midnight_work_budget::IO_SLOTS, || cancel.is_cancelled())
        .ok_or_else(|| {
            fail(
                path,
                io::ErrorKind::Interrupted,
                "Command metadata preparation cancelled.",
            )
        })?;
    action()
}
