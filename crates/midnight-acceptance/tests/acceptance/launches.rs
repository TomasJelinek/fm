//! Stand-ins for what would start programs on the desktop: a Custom Command's terminal,
//! the command line's terminal, a file's association, a system location and a web or mail
//! link a viewer followed, a Custom Command's console window. Each launch
//! is recorded in [`Launches`] for the scenario to check, and none starts a process:
//! the Engine gets one that has already ended successfully. Only a Custom Command's
//! program runs, as configured; the scenarios configure harmless programs that write into
//! their own work folder.
use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{Arc, Mutex, PoisonError},
};

/// One launch the application asked for.
#[derive(Clone, Debug)]
#[allow(dead_code)] // The scenarios read the fields they check.
pub enum Launch {
    /// A Custom Command's program (`Config::user_commands`), started as configured.
    Program {
        executable: PathBuf,
        arguments: Vec<OsString>,
        directory: PathBuf,
    },
    /// A Custom Command's program in a console window (`Config::user_commands`): the
    /// launcher would open a console, so it is recorded and never started.
    Console {
        executable: PathBuf,
        arguments: Vec<OsString>,
        directory: PathBuf,
    },
    /// A terminal: a Custom Command in terminal mode (`Config::user_commands`, with its
    /// environment), or the command line and its terminal button (`Config::desktop`).
    /// Never started.
    Terminal {
        request: TerminalRequest,
        environment: Vec<(String, OsString)>,
    },
    /// A file opened with its association (`Config::desktop`). Never started.
    Open(PathBuf),
    /// A system location such as the Recycle Bin (`Config::desktop`). Never opened.
    SystemLocation(SystemLocation),
    /// A web or mail link a viewer followed (`OpenService::open_url`, `Config::desktop`),
    /// as the address the system's handler would get. No browser or mail program starts.
    OpenUrl(String),
}

/// Every launch the application asked for, in order.
#[derive(Default)]
pub struct Launches(Mutex<Vec<Launch>>);

impl Launches {
    pub fn all(&self) -> Vec<Launch> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, launch: Launch) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(launch);
    }
}

/// The process of a launch that was recorded instead: it has already ended
/// successfully, and its id (0) is no real process's.
struct NotStarted;

impl TerminalProcess for NotStarted {
    fn id(&self) -> u32 {
        0
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        Ok(Some(ExitStatus::default()))
    }
}

fn interrupted(directory: &Path) -> FsError {
    FsError::from_io(
        "Custom Command terminal",
        directory,
        std::io::ErrorKind::Interrupted.into(),
    )
}

/// Record a terminal instead of starting it.
fn terminal(
    launches: &Launches,
    request: &TerminalRequest,
    environment: &[(String, OsString)],
    cancel: Option<&Cancellation>,
) -> Result<Box<dyn TerminalProcess>, FsError> {
    if cancel.is_some_and(Cancellation::is_cancelled) {
        return Err(interrupted(&request.working_directory));
    }
    launches.push(Launch::Terminal {
        request: request.clone(),
        environment: environment.to_vec(),
    });
    Ok(Box::new(NotStarted))
}

/// The Custom Command launcher of `Config::user_commands`: the platform's own, whose
/// validation runs as in production and whose programs start, except that terminals
/// are recorded and never started.
pub struct UserCommands {
    native: Arc<LocalFileSystem>,
    launches: Arc<Launches>,
}

impl UserCommands {
    pub fn new(native: Arc<LocalFileSystem>, launches: Arc<Launches>) -> Self {
        Self { native, launches }
    }
}

impl TerminalService for UserCommands {
    fn shell_flavor(&self) -> ShellFlavor {
        self.native.shell_flavor()
    }

    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        terminal(&self.launches, request, &[], None)
    }

    fn launch_terminal_with_environment(
        &self,
        request: &TerminalRequest,
        environment: &[(String, OsString)],
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        terminal(&self.launches, request, environment, Some(cancel))
    }
}

impl UserCommandService for UserCommands {
    fn validate_user_command(
        &self,
        process: &UserCommandProcess,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.native.validate_user_command(process, cancel)
    }

    fn launch_user_program(
        &self,
        executable: &Path,
        arguments: &[OsString],
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.launches.push(Launch::Program {
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            directory: directory.to_path_buf(),
        });
        self.native
            .launch_user_program(executable, arguments, directory, cancel)
    }

