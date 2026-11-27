//! Copies that Windows Installer manages (the release MSI): finding them, handing a
//! verified MSI of a newer release to `msiexec`, and reporting how it went.
//!
//! An MSI copy has no `BUILD-INFO.json`, so it is never swapped in place (`apply`):
//! Windows Installer owns its files, its registration and its uninstall entry, and only a
//! newer MSI of the same `UpgradeCode` may replace them. The flow
//! (docs/developer/UPDATES.md, "Windows Installer copies"):
//!
//! 1. `detect` asks Windows Installer for the products of the MSI's fixed `UpgradeCode`
//!    and finds the one whose `InstallLocation` is the running executable's folder, in
//!    the per-user or the per-machine context. That context is the installation's scope.
//! 2. The controller downloads `midnight-explorer-<version>-<target>.msi` and checks it
//!    against its line in the signed `SHA256SUMS`, as it checks an archive.
//! 3. "Install now" closes the window the ordinary way; the application copies its own
//!    executable to `<private folder>/installer/` and starts that copy with
//!    `--install-update` (`InstallRequest`), then exits.
//! 4. The copy (`install`) waits until no copy runs from the installation (each holds a
//!    shared `InstanceLock`), opens the MSI so that no program can change, rename or
//!    delete it, checks its SHA-256 against the signed digest again, starts `msiexec`
//!    (through UAC for a per-machine installation) and waits for it. Only then does it
//!    let go of the MSI, record the outcome for the next start and start the installed
//!    executable again, which says how the update went.
use crate::{
    archive::BUILD_INFO,
    install::{NotUpdatable, remove_tree, write_json},
    trust,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    ffi::OsString,
    fmt, fs,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
pub use crate::installer_windows::{SystemHost, SystemProducts};

/// The MSI's `UpgradeCode` (`scripts/package-msi.py`); it never changes.
pub const UPGRADE_CODE: &str = "{DDF2A2BA-6901-484C-97B2-4EF662BF5E72}";
/// The executable that the MSI installs.
pub const EXECUTABLE: &str = "midnight-explorer.exe";
/// The first argument of the installer hand-off process.
pub const INSTALL_ARGUMENT: &str = "--install-update";
/// The helper's folder below the private folder.
pub const HELPER_FOLDER: &str = "installer";
const RESULT_FILE: &str = "installer-result.json";
/// The per-user installation's `msiexec` log, in the private folder.
pub const LOG_FILE: &str = "installer.log";
/// How long the hand-off waits for every copy of the installation to exit.
pub const INSTANCES_WAIT: Duration = Duration::from_secs(60);

/// The installation context that Windows Installer recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// "Just for me": per user, without administrator rights.
    User,
    /// "For all users": per machine; installing needs administrator approval.
    Machine,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Machine => "machine",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(Self::User),
            "machine" => Some(Self::Machine),
            _ => None,
        }
    }
}

/// What Windows Installer records about one installed product in one context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductInfo {
    /// `InstallLocation` (the MSI sets it from `INSTALLDIR`).
    pub location: PathBuf,
    /// `VersionString`.
    pub version: String,
}

/// Windows Installer's records; the system implementation asks msi.dll, tests use a fake.
pub trait Products {
    /// Product codes of the installed products with `upgrade_code`, for this user and
    /// the machine.
    fn related(&self, upgrade_code: &str) -> Vec<String>;
    /// The product as installed in `scope`, `None` when it is not installed there.
    fn product(&self, product_code: &str, scope: Scope) -> Option<ProductInfo>;
}

/// A copy that Windows Installer installed and that a newer MSI updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledCopy {
    /// The installation folder: the running executable's folder.
    pub folder: PathBuf,
    pub product_code: String,
    pub scope: Scope,
    pub version: Version,
}

