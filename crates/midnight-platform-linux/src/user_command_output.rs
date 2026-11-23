//! Run directly with its output shown: captured in pipes for the app's output window, or
//! in the configured terminal emulator through the app's own launcher. Both pass the
//! literal argv; neither builds shell source.
use midnight_core::{
    CapturedStreams, CapturedUserProgram, ConsoleLauncher, ProgramExit, describe_duration,
    display_command_line,
};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::Instant,
};

static LAUNCHER: OnceLock<ConsoleLauncher> = OnceLock::new();

/// Set once, at startup, by the application (its own executable with
/// `RUN_AND_WAIT_ARGUMENT`). False when a launcher was already set.
pub fn set_console_launcher(launcher: ConsoleLauncher) -> bool {
    LAUNCHER.set(launcher).is_ok()
}

struct Captured {
    child: Child,
    streams: Option<CapturedStreams>,
    exit: Option<ProgramExit>,
}

fn exit(status: std::process::ExitStatus) -> ProgramExit {
    match (status.code(), status.signal()) {
        (Some(code), _) => ProgramExit::Code(i64::from(code)),
        (None, Some(signal)) => ProgramExit::Signal(signal),
        (None, None) => ProgramExit::Code(-1),
    }
}

impl CapturedUserProgram for Captured {
    fn id(&self) -> u32 {
        self.child.id()
    }
    fn take_streams(&mut self) -> Option<CapturedStreams> {
        self.streams.take()
    }
    fn try_wait(&mut self) -> io::Result<Option<ProgramExit>> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait()?.map(exit);
        }
        Ok(self.exit)
    }
    fn terminate_tree(&mut self, force: bool) -> io::Result<()> {
        if self.exit.is_some() {
            return Ok(());
        }
        let group = libc::pid_t::try_from(self.child.id())
            .map_err(|_| io::Error::other("process id out of range"))?;
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: signals this program's own process group, which it leads: the child
        // is not yet reaped, so its id and group id cannot have been reused.
        if unsafe { libc::kill(-group, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
}

/// Start the prepared Run directly `command` with no terminal input (`/dev/null`),
/// standard output and error in pipes and the program leading a new process group, so
/// Stop ends the processes it started too.
pub(crate) fn spawn_captured(mut command: Command) -> io::Result<Box<dyn CapturedUserProgram>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("The program's output pipes are missing."));
    };
    Ok(Box::new(Captured {
        child,
        streams: Some(CapturedStreams {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
        }),
        exit: None,
    }))
}

/// The configured terminal emulator running the launcher, which runs `executable` with
/// exactly `arguments` in `directory` and keeps the terminal open after it exits.
pub(crate) fn console_command(
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
) -> io::Result<Command> {
    let launcher = LAUNCHER.get().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Console window output is unavailable: the app's console launcher is not set.",
        )
    })?;
    console_command_with(
        &crate::desktop::terminal_executable()?,
        launcher,
        executable,
        arguments,
        directory,
    )
}

pub(crate) fn console_command_with(
    terminal: &Path,
    launcher: &ConsoleLauncher,
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
) -> io::Result<Command> {
    let mut command = crate::desktop::terminal_host_command(terminal);
    // Positional native arguments after the terminal's own execute option: the terminal
    // starts the launcher with them as its argv, and the launcher starts the program.
    command
        .arg(&launcher.executable)
        .args(launcher.arguments(executable, arguments))
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::user_commands::validate_os_arguments(&command)?;
    Ok(command)
}

/// The Console window launcher (`midnight-explorer --run-and-wait <program> [argument…]`): runs the
/// program in this terminal, shows how it ended and waits for a key. Returns the
/// program's exit code (1 when it could not start or was signalled).
pub fn run_and_wait(arguments: Vec<OsString>) -> i32 {
    let code = run_program(arguments, None, &mut io::stdout(), true);
    let _ = write!(io::stdout(), "Press any key to close this window.");
    let _ = io::stdout().flush();
    wait_for_key();
    code
}

/// Run `arguments[0]` with the rest as its literal arguments in this terminal (in
/// `directory`, else this process's working folder, which the terminal set to the
/// command's), and write what ran and how it ended to `output`.
pub(crate) fn run_program(
    arguments: Vec<OsString>,
    directory: Option<&Path>,
    output: &mut dyn Write,
    interactive: bool,
) -> i32 {
    let mut arguments = arguments.into_iter();
    let Some(program) = arguments.next().map(PathBuf::from) else {
        let _ = writeln!(output, "Nothing to run.");
        return 2;
    };
    let rest: Vec<OsString> = arguments.collect();
    let line = display_command_line(&program, &rest);
    if interactive {
        // The terminal's title (OSC 0), as the Windows console shows its program.
        let _ = write!(output, "\x1b]0;{}\x07", line.replace(['\x07', '\x1b'], " "));
    }
    let _ = writeln!(output, "> {line}\n");
    let _ = output.flush();
    let started = Instant::now();
    let mut command = Command::new(&program);
    command.args(&rest);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    crate::open_files::with_original_open_file_limit(&mut command);
    let child = command.spawn();
    if interactive {
        // Ctrl+C reaches the program (the terminal's foreground group), and this
        // launcher stays to report how it ended. Set after the spawn: an ignored signal
        // would be inherited.
        ignore_interrupts();
    }
    let status = child.and_then(|mut child| child.wait());
    let elapsed = describe_duration(started.elapsed());
    match status {
        Ok(status) => {
            let exit = exit(status);
            let _ = writeln!(output, "\n{} · {elapsed}", exit.describe());
            match exit {
                ProgramExit::Code(code) => i32::try_from(code).unwrap_or(1),
                ProgramExit::Signal(_) => 1,
            }
        }
        Err(error) => {
            let _ = writeln!(output, "\nThe program could not start: {error}");
            1
        }
    }
}

fn ignore_interrupts() {
    // SAFETY: SIG_IGN for SIGINT and SIGQUIT in this single-purpose launcher process.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
}

/// Wait for one key press: the terminal is switched out of line mode for the read and
/// back afterwards. Without a terminal (input redirected), end of input also ends it.
fn wait_for_key() {
    // SAFETY: termios is a plain C structure for which all-zero bytes are valid.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: a terminal attribute query on standard input into owned storage.
    let raw = unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } == 0;
    if raw {
        let mut changed = saved;
        changed.c_lflag &= !(libc::ICANON | libc::ECHO);
        changed.c_cc[libc::VMIN] = 1;
        changed.c_cc[libc::VTIME] = 0;
        // SAFETY: as above; keys typed while the program ran are discarded.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &changed) };
    }
    let mut byte = [0u8; 1];
    let _ = io::stdin().read(&mut byte);
    if raw {
        // SAFETY: restores the attributes saved above.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) };
    }
}

#[cfg(test)]
#[path = "user_command_output_tests.rs"]
mod tests;
