//! Run directly with its output shown: captured in pipes for the app's output window, or
//! in a new console through the app's own launcher. Both pass the literal argv; neither
//! builds shell source.
use crate::terminal_process::{LaunchSpec, terminated};
use midnight_core::{
    CapturedStreams, CapturedUserProgram, ConsoleLauncher, ProgramExit, TerminalProcess,
    describe_duration, display_command_line,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::OnceLock,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Globalization::{CP_OEMCP, MultiByteToWideChar},
    System::{
        JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject},
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

/// Pipe buffer per stream: a fast writer fills it only while the reader is busy.
const PIPE_BUFFER: u32 = 64 * 1024;

static LAUNCHER: OnceLock<ConsoleLauncher> = OnceLock::new();

/// Set once, at startup, by the application (its own executable with
/// `RUN_AND_WAIT_ARGUMENT`). False when a launcher was already set.
pub fn set_console_launcher(launcher: ConsoleLauncher) -> bool {
    LAUNCHER.set(launcher).is_ok()
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// An anonymous pipe, both ends not inheritable: (read, write).
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (null_mut(), null_mut());
    // SAFETY: two writable handle slots; default security (not inheritable).
    check(unsafe { CreatePipe(&mut read, &mut write, null(), PIPE_BUFFER) })?;
    // SAFETY: CreatePipe succeeded and transferred both handles to this scope.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

/// One `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` attribute list.
struct Attributes(Vec<usize>);
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: the first call only reports the size; the second gets aligned storage.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            check(InitializeProcThreadAttributeList(
                storage.as_mut_ptr().cast(),
                1,
                0,
                &mut bytes,
            ))?;
            Ok(Self(storage))
        }
    }
    fn handles<const N: usize>(&mut self, handles: &[*mut core::ffi::c_void; N]) -> io::Result<()> {
        // SAFETY: an initialized list; the handle array outlives CreateProcessW.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.0.as_mut_ptr().cast(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of::<[*mut core::ffi::c_void; N]>(),
                null_mut(),
                null(),
            )
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: this owns the initialized list; process creation has finished.
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

/// A pipe's read end: the writer closing it (`ERROR_BROKEN_PIPE`) is the end of the stream.
struct PipeReader(File);
impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buffer) {
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            result => result,
        }
    }
}

struct Captured {
    process: OwnedHandle,
    /// The job holding the program and every process it starts; None where the system
    /// refused one (Stop then ends the program only).
    job: Option<OwnedHandle>,
    id: u32,
    streams: Option<CapturedStreams>,
    exit: Option<ProgramExit>,
}

impl CapturedUserProgram for Captured {
    fn id(&self) -> u32 {
        self.id
    }
    fn take_streams(&mut self) -> Option<CapturedStreams> {
        self.streams.take()
    }
    fn try_wait(&mut self) -> io::Result<Option<ProgramExit>> {
        if self.exit.is_some() {
            return Ok(self.exit);
        }
        // SAFETY: a live owned process handle, queried without waiting.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => return Ok(None),
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => return Err(io::Error::other("Unexpected process wait result")),
        }
        let mut code = 0u32;
        // SAFETY: the process is signaled; any code, even 259, is its exit code.
        check(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
        self.exit = Some(ProgramExit::Code(i64::from(code)));
        Ok(self.exit)
    }
    fn terminate_tree(&mut self, _force: bool) -> io::Result<()> {
        // SAFETY: owned live job/process handles; only this program's tree is ended.
        match &self.job {
            Some(job) => check(unsafe { TerminateJobObject(job.as_raw_handle(), 1) }),
            None => check(unsafe { TerminateProcess(self.process.as_raw_handle(), 1) }),
        }
    }
    fn decoder(&self) -> midnight_core::OutputDecoder {
        decode_console
    }
}

/// UTF-8 when the line is valid UTF-8 (git, most cross-platform tools), else the OEM code
/// page that the program's hidden console uses by default (cmd, older console tools).
pub(crate) fn decode_console(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let Ok(length) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    // SAFETY: a readable byte slice of `length` bytes; null output only asks the size.
    let units = unsafe { MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), length, null_mut(), 0) };
    if units <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; units as usize];
    // SAFETY: as above, with writable storage of exactly `units` UTF-16 units.
    let written = unsafe {
        MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            length,
            wide.as_mut_ptr(),
            units,
        )
    };
    if written <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    String::from_utf16_lossy(&wide[..written as usize])
}