/// The installed MSI copy that `executable` runs from. It must be named
/// `midnight-explorer.exe`, have no `BUILD-INFO.json` beside it, and Windows Installer
/// must record exactly one installation of this product in its folder, of the running
/// version.
pub fn detect(
    executable: &Path,
    current: &Version,
    products: &dyn Products,
) -> Result<InstalledCopy, NotUpdatable> {
    let folder = executable.parent().ok_or(NotUpdatable::NotAReleaseFolder)?;
    let named = executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(EXECUTABLE));
    if !named || folder.join(BUILD_INFO).exists() {
        return Err(NotUpdatable::NotAReleaseFolder);
    }
    let mut found = Vec::new();
    for product in products.related(UPGRADE_CODE) {
        for scope in [Scope::User, Scope::Machine] {
            if let Some(info) = products.product(&product, scope)
                && same_folder(&info.location, folder)
            {
                found.push((product.clone(), scope, info.version));
            }
        }
    }
    let (product_code, scope, version) = match found.len() {
        0 => return Err(NotUpdatable::NotAReleaseFolder),
        1 => found.remove(0),
        _ => {
            return Err(NotUpdatable::Mismatch(
                "Windows Installer records more than one installation in its folder.".into(),
            ));
        }
    };
    let recorded = Version::parse(&version).ok();
    if recorded.as_ref() != Some(current) {
        return Err(NotUpdatable::Mismatch(format!(
            "Windows Installer records version {version} in its folder, not the running {current}."
        )));
    }
    Ok(InstalledCopy {
        folder: folder.to_path_buf(),
        product_code,
        scope,
        version: current.clone(),
    })
}

/// Whether both paths name one existing folder (case, a trailing separator and links
/// do not matter).
fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// A downloaded MSI whose SHA-256 matched its signed line: what "Install now" hands over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyInstaller {
    pub copy: InstalledCopy,
    /// The MSI in the private download folder.
    pub package: PathBuf,
    /// Its signed SHA-256.
    pub sha256: [u8; 32],
    pub version: String,
    pub page_url: String,
}

/// What the hand-off process is asked to do. Travels as command-line arguments, so
/// paths and arguments keep their native form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest {
    pub package: PathBuf,
    pub sha256: [u8; 32],
    pub scope: Scope,
    /// The installation folder (`INSTALLDIR`), where the executable starts again.
    pub folder: PathBuf,
    /// The version being installed.
    pub to: String,
    /// The user's private update folder: the instance locks, the outcome and the log.
    pub private_folder: PathBuf,
    pub working_folder: PathBuf,
    pub arguments: Vec<OsString>,
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl InstallRequest {
    /// `--install-update PACKAGE SHA256 SCOPE FOLDER VERSION PRIVATE WORKING -- ARGUMENTS...`
    pub fn to_arguments(&self) -> Vec<OsString> {
        let mut arguments = vec![
            OsString::from(INSTALL_ARGUMENT),
            self.package.clone().into_os_string(),
            OsString::from(hex(&self.sha256)),
            OsString::from(self.scope.as_str()),
            self.folder.clone().into_os_string(),
            OsString::from(&self.to),
            self.private_folder.clone().into_os_string(),
            self.working_folder.clone().into_os_string(),
            OsString::from("--"),
        ];
        arguments.extend(self.arguments.iter().cloned());
        arguments
    }

    /// Parses the arguments after the program name; `None` unless the first one is
    /// `--install-update`.
    pub fn from_arguments(arguments: &[OsString]) -> Option<Result<Self, String>> {
        if arguments
            .first()
            .is_none_or(|first| first != INSTALL_ARGUMENT)
        {
            return None;
        }
        let parsed = (|| {
            let text = |index: usize| arguments.get(index).and_then(|value| value.to_str());
            let request = Self {
                package: PathBuf::from(arguments.get(1)?),
                sha256: trust::decode_hex(text(2)?)?,
                scope: Scope::parse(text(3)?)?,
                folder: PathBuf::from(arguments.get(4)?),
                to: Version::parse(text(5)?).ok()?.to_string(),
                private_folder: PathBuf::from(arguments.get(6)?),
                working_folder: PathBuf::from(arguments.get(7)?),
                arguments: Vec::new(),
            };
            (arguments.get(8)? == "--").then_some(())?;
            let absolute = [&request.package, &request.folder, &request.private_folder]
                .iter()
                .all(|path| path.is_absolute());
            absolute.then_some(Self {
                arguments: arguments[9..].to_vec(),
                ..request
            })
        })();
        Some(parsed.ok_or_else(|| "Invalid installer arguments.".to_owned()))
    }
}

/// The `msiexec` start: its parameters and whether it runs elevated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsiCommand {
    /// Through UAC (`runas`): a per-machine installation.
    pub elevated: bool,
    /// The parameters exactly as `msiexec` reads them.
    pub parameters: String,
}

