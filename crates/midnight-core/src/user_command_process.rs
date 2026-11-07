//! Validated native process data; no joined argv or template source is executable here.
use crate::{
    Cancellation, FsError, ShellFlavor, TerminalMode, TerminalProcess, TerminalRequest,
    TerminalService,
};
use std::{
    collections::HashSet,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

pub const USER_COMMAND_ENVIRONMENT: [&str; 6] = [
    "ME_ACTIVE_PATH",
    "ME_OTHER_PATH",
    "ME_FILE_PATH",
    "ME_FILE_NAME",
    "ME_FILE_STEM",
    "ME_FILE_EXTENSION",
];
#[derive(Clone)]
pub enum UserCommandProcessInvocation {
    Program {
        executable: PathBuf,
        arguments: Vec<OsString>,
    },
    Terminal {
        shell: ShellFlavor,
        script: String,
        environment: Vec<(String, OsString)>,
    },
}
impl std::fmt::Debug for UserCommandProcessInvocation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Program { arguments, .. } => output
                .debug_struct("Program")
                .field("argument_count", &arguments.len())
                .finish_non_exhaustive(),
            Self::Terminal {
                shell,
                script,
                environment,
            } => output
                .debug_struct("Terminal")
                .field("shell", shell)
                .field("source_bytes", &script.len())
                .field("environment_count", &environment.len())
                .finish_non_exhaustive(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct UserCommandProcess {
    pub working_directory: PathBuf,
    pub invocation: UserCommandProcessInvocation,
}
impl UserCommandProcess {
    /// Shared native preflight; adapters add executable/profile/OS argv validation.
    pub fn check(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.validate().map_err(|message| {
            FsError::from_io(
                "Custom Command",
                &self.working_directory,
                io::Error::new(io::ErrorKind::InvalidInput, message),
            )
        })?;
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "Custom Command",
                &self.working_directory,
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Command cancelled before launch.",
                ),
            ));
        }
        if matches!(
            self.invocation,
            UserCommandProcessInvocation::Terminal {
                shell: ShellFlavor::Cmd,
                ..
            }
        ) {
            return Err(FsError::from_io(
                "Custom Command terminal",
                &self.working_directory,
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Cmd Custom Commands await hostile native environment-reference qualification. Choose Program or PowerShell.",
                ),
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let native =
            |path: &Path| path.is_absolute() && !path.as_os_str().as_encoded_bytes().contains(&0);
        if !native(&self.working_directory) {
            return Err("Command working directory must be an absolute native path without NUL.");
        }
        let mut bytes = self
            .working_directory
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .saturating_add(std::mem::size_of::<Self>());
        match &self.invocation {
            UserCommandProcessInvocation::Program {
                executable,
                arguments,
            } => {
                if !native(executable)
                    || arguments.len() > crate::MAX_USER_COMMAND_EXPANDED_ARGUMENTS
                {
                    return Err(
                        "Program executable must be absolute and expanded argv must contain at most 4160 arguments.",
                    );
                }
                bytes = bytes.saturating_add(executable.as_os_str().as_encoded_bytes().len());
                bytes = bytes.saturating_add(
                    arguments
                        .capacity()
                        .saturating_mul(std::mem::size_of::<OsString>()),
                );
                for argument in arguments {
                    if argument.as_encoded_bytes().contains(&0) {
                        return Err("Native arguments cannot contain NUL.");
                    }
                    bytes = bytes.saturating_add(argument.as_encoded_bytes().len());
                }
            }
            UserCommandProcessInvocation::Terminal {
                script,
                environment,
                ..
            } => {
                if script.trim().is_empty() || script.contains('\0') {
                    return Err("Terminal source must be nonempty without NUL.");
                }
                bytes = bytes.saturating_add(script.len());
                bytes = bytes.saturating_add(
                    environment
                        .capacity()
                        .saturating_mul(std::mem::size_of::<(String, OsString)>()),
                );
                let mut keys = HashSet::new();
                for (key, value) in environment {
                    if !USER_COMMAND_ENVIRONMENT.contains(&key.as_str())
                        || !keys.insert(key)
                        || value.as_encoded_bytes().contains(&0)
                    {
                        return Err(
                            "Only unique fixed Custom Command environment keys without NUL are allowed.",
                        );
                    }
                    bytes = bytes
                        .saturating_add(key.len())
                        .saturating_add(value.as_encoded_bytes().len());
                }
            }
        }
        if bytes > 8 * 1024 * 1024 {
            return Err("Native process data exceeds its 8 MiB workload budget.");
        }
        Ok(())
    }
}

pub trait UserCommandService: TerminalService {
    /// Backend preflight of every expanded plan before the first per-item spawn.
    fn validate_user_command(
        &self,
        process: &UserCommandProcess,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        process.check(cancel)
    }
    /// Called on a backend worker with separate native argv and explicit cwd.
    fn launch_user_program(
        &self,
        executable: &Path,
        arguments: &[OsString],
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError>;

    fn launch_user_command(
        &self,
        process: &UserCommandProcess,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.validate_user_command(process, cancel)?;
        match &process.invocation {
            UserCommandProcessInvocation::Program {
                executable,
                arguments,
            } => {
                self.launch_user_program(executable, arguments, &process.working_directory, cancel)
            }
            UserCommandProcessInvocation::Terminal {
                shell: ShellFlavor::Cmd,
                ..
            } => Err(FsError::from_io(
                "Custom Command terminal",
                &process.working_directory,
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Cmd Custom Commands await hostile native environment-reference qualification. Choose Program or PowerShell.",
                ),
            )),
            UserCommandProcessInvocation::Terminal {
                shell,
                script,
                environment,
            } => self.launch_terminal_with_environment(
                &TerminalRequest {
                    command: script.clone(),
                    working_directory: process.working_directory.clone(),
                    shell: *shell,
                    mode: TerminalMode::Standard,
                },
                environment,
                cancel,
            ),
        }
    }
}
