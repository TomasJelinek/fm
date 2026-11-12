//! Literal native executable/argv launches; terminal source is a separate explicit mode.
use crate::WindowsFileSystem;
use midnight_core::*;
use std::{
    ffi::OsString,
    fs, io,
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

impl UserCommandService for WindowsFileSystem {
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
        let _error_mode = LaunchErrorMode::new().map_err(|cause| error(executable, cause))?;
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
    let spec = crate::terminal_process::LaunchSpec {
        application: executable.to_owned(),
        arguments: arguments.to_owned(),
        raw_tail: None,
        cmd_environment: false,
        working_directory: directory.to_owned(),
    };
    crate::terminal_process::validate(&spec).map_err(|cause| error(directory, cause))?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    Ok(command)
}
fn error(path: &Path, cause: io::Error) -> FsError {
    FsError::from_io("Custom Command", path, cause)
}

/// Thread-local suppression of native loader dialogs; no process-wide state changes.
struct LaunchErrorMode {
    previous: u32,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl LaunchErrorMode {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            GetThreadErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetThreadErrorMode,
        };
        // SAFETY: both calls affect only the current backend worker thread; output is writable.
        let current = unsafe { GetThreadErrorMode() };
        let mut previous = 0;
        // SAFETY: valid documented flags, preserving the thread's existing modes.
        if unsafe {
            SetThreadErrorMode(
                current | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX,
                &mut previous,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            previous,
            _thread: std::marker::PhantomData,
        })
    }
}
impl Drop for LaunchErrorMode {
    fn drop(&mut self) {
        // SAFETY: this !Send guard restores the successful initialization on its original thread.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(
                self.previous,
                std::ptr::null_mut(),
            )
        };
    }
}
