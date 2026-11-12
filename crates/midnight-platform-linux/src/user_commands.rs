//! Native argv and explicit cwd. No program launch is reconstructed as shell source.
use crate::LinuxFileSystem;
use midnight_core::*;
use std::{
    ffi::OsString,
    fs, io,
    path::Path,
    process::{Command, Stdio},
};

impl UserCommandService for LinuxFileSystem {
    fn validate_user_command(
        &self,
        process: &UserCommandProcess,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        process.check(cancel)?;
        match &process.invocation {
            UserCommandProcessInvocation::Program {
                executable,
                arguments,
            } => {
                prepare(executable, arguments, &process.working_directory, cancel)?;
                Ok(())
            }
            UserCommandProcessInvocation::Terminal {
                shell,
                script,
                environment,
            } => {
                crate::desktop::prepare_user_terminal(
                    &TerminalRequest {
                        command: script.clone(),
                        working_directory: process.working_directory.clone(),
                        shell: *shell,
                        mode: TerminalMode::Standard,
                    },
                    environment,
                    cancel,
                )?;
                Ok(())
            }
        }
    }
    fn launch_user_program(
        &self,
        executable: &Path,
        arguments: &[OsString],
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        let mut command = prepare(executable, arguments, directory, cancel)?;
        if cancel.is_cancelled() {
            return Err(error(directory, io::ErrorKind::Interrupted.into()));
        }
        command
            .spawn()
            .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
            .map_err(|cause| error(executable, cause))
    }
}
fn prepare(
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
    cancel: &Cancellation,
) -> Result<Command, FsError> {
    UserCommandProcess {
        working_directory: directory.to_owned(),
        invocation: UserCommandProcessInvocation::Program {
            executable: executable.to_owned(),
            arguments: arguments.to_owned(),
        },
    }
    .check(cancel)?;
    crate::editor::validate_program(executable)?;
    if !fs::metadata(directory)
        .map_err(|cause| error(directory, cause))?
        .is_dir()
    {
        return Err(error(
            directory,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Command cwd is not a directory.",
            ),
        ));
    }
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    validate_os_arguments(&command).map_err(|cause| error(directory, cause))?;
    Ok(command)
}

pub(crate) fn validate_os_arguments(command: &Command) -> io::Result<()> {
    use std::{collections::HashMap, os::unix::ffi::OsStrExt};
    let (maximum, page) = argument_limits();
    if maximum <= 0 || page <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Native argv limits are unavailable.",
        ));
    }
    let string_limit = (page as usize).saturating_mul(32);
    let mut environment = std::env::vars_os().collect::<HashMap<_, _>>();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            environment.insert(name.to_owned(), value.to_owned());
        } else {
            environment.remove(name);
        }
    }
    let mut bytes = 0usize;
    let mut count = 2usize;
    for argument in std::iter::once(command.get_program()).chain(command.get_args()) {
        let length = argument.as_bytes().len().saturating_add(1);
        if length > string_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Native argument exceeds Linux's per-string limit.",
            ));
        }
        bytes = bytes.saturating_add(length);
        count += 1;
    }
    for (key, value) in environment {
        let length = key
            .as_bytes()
            .len()
            .saturating_add(value.as_bytes().len())
            .saturating_add(2);
        if length > string_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Native environment value exceeds Linux's per-string limit.",
            ));
        }
        bytes = bytes.saturating_add(length);
        count += 1;
    }
    bytes = bytes.saturating_add(count.saturating_mul(std::mem::size_of::<*const u8>()));
    if bytes > maximum as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Native argv/environment exceeds the current Linux ARG_MAX budget.",
        ));
    }
    Ok(())
}
fn error(path: &Path, cause: io::Error) -> FsError {
    FsError::from_io("Custom Command", path, cause)
}

#[allow(
    unsafe_code,
    reason = "Thread-safe POSIX scalar queries isolated inside the native adapter."
)]
fn argument_limits() -> (libc::c_long, libc::c_long) {
    // SAFETY: sysconf reads scalar process/kernel limits and borrows no pointers.
    unsafe {
        (
            libc::sysconf(libc::_SC_ARG_MAX),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    }
}
