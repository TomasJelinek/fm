//! Native Windows editor launch; no PATH lookup, shell expansion, or batch files.
use crate::WindowsFileSystem;
use midnight_core::{EditorCommand, EditorPreferences, EditorService, FsError, TerminalProcess};
use std::{
    fs, io,
    path::Path,
    process::{Command, Stdio},
};

impl EditorService for WindowsFileSystem {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        let executable = crate::desktop::system_directory()
            .map_err(|cause| error(Path::new(""), cause))?
            .join("notepad.exe");
        validate_program(&executable)?;
        Ok(EditorCommand {
            executable,
            arguments: vec!["{file}".into()],
        })
    }
    fn launch_editor(
        &self,
        file: &Path,
        editor: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        let mut command = prepare(self, file, editor)?;
        command
            .spawn()
            .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
            .map_err(|cause| error(file, cause))
    }
}

fn prepare(
    service: &WindowsFileSystem,
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
    let metadata = fs::symlink_metadata(file).map_err(|cause| error(file, cause))?;
    if !metadata.file_type().is_file() {
        return Err(error(
            file,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Only regular local files can be edited.",
            ),
        ));
    }
    let spec = match &editor.executable {
        Some(executable) => EditorCommand {
            executable: executable.clone(),
            arguments: editor.arguments.clone(),
        },
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
    if let Some(parent) = spec.executable.parent() {
        command.current_dir(parent);
    }
    Ok(command)
}

fn validate_program(executable: &Path) -> Result<(), FsError> {
    let extension = executable
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !executable.is_absolute()
        || !["exe", "com"]
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    {
        return Err(error(
            executable,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose an absolute .exe or .com editor path; shell scripts are unsupported.",
            ),
        ));
    }
    if !fs::metadata(executable)
        .map_err(|cause| error(executable, cause))?
        .is_file()
    {
        return Err(error(
            executable,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Editor executable is not a file.",
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