/// Start `executable` with exactly `arguments` in `directory`: no window, standard
/// input the NUL device, standard output and error in pipes, the program and its
/// descendants in a job object. The process starts suspended and runs only once it
/// belongs to the job, so none of its children can escape Stop.
pub(crate) fn spawn_captured(
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
) -> io::Result<Box<dyn CapturedUserProgram>> {
    // CreateProcessW cannot load a batch file. Start our helper inside the capture
    // job, so Stop reaches the script and its children and the pipes stay connected.
    let batch = is_batch(executable);
    let forwarded;
    let (executable, arguments) = if batch {
        let helper = LAUNCHER.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "The captured batch launcher is not configured.",
            )
        })?;
        forwarded = [
            OsString::from(RUN_BATCH_ARGUMENT),
            executable.as_os_str().to_owned(),
        ]
        .into_iter()
        .chain(arguments.iter().cloned())
        .collect::<Vec<_>>();
        (helper.executable.as_path(), forwarded.as_slice())
    } else {
        (executable, arguments)
    };
    let spec = LaunchSpec {
        application: executable.to_owned(),
        arguments: arguments.to_owned(),
        raw_tail: None,
        cmd_environment: false,
        working_directory: directory.to_owned(),
    };
    crate::terminal_process::validate(&spec)?;
    let application = terminated(executable.as_os_str())?;
    let directory = terminated(directory.as_os_str())?;
    let mut line = spec.command_line()?;
    let (stdout, child_stdout) = pipe()?;
    let (stderr, child_stderr) = pipe()?;
    let child_stdin = OwnedHandle::from(File::open("NUL")?);
    let inherited = [
        child_stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let mut attributes = Attributes::new()?;
    attributes.handles(&inherited)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited[0];
    startup.StartupInfo.hStdOutput = inherited[1];
    startup.StartupInfo.hStdError = inherited[2];
    startup.lpAttributeList = attributes.0.as_mut_ptr().cast();
    // SAFETY: a new unnamed job; ownership only after success.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if batch && job.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateJobObjectW returned a new owned handle.
    let job = (!job.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(job) });
    let mut information = PROCESS_INFORMATION::default();
    {
        let _error_mode = crate::user_commands::LaunchErrorMode::new()?;
        // Other launchers that let CreateProcessW inherit every inheritable handle wait
        // while these three are inheritable; they are closed before the lock is released.
        let _creation = midnight_worker_platform::lock_process_creation();
        let child_ends = [child_stdin, child_stdout, child_stderr];
        for handle in &child_ends {
            // SAFETY: an owned live handle; only its inheritance flag changes, and it is
            // closed below while the creation lock is still held.
            check(unsafe {
                SetHandleInformation(
                    handle.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT,
                )
            })?;
        }
        // SAFETY: terminated application, mutable command line and cwd; initialized
        // extended startup information restricted to exactly the three child handles;
        // the inherited (null) environment. Ownership of the results follows below.
        check(unsafe {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT,
                null(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut information,
            )
        })?;
        drop(child_ends);
    }
    // SAFETY: CreateProcessW succeeded and transferred both handles to this scope.
    let process = unsafe { OwnedHandle::from_raw_handle(information.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(information.hThread) };
    let job = match job {
        // SAFETY: both handles are owned and the helper has not resumed.
        Some(job)
            if unsafe {
                AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle())
            } != 0 =>
        {
            Some(job)
        }
        Some(_) if batch => {
            let error = io::Error::last_os_error();
            // SAFETY: Stop must reach the interpreter. End the suspended helper before it can start it.
            unsafe { TerminateProcess(process.as_raw_handle(), 1) };
            return Err(error);
        }
        _ => None,
    };
    // SAFETY: the owned primary thread of the suspended process.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        let error = io::Error::last_os_error();
        // SAFETY: end the never-resumed process this call created.
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
        return Err(error);
    }
    Ok(Box::new(Captured {
        process,
        job,
        id: information.dwProcessId,
        streams: Some(CapturedStreams {
            stdout: Box::new(PipeReader(File::from(stdout))),
            stderr: Box::new(PipeReader(File::from(stderr))),
        }),
        exit: None,
    }))
}

/// Start the launcher in a new console window, which runs `executable` with exactly
/// `arguments` in `directory` and keeps the console open after it exits.
pub(crate) fn spawn_console(
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
) -> io::Result<Box<dyn TerminalProcess>> {
    let launcher = LAUNCHER.get().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Console window output is unavailable: the app's console launcher is not set.",
        )
    })?;
    spawn_console_with(launcher, executable, arguments, directory, false)
}