/// A path for `msiexec`'s command line: quoted, so it must be Unicode without quotes.
fn quoted(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .filter(|text| !text.contains('"') && !text.is_empty())
        .ok_or_else(|| format!("{} cannot be passed to Windows Installer.", path.display()))?;
    Ok(format!("\"{text}\""))
}

/// The `msiexec` command for `request`. `/passive` shows Windows Installer's progress
/// window and asks nothing; `/norestart` never restarts Windows (a needed restart is
/// reported as 3010). The scope matches the installed copy's, so the MSI's major upgrade
/// finds and replaces it, and `INSTALLDIR` keeps its folder. Per user, `msiexec` writes a
/// verbose log to the private folder; per machine it writes none, because an elevated
/// process must not write to a path that the user's programs can redirect.
pub fn msiexec_command(request: &InstallRequest) -> Result<MsiCommand, String> {
    let package = quoted(&request.package)?;
    let folder = request
        .folder
        .to_str()
        .map(|text| text.trim_end_matches(['\\', '/']))
        .map(Path::new)
        .ok_or_else(|| "The installation folder cannot be passed to Windows Installer.".to_owned())
        .and_then(quoted)?;
    let parameters = match request.scope {
        Scope::User => format!(
            "/i {package} /passive /norestart /l*v {} ALLUSERS=2 MSIINSTALLPERUSER=1 INSTALLDIR={folder}",
            quoted(&request.private_folder.join(LOG_FILE))?
        ),
        Scope::Machine => {
            format!("/i {package} /passive /norestart ALLUSERS=1 INSTALLDIR={folder}")
        }
    };
    Ok(MsiCommand {
        elevated: request.scope == Scope::Machine,
        parameters,
    })
}

/// How the installation went, kept in `installer-result.json` for the next start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum InstallOutcome {
    Installed {
        to: String,
    },
    /// Installed, but Windows must restart to replace files that were in use (3010).
    RestartRequired {
        to: String,
    },
    /// The user declined UAC or cancelled Windows Installer (1223, 1602).
    Cancelled {
        to: String,
    },
    /// Another installation was in progress (1618).
    Busy {
        to: String,
    },
    /// Windows Installer failed (and rolled back), or the hand-off refused to start it.
    Failed {
        to: String,
        reason: String,
    },
}

/// `ERROR_CANCELLED`: the user declined the UAC prompt.
pub const ERROR_CANCELLED: u32 = 1223;

/// The outcome of `msiexec`'s exit status `code`. `log` is the verbose log, if any.
pub fn outcome_of(code: u32, to: &str, log: Option<&Path>) -> InstallOutcome {
    let to = to.to_owned();
    let details = log
        .map(|log| format!(" Its log is {}.", log.display()))
        .unwrap_or_default();
    let failed = |reason: String| InstallOutcome::Failed {
        to: to.clone(),
        reason,
    };
    match code {
        0 => InstallOutcome::Installed { to },
        1641 | 3010 => InstallOutcome::RestartRequired { to },
        1602 | ERROR_CANCELLED => InstallOutcome::Cancelled { to },
        1618 => InstallOutcome::Busy { to },
        1603 => failed(format!(
            "Windows Installer stopped with a fatal error (1603).{details}"
        )),
        1619 | 1620 => failed(format!(
            "Windows Installer could not open the downloaded package ({code}).{details}"
        )),
        1625 => failed("A Windows policy does not allow this installation (1625).".into()),
        1633 => {
            failed("Windows Installer does not support this package on this system (1633).".into())
        }
        1638 => failed(format!(
            "Windows Installer reports that another version of this product is already installed (1638).{details}"
        )),
        code => failed(format!(
            "Windows Installer stopped with error {code}.{details}"
        )),
    }
}

impl InstallOutcome {
    /// What the next start says about it.
    pub fn notice(&self) -> String {
        match self {
            Self::Installed { to } => format!("Updated to version {to}."),
            Self::RestartRequired { to } => {
                format!("Version {to} is installed. Restart Windows to finish the update.")
            }
            Self::Cancelled { to } => {
                format!("The update to version {to} was cancelled. Nothing changed.")
            }
            Self::Busy { to } => format!(
                "Version {to} was not installed because another installation was in progress. Nothing changed; try again when it has finished."
            ),
            Self::Failed { to, reason } => {
                format!("Version {to} was not installed. {reason} Nothing changed.")
            }
        }
    }
}

