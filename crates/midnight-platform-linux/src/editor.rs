//! Graphical editor discovery uses fixed absolute locations, never the panel CWD or PATH.
use crate::LinuxFileSystem;
use midnight_core::{EditorCommand, EditorPreferences, EditorService, FsError, TerminalProcess};
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

const GRAPHICAL_EDITORS: &[&str] = &[
    "/usr/bin/gnome-text-editor",
    "/usr/bin/gedit",
    "/usr/bin/kate",
    "/usr/bin/kwrite",
    "/usr/bin/mousepad",
    "/usr/bin/xed",
    "/usr/bin/pluma",
    "/usr/bin/leafpad",
];

impl EditorService for LinuxFileSystem {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        discover(GRAPHICAL_EDITORS.iter().map(Path::new))
    }
    fn launch_editor(
        &self,
        file: &Path,
        editor: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        prepare(self, file, editor)?
            .spawn()
            .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
            .map_err(|cause| error(file, cause))
    }
}

fn discover<'a>(candidates: impl Iterator<Item = &'a Path>) -> Result<EditorCommand, FsError> {
    for path in candidates {
        if validate_program(path).is_ok() {
            return Ok(EditorCommand {
                executable: path.into(),
                arguments: vec!["--".into(), "{file}".into()],
            });
        }
    }
    Err(error(
        Path::new(""),
        io::Error::new(
            io::ErrorKind::NotFound,
            "No supported graphical editor is installed. Set an absolute editor executable in Options > Editors.",
        ),
    ))
}

fn prepare(
    service: &LinuxFileSystem,
    file: &Path,
    editor: &EditorPreferences,
) -> Result<Command, FsError> {
    editor
        .validate()
        .map_err(|cause| error(file, io::Error::new(io::ErrorKind::InvalidInput, cause)))?;
    if !file.is_absolute() || file.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(error(
            file,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Editing requires a local absolute file path.",
            ),
        ));
    }
    if !fs::symlink_metadata(file)
        .map_err(|cause| error(file, cause))?
        .file_type()
        .is_file()
    {
        return Err(error(
            file,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Only regular local files can be edited.",
            ),
        ));
    }
    let selected = midnight_platform_common::editor_rules::select(editor, file)
        .map_err(|cause| error(file, io::Error::new(io::ErrorKind::InvalidInput, cause)))?;
    let spec = match selected.command {
        Some(command) => command,
        None => service.default_editor()?,
    };
    validate_program(&spec.executable)?;
    let arguments = spec
        .arguments_for(file)
        .map_err(|cause| error(file, io::Error::new(io::ErrorKind::InvalidInput, cause)))?;
    let mut command = Command::new(&spec.executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = selected
        .working_directory
        .as_deref()
        .or_else(|| spec.executable.parent())
    {
        command.current_dir(directory);
    }
    Ok(command)
}

pub(crate) fn validate_program(executable: &Path) -> Result<(), FsError> {
    if !executable.is_absolute() || executable.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(error(
            executable,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Editor executable must be an absolute path without NUL.",
            ),
        ));
    }
    let metadata = fs::metadata(executable).map_err(|cause| error(executable, cause))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(error(
            executable,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Editor executable must be an executable regular file.",
            ),
        ));
    }
    Ok(())
}

fn error(path: &Path, cause: io::Error) -> FsError {
    FsError::from_io("external editor", path, cause)
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