pub(crate) fn spawn_console_with(
    launcher: &ConsoleLauncher,
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
    hidden: bool,
) -> io::Result<Box<dyn TerminalProcess>> {
    crate::terminal_process::spawn_inner(
        &LaunchSpec {
            application: launcher.executable.clone(),
            arguments: launcher.arguments(executable, arguments),
            raw_tail: None,
            cmd_environment: false,
            working_directory: directory.to_owned(),
        },
        hidden,
    )
}

pub const RUN_BATCH_ARGUMENT: &str = "--run-captured-batch";

fn is_batch(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
}

/// Use Rust's batch argument escaping and inherit the capture pipes.
pub fn run_captured_batch(mut arguments: impl Iterator<Item = OsString>) -> i32 {
    use std::os::windows::process::CommandExt;
    let Some(path) = arguments.next().map(PathBuf::from) else {
        return 2;
    };
    if !path.is_absolute() || !is_batch(&path) {
        return 2;
    }
    match std::process::Command::new(path)
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("The batch file could not start: {error}");
            1
        }
    }
}

/// The Console window launcher (`midnight-explorer --run-and-wait <program> [argument…]`): runs the
/// program in this process's console, shows how it ended and waits for a key. Returns
/// the program's exit code (1 when it could not start).
pub fn run_and_wait(arguments: Vec<OsString>) -> i32 {
    use windows_sys::Win32::System::Console::{AllocConsole, GetConsoleWindow};
    // The application is a GUI program: a console exists only once it asks for one.
    // SAFETY: plain console queries and allocation for this process.
    if unsafe { GetConsoleWindow() }.is_null() {
        unsafe { AllocConsole() };
    }
    ignore_console_interrupts();
    let code = run_program(arguments, &mut io::stdout(), true);
    let _ = write!(io::stdout(), "Press any key to close this window.");
    let _ = io::stdout().flush();
    wait_for_key();
    code
}

/// Ctrl+C and Ctrl+Break end the program, not the launcher that reports on it. A
/// handler (unlike ignoring them) is not inherited by the program.
fn ignore_console_interrupts() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe extern "system" fn handled(_event: u32) -> i32 {
        1
    }
    // SAFETY: a valid handler for the whole process lifetime.
    unsafe { SetConsoleCtrlHandler(Some(handled), 1) };
}

/// Run `arguments[0]` with the rest as its literal arguments, its standard streams the
/// console's, and write what ran and how it ended to `output`.
pub(crate) fn run_program(arguments: Vec<OsString>, output: &mut dyn Write, title: bool) -> i32 {
    let mut arguments = arguments.into_iter();
    let Some(program) = arguments.next().map(PathBuf::from) else {
        let _ = writeln!(output, "Nothing to run.");
        return 2;
    };
    let rest: Vec<OsString> = arguments.collect();
    let line = display_command_line(&program, &rest);
    if title {
        set_title(&line);
    }
    let _ = writeln!(output, "> {line}\n");
    let _ = output.flush();
    let started = Instant::now();
    let status = std::process::Command::new(&program).args(&rest).status();
    let elapsed = describe_duration(started.elapsed());
    match status {
        Ok(status) => {
            let code = status.code().unwrap_or(1);
            let exit = ProgramExit::Code(i64::from(code as u32));
            let _ = writeln!(output, "\n{} · {elapsed}", exit.describe());
            code
        }
        Err(error) => {
            let _ = writeln!(output, "\nThe program could not start: {error}");
            1
        }
    }
}

fn set_title(line: &str) {
    use windows_sys::Win32::System::Console::SetConsoleTitleW;
    let title: Vec<u16> = line.encode_utf16().take(1024).chain([0]).collect();
    // SAFETY: a terminated UTF-16 string alive through the call.
    unsafe { SetConsoleTitleW(title.as_ptr()) };
}

/// Wait for a key press in this console; keys typed while the program ran do not count,
/// and neither do Shift, Ctrl and Alt alone.
fn wait_for_key() {
    use windows_sys::Win32::System::Console::{
        FlushConsoleInputBuffer, GetStdHandle, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
        STD_INPUT_HANDLE,
    };
    // SAFETY: this process's console input handle; reads into one owned record.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        FlushConsoleInputBuffer(input);
        loop {
            let mut record: INPUT_RECORD = std::mem::zeroed();
            let mut read = 0;
            if ReadConsoleInputW(input, &mut record, 1, &mut read) == 0 || read == 0 {
                return;
            }
            if u32::from(record.EventType) == KEY_EVENT
                && record.Event.KeyEvent.bKeyDown != 0
                && !matches!(record.Event.KeyEvent.wVirtualKeyCode, 0x10..=0x12)
            {
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "user_command_output_tests.rs"]
mod tests;