impl fmt::Display for InstallOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.notice())
    }
}

/// The steps of the hand-off that touch the system; replaced in tests.
pub trait Host {
    /// Waits until no copy runs from the installation; false when one still does.
    fn wait_for_instances(&self, request: &InstallRequest) -> bool;
    /// Opens `package` so that no program can change, rename or delete it while the
    /// returned guard lives, and checks its SHA-256 through that handle.
    fn pin(&self, package: &Path, sha256: &[u8; 32]) -> Result<Box<dyn Any>, String>;
    /// Starts `msiexec` with `command` and waits for its exit status.
    fn run(&self, command: &MsiCommand) -> io::Result<u32>;
    /// Starts the installed application again.
    fn relaunch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        working_folder: &Path,
    ) -> io::Result<()>;
}

#[derive(Serialize, Deserialize)]
struct ResultFile {
    folder: PathBuf,
    #[serde(flatten)]
    outcome: InstallOutcome,
}

/// Runs the hand-off for `request`: wait, pin and check, `msiexec`, record, relaunch.
/// Every path ends with the installed application running again: the new version, or
/// the previous one when Windows Installer did not install (it rolls back on failure).
pub fn install(request: &InstallRequest, host: &dyn Host) -> InstallOutcome {
    let outcome = install_steps(request, host);
    let record = ResultFile {
        folder: request.folder.clone(),
        outcome: outcome.clone(),
    };
    if let Err(error) = write_json(&request.private_folder.join(RESULT_FILE), &record) {
        eprintln!("Midnight Explorer: the update's outcome cannot be recorded: {error}");
    }
    if matches!(
        outcome,
        InstallOutcome::Installed { .. } | InstallOutcome::RestartRequired { .. }
    ) {
        // The package is used up; Windows Installer keeps its own copy.
        let _ = fs::remove_file(&request.package);
        if let Some(folder) = request.package.parent() {
            let _ = fs::remove_dir(folder);
        }
    }
    let executable = request.folder.join(EXECUTABLE);
    if let Err(error) = host.relaunch(&executable, &request.arguments, &request.working_folder) {
        eprintln!(
            "Midnight Explorer: {} cannot be started: {error}",
            executable.display()
        );
    }
    outcome
}

fn install_steps(request: &InstallRequest, host: &dyn Host) -> InstallOutcome {
    let failed = |reason: String| InstallOutcome::Failed {
        to: request.to.clone(),
        reason,
    };
    let command = match msiexec_command(request) {
        Ok(command) => command,
        Err(reason) => return failed(reason),
    };
    if !host.wait_for_instances(request) {
        return failed(
            "Another Midnight Explorer window of this installation is still open. Close it and install the update again."
                .into(),
        );
    }
    // Held until msiexec has exited: what it reads is the content checked here.
    let guard = match host.pin(&request.package, &request.sha256) {
        Ok(guard) => guard,
        Err(reason) => return failed(reason),
    };
    let status = host.run(&command);
    drop(guard);
    let log = (!command.elevated).then(|| request.private_folder.join(LOG_FILE));
    match status {
        Ok(code) => outcome_of(code, &request.to, log.as_deref()),
        Err(error) if error.raw_os_error() == Some(ERROR_CANCELLED as i32) => {
            InstallOutcome::Cancelled {
                to: request.to.clone(),
            }
        }
        Err(error) => failed(format!("Windows Installer could not be started: {error}.")),
    }
}

/// Checks the SHA-256 of the pinned `package` (read from its start) against `sha256`.
pub(crate) fn check_digest(package: &File, sha256: &[u8; 32]) -> Result<(), String> {
    let mut reader = package;
    let actual = trust::sha256_reader(&mut reader)
        .map_err(|error| format!("The downloaded installer cannot be read: {error}"))?;
    if &actual != sha256 {
        return Err(
            "The downloaded installer does not match its signed checksum: it was changed after it was checked."
                .into(),
        );
    }
    Ok(())
}