    /// An output window's program runs as configured, its output captured.
    fn launch_user_program_captured(
        &self,
        executable: &Path,
        arguments: &[OsString],
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn CapturedUserProgram>, FsError> {
        self.launches.push(Launch::Program {
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            directory: directory.to_path_buf(),
        });
        self.native
            .launch_user_program_captured(executable, arguments, directory, cancel)
    }

    fn launch_user_program_console(
        &self,
        executable: &Path,
        arguments: &[OsString],
        directory: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        if cancel.is_cancelled() {
            return Err(interrupted(directory));
        }
        self.launches.push(Launch::Console {
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            directory: directory.to_path_buf(),
        });
        Ok(Box::new(NotStarted))
    }
}

/// The desktop service of `Config::desktop`: the platform's read-only parts (locations,
/// icons, property details, link targets, the shell's flavour), with every launch
/// recorded and never started. Properties never opens the system's dialog: the
/// application shows its own summary.
pub struct Desktop {
    native: Arc<LocalFileSystem>,
    launches: Arc<Launches>,
    native_properties: bool,
}

impl Desktop {
    pub fn new(native: Arc<LocalFileSystem>, launches: Arc<Launches>) -> Self {
        Self {
            native,
            launches,
            native_properties: false,
        }
    }

    /// Properties opens the platform's own dialog (`Config::native_properties`).
    pub fn with_native_properties(mut self, native_properties: bool) -> Self {
        self.native_properties = native_properties;
        self
    }
}

impl TerminalService for Desktop {
    fn shell_flavor(&self) -> ShellFlavor {
        self.native.shell_flavor()
    }

    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        terminal(&self.launches, request, &[], None)
    }

    fn launch_terminal_with_environment(
        &self,
        request: &TerminalRequest,
        environment: &[(String, OsString)],
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        terminal(&self.launches, request, environment, Some(cancel))
    }
}

impl OpenService for Desktop {
    /// A folder (or a link to one) is inspected by the platform, which starts nothing
    /// for it; a file's association is recorded instead of started.
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => self.native.open_entry(path),
            Ok(metadata) if metadata.is_file() => {
                self.launches.push(Launch::Open(path.to_path_buf()));
                Ok(OpenEntryResult::File { process: None })
            }
            Ok(_) => Err(FsError::from_io(
                "open",
                path,
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Only files and folders are opened.",
                ),
            )),
            Err(error) => Err(FsError::from_io("open", path, error)),
        }
    }

    fn read_link_target(&self, link: &Path) -> Result<Option<PathBuf>, FsError> {
        self.native.read_link_target(link)
    }

    fn open_system_location(
        &self,
        location: SystemLocation,
    ) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
        self.launches.push(Launch::SystemLocation(location));
        Ok(None)
    }

    /// The link is recorded and taken, as a handler that accepted it would answer; the
    /// trait's own default would refuse it, and the platform's would start a browser.
    fn open_url(&self, url: &ExternalUrl) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
        self.launches.push(Launch::OpenUrl(url.as_str().to_owned()));
        Ok(None)
    }

    fn directory_destination(&self, path: &Path) -> Result<PathBuf, FsError> {
        self.native.directory_destination(path)
    }
}

impl IconService for Desktop {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        self.native.load_icon(key)
    }

    fn icon_edge(&self, drawn: u16) -> u16 {
        self.native.icon_edge(drawn)
    }

    fn load_executable_icon(
        &self,
        path: &Path,
        size: u16,
        cancel: &Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        self.native.load_executable_icon(path, size, cancel)
    }

    fn load_file_icon(
        &self,
        path: &Path,
        edge: u16,
        cancel: &Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        self.native.load_file_icon(path, edge, cancel)
    }
}

impl PropertyService for Desktop {
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError> {
        if self.native_properties {
            return self.native.show_properties(paths);
        }
        Ok(false)
    }

    fn property_details(&self, path: &Path) -> Result<Option<PropertyDetails>, FsError> {
        self.native.property_details(path)
    }
}

impl LocationService for Desktop {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        self.native.locations()
    }

    fn supports_system_location(&self, location: SystemLocation) -> bool {
        self.native.supports_system_location(location)
    }
}