/// The outcome that a hand-off recorded for the installation in `folder`, read once at
/// start. Also removes the hand-off's executable copy, which is no longer needed.
pub fn startup(private_folder: &Path, folder: &Path) -> Option<InstallOutcome> {
    let _ = remove_tree(&private_folder.join(HELPER_FOLDER));
    let path = private_folder.join(RESULT_FILE);
    let text = fs::read(&path).ok()?;
    let record: ResultFile = serde_json::from_slice(&text).ok()?;
    // A record of another installation (another copy of this user's) waits for it.
    if record.folder != folder && !same_folder(&record.folder, folder) {
        return None;
    }
    let _ = fs::remove_file(&path);
    Some(record.outcome)
}

/// The instance lock file of the installation in `folder`, in the private folder (an
/// installation for all users cannot hold it in its own folder).
pub fn instance_lock_path(private_folder: &Path, folder: &Path) -> PathBuf {
    let key = folder.to_string_lossy().to_lowercase();
    let digest = Sha256::digest(key.trim_end_matches(['\\', '/']).as_bytes());
    let name: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    private_folder
        .join("instances")
        .join(format!("{name}.lock"))
}

fn open_lock(private_folder: &Path, folder: &Path) -> io::Result<File> {
    let path = instance_lock_path(private_folder, folder);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// The shared lock that every running copy of this user's holds on its MSI installation;
/// the hand-off waits for an exclusive one.
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Takes the shared lock for the installation in `folder`; `None` while a hand-off
    /// holds it or when the private folder is not usable.
    pub fn acquire(private_folder: &Path, folder: &Path) -> Option<Self> {
        let file = open_lock(private_folder, folder).ok()?;
        file.try_lock_shared().ok()?;
        Some(Self { _file: file })
    }
}

/// Waits up to `limit` for the exclusive instance lock of the installation in `folder`;
/// the lock is held as long as the returned file is open.
pub fn wait_for_instances(private_folder: &Path, folder: &Path, limit: Duration) -> Option<File> {
    let file = open_lock(private_folder, folder).ok()?;
    let deadline = Instant::now() + limit;
    loop {
        if file.try_lock().is_ok() {
            return Some(file);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Prepares "Install now": copies the running executable to `<private
/// folder>/installer/` and returns that copy and its request. The installation's own
/// files are replaced by Windows Installer, so the hand-off must not run from them.
pub fn prepare(
    ready: &ReadyInstaller,
    current_executable: &Path,
    private_folder: &Path,
    working_folder: PathBuf,
    arguments: Vec<OsString>,
) -> io::Result<(PathBuf, InstallRequest)> {
    if !ready.package.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the downloaded installer is missing",
        ));
    }
    let helper_folder = private_folder.join(HELPER_FOLDER);
    remove_tree(&helper_folder)?;
    fs::create_dir_all(&helper_folder)?;
    let helper = helper_folder.join(
        current_executable
            .file_name()
            .ok_or_else(|| io::Error::other("the executable has no name"))?,
    );
    fs::copy(current_executable, &helper)?;
    Ok((
        helper,
        InstallRequest {
            package: ready.package.clone(),
            sha256: ready.sha256,
            scope: ready.copy.scope,
            folder: ready.copy.folder.clone(),
            to: ready.version.clone(),
            private_folder: private_folder.to_path_buf(),
            working_folder,
            arguments,
        },
    ))
}

/// Starts the hand-off prepared by `prepare`; the caller exits next.
pub fn start(helper: &Path, request: &InstallRequest) -> io::Result<()> {
    std::process::Command::new(helper)
        .args(request.to_arguments())
        .current_dir(&request.private_folder)
        .spawn()
        .map(drop)
}

/// The hand-off's entry point: installs what the arguments describe and returns the
/// process exit status.
pub fn run(arguments: &[OsString]) -> u8 {
    match InstallRequest::from_arguments(arguments) {
        #[cfg(windows)]
        Some(Ok(request)) => match install(&request, &crate::installer_windows::SystemHost) {
            InstallOutcome::Installed { .. } | InstallOutcome::RestartRequired { .. } => 0,
            _ => 1,
        },
        _ => 2,
    }
}

/// The installed copy that `executable` runs from, as Windows Installer records it.
#[cfg(windows)]
pub fn detect_system(executable: &Path, current: &Version) -> Result<InstalledCopy, NotUpdatable> {
    detect(
        executable,
        current,
        &crate::installer_windows::SystemProducts,
    )
}
