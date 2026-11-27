//! Copies that Windows Installer manages: detection through a fake product registry,
//! the MSI's asset and signed line, the `msiexec` command per scope, exit statuses, and
//! the hand-off's order (wait → pin and check → msiexec → release → record → relaunch)
//! with a fake host. Nothing here starts `msiexec` or asks for elevation.
use crate::support::{Answer, FakeTransport, TestKey, sha256_hex};
use midnight_update::{
    apply::StartupReport,
    client::{Endpoints, UpdateClient},
    controller::{Environment, Phase, Snapshot, UpdateController},
    install::NotUpdatable,
    installer::{
        self, EXECUTABLE, Host, InstallOutcome, InstallRequest, InstalledCopy, MsiCommand,
        ProductInfo, Products, ReadyInstaller, Scope, UPGRADE_CODE,
    },
    release::{Platform, current_version, parse_releases},
};
use semver::Version;
use std::{
    any::Any,
    cell::RefCell,
    collections::HashMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const API: &str = "https://api.github.com/repos/TomasJelinek/fm/releases?per_page=30";
const BASE: &str = "https://github.com/TomasJelinek/fm/releases";
const PRODUCT: &str = "{11111111-2222-3333-4444-555555555555}";

/// Windows Installer's records, as a table.
#[derive(Default)]
struct FakeProducts {
    related: HashMap<String, Vec<String>>,
    installed: HashMap<(String, Scope), ProductInfo>,
}

impl FakeProducts {
    fn with(mut self, product: &str, scope: Scope, location: &Path, version: &str) -> Self {
        let related = self.related.entry(UPGRADE_CODE.to_owned()).or_default();
        if !related.iter().any(|code| code == product) {
            related.push(product.to_owned());
        }
        self.installed.insert(
            (product.to_owned(), scope),
            ProductInfo {
                location: location.to_owned(),
                version: version.to_owned(),
            },
        );
        self
    }
}

impl Products for FakeProducts {
    fn related(&self, upgrade_code: &str) -> Vec<String> {
        self.related.get(upgrade_code).cloned().unwrap_or_default()
    }

    fn product(&self, product_code: &str, scope: Scope) -> Option<ProductInfo> {
        self.installed
            .get(&(product_code.to_owned(), scope))
            .cloned()
    }
}

/// An installation folder holding the executable; `build_info` adds `BUILD-INFO.json`.
fn installation(root: &Path, build_info: bool) -> PathBuf {
    let folder = root.join("Midnight Explorer (MSI)");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(EXECUTABLE), b"executable").unwrap();
    if build_info {
        fs::write(folder.join("BUILD-INFO.json"), b"{}").unwrap();
    }
    folder
}

#[test]
fn a_copy_that_windows_installer_records_in_its_folder_is_detected_with_its_scope() {
    let root = tempfile::tempdir().unwrap();
    let folder = installation(root.path(), false);
    let current = current_version();
    for scope in [Scope::User, Scope::Machine] {
        let products = FakeProducts::default().with(PRODUCT, scope, &folder, &current.to_string());
        let copy = installer::detect(&folder.join(EXECUTABLE), &current, &products).unwrap();
        assert_eq!(
            copy,
            InstalledCopy {
                folder: folder.clone(),
                product_code: PRODUCT.into(),
                scope,
                version: current.clone(),
            }
        );
    }
    // InstallLocation carries a trailing separator and may differ in case.
    let recorded = PathBuf::from(format!(
        "{}{}",
        folder.display().to_string().to_uppercase(),
        std::path::MAIN_SEPARATOR
    ));
    if cfg!(windows) {
        let products =
            FakeProducts::default().with(PRODUCT, Scope::User, &recorded, &current.to_string());
        let copy = installer::detect(&folder.join(EXECUTABLE), &current, &products).unwrap();
        assert_eq!(copy.scope, Scope::User);
    }
    // Another product installed elsewhere in the other scope does not matter.
    let elsewhere = root.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let products = FakeProducts::default()
        .with(PRODUCT, Scope::Machine, &folder, &current.to_string())
        .with(
            "{99999999-2222-3333-4444-555555555555}",
            Scope::User,
            &elsewhere,
            "0.6.1",
        );
    let copy = installer::detect(&folder.join(EXECUTABLE), &current, &products).unwrap();
    assert_eq!(copy.scope, Scope::Machine);
}

#[test]
fn a_copy_that_windows_installer_does_not_manage_is_not_detected() {
    let root = tempfile::tempdir().unwrap();
    let folder = installation(root.path(), false);
    let current = current_version();
    let version = current.to_string();
    let executable = folder.join(EXECUTABLE);
    // No product at all, or one installed in another folder.
    assert_eq!(
        installer::detect(&executable, &current, &FakeProducts::default()),
        Err(NotUpdatable::NotAReleaseFolder)
    );
    let other = root.path().join("other");
    fs::create_dir_all(&other).unwrap();
    let elsewhere = FakeProducts::default().with(PRODUCT, Scope::User, &other, &version);
    assert_eq!(
        installer::detect(&executable, &current, &elsewhere),
        Err(NotUpdatable::NotAReleaseFolder)
    );
    // An extracted release folder (BUILD-INFO.json) updates itself, even in that folder.
    let here = FakeProducts::default().with(PRODUCT, Scope::User, &folder, &version);
    fs::write(folder.join("BUILD-INFO.json"), b"{}").unwrap();
    assert_eq!(
        installer::detect(&executable, &current, &here),
        Err(NotUpdatable::NotAReleaseFolder)
    );
    fs::remove_file(folder.join("BUILD-INFO.json")).unwrap();
    // Another executable name (a development build, a renamed copy).
    fs::write(folder.join("other.exe"), b"x").unwrap();
    assert_eq!(
        installer::detect(&folder.join("other.exe"), &current, &here),
        Err(NotUpdatable::NotAReleaseFolder)
    );
    // Windows Installer records another version: the files were replaced by hand.
    let older = FakeProducts::default().with(PRODUCT, Scope::User, &folder, "0.0.1");
    assert!(matches!(
        installer::detect(&executable, &current, &older),
        Err(NotUpdatable::Mismatch(reason)) if reason.contains("0.0.1")
    ));
    // The same folder recorded in both scopes is ambiguous.
    let both = FakeProducts::default()
        .with(PRODUCT, Scope::User, &folder, &version)
        .with(PRODUCT, Scope::Machine, &folder, &version);
    assert!(matches!(
        installer::detect(&executable, &current, &both),
        Err(NotUpdatable::Mismatch(_))
    ));
}

#[test]
fn the_msi_is_the_package_of_an_installed_copy() {
    let version = Version::parse("0.6.3").unwrap();
    let windows = Platform {
        target: "x86_64-pc-windows-msvc",
        archive_suffix: "zip",
        installer: true,
    };
    assert_eq!(
        windows.installer_name(&version).as_deref(),
        Some("midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi")
    );
    assert_eq!(
        windows.package_name(&version, false).as_deref(),
        Some("midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip")
    );
    assert_eq!(
        windows.package_name(&version, true),
        windows.installer_name(&version)
    );
    let linux = Platform {
        target: "x86_64-unknown-linux-gnu",
        archive_suffix: "tar.gz",
        installer: false,
    };
    assert_eq!(linux.installer_name(&version), None);
    // A release without the MSI cannot update an installed copy, but still the archive.
    let listing = serde_json::json!([{
        "tag_name": "v0.6.3", "html_url": "https://github.com/x",
        "assets": [
            {"name": "midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip", "size": 1, "browser_download_url": "https://github.com/a"},
            {"name": "SHA256SUMS", "size": 1, "browser_download_url": "https://github.com/b"},
            {"name": "SHA256SUMS.minisig", "size": 1, "browser_download_url": "https://github.com/c"},
        ]
    }]);
    let release = &parse_releases(&serde_json::to_vec(&listing).unwrap()).unwrap()[0];
    assert_eq!(release.missing_package(&windows, false), None);
    let reason = release.missing_package(&windows, true).unwrap();
    assert!(
        reason.contains("midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi"),
        "{reason}"
    );
    assert!(release.missing_package(&linux, true).is_some());
}

fn request(root: &Path, scope: Scope) -> InstallRequest {
    InstallRequest {
        package: root
            .join(r"updates\download\0.6.3\midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi"),
        sha256: [7; 32],
        scope,
        folder: root.join(r"Programs\Midnight Explorer (MSI)\"),
        to: "0.6.3".into(),
        private_folder: root.join("updates"),
        working_folder: root.join("home"),
        arguments: vec![OsString::from("--left"), OsString::from("C:\\x y")],
    }
}

#[test]
fn msiexec_installs_in_the_copys_scope_and_folder_without_restarting_windows() {
    let root = Path::new(r"C:\Users\me\AppData\Local");
    let user = installer::msiexec_command(&request(root, Scope::User)).unwrap();
    let package = r#""C:\Users\me\AppData\Local\updates\download\0.6.3\midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi""#;
    let folder = r#""C:\Users\me\AppData\Local\Programs\Midnight Explorer (MSI)""#;
    if cfg!(windows) {
        assert_eq!(
            user,
            MsiCommand {
                elevated: false,
                parameters: format!(
                    r#"/i {package} /passive /norestart /l*v "C:\Users\me\AppData\Local\updates\installer.log" ALLUSERS=2 MSIINSTALLPERUSER=1 INSTALLDIR={folder}"#
                ),
            }
        );
        let machine = installer::msiexec_command(&request(root, Scope::Machine)).unwrap();
        assert_eq!(
            machine,
            MsiCommand {
                elevated: true,
                parameters: format!(
                    "/i {package} /passive /norestart ALLUSERS=1 INSTALLDIR={folder}"
                ),
            }
        );
    }
    assert!(!user.elevated);
    // A quote in a path would end the quoted value early: refused.
    let mut quoted = request(root, Scope::User);
    quoted.package = PathBuf::from("C:\\a\"b.msi");
    assert!(installer::msiexec_command(&quoted).is_err());
}

#[test]
fn the_request_travels_as_arguments() {
    let root = if cfg!(windows) {
        Path::new(r"C:\Users\me")
    } else {
        Path::new("/home/me")
    };
    let request = request(root, Scope::Machine);
    let arguments = request.to_arguments();
    assert_eq!(arguments[0], installer::INSTALL_ARGUMENT);
    assert_eq!(
        InstallRequest::from_arguments(&arguments),
        Some(Ok(request.clone()))
    );
    assert_eq!(
        InstallRequest::from_arguments(&[OsString::from("--left")]),
        None
    );
    let mut damaged = arguments.clone();
    damaged[2] = OsString::from("not a digest");
    assert!(matches!(
        InstallRequest::from_arguments(&damaged),
        Some(Err(_))
    ));
    let mut relative = arguments;
    relative[1] = OsString::from("package.msi");
    assert!(matches!(
        InstallRequest::from_arguments(&relative),
        Some(Err(_))
    ));
}

#[test]
fn exit_statuses_say_what_happened() {
    let log = Path::new(r"C:\updates\installer.log");
    let to = "0.6.3";
    assert_eq!(
        installer::outcome_of(0, to, None),
        InstallOutcome::Installed { to: to.into() }
    );
    assert_eq!(
        installer::outcome_of(3010, to, None),
        InstallOutcome::RestartRequired { to: to.into() }
    );
    for cancelled in [1602, 1223] {
        assert_eq!(
            installer::outcome_of(cancelled, to, None),
            InstallOutcome::Cancelled { to: to.into() }
        );
    }
    assert_eq!(
        installer::outcome_of(1618, to, None),
        InstallOutcome::Busy { to: to.into() }
    );
    let InstallOutcome::Failed { reason, .. } = installer::outcome_of(1603, to, Some(log)) else {
        panic!("1603 is a failure");
    };
    assert!(
        reason.contains("1603") && reason.contains("installer.log"),
        "{reason}"
    );
    let InstallOutcome::Failed { reason, .. } = installer::outcome_of(1234, to, None) else {
        panic!("an unknown status is a failure");
    };
    assert!(reason.contains("1234"), "{reason}");
    // What the next start says.
    assert_eq!(
        InstallOutcome::Installed { to: to.into() }.notice(),
        "Updated to version 0.6.3."
    );
    assert!(
        InstallOutcome::RestartRequired { to: to.into() }
            .notice()
            .contains("Restart Windows")
    );
    assert!(
        InstallOutcome::Cancelled { to: to.into() }
            .notice()
            .contains("cancelled. Nothing changed.")
    );
    assert!(
        InstallOutcome::Busy { to: to.into() }
            .notice()
            .contains("another installation")
    );
}

/// Records every step of the hand-off in order.
struct FakeHost {
    events: Rc<RefCell<Vec<String>>>,
    instances_closed: bool,
    pin: Result<(), String>,
    status: Box<dyn Fn() -> io::Result<u32>>,
    result_at_relaunch: RefCell<Option<Option<InstallOutcome>>>,
}

struct Pin(Rc<RefCell<Vec<String>>>);

impl Drop for Pin {
    fn drop(&mut self) {
        self.0.borrow_mut().push("unpin".into());
    }
}

impl FakeHost {
    fn new(status: impl Fn() -> io::Result<u32> + 'static) -> Self {
        Self {
            events: Rc::default(),
            instances_closed: true,
            pin: Ok(()),
            status: Box::new(status),
            result_at_relaunch: RefCell::new(None),
        }
    }

    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }
}

impl Host for FakeHost {
    fn wait_for_instances(&self, _request: &InstallRequest) -> bool {
        self.events.borrow_mut().push("wait".into());
        self.instances_closed
    }

    fn pin(&self, _package: &Path, sha256: &[u8; 32]) -> Result<Box<dyn Any>, String> {
        assert_eq!(sha256, &[7; 32]);
        self.events.borrow_mut().push("pin".into());
        self.pin.clone()?;
        Ok(Box::new(Pin(self.events.clone())))
    }

    fn run(&self, command: &MsiCommand) -> io::Result<u32> {
        self.events
            .borrow_mut()
            .push(format!("msiexec elevated={}", command.elevated));
        (self.status)()
    }

    fn relaunch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        _working_folder: &Path,
    ) -> io::Result<()> {
        assert_eq!(executable.file_name().unwrap(), EXECUTABLE);
        assert_eq!(arguments[0], "--left");
        self.events.borrow_mut().push("relaunch".into());
        // The outcome is on disk before the application starts again.
        let request_folder = executable.parent().unwrap();
        let private = request_folder
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("updates");
        *self.result_at_relaunch.borrow_mut() = Some(peek_result(&private));
        Ok(())
    }
}

fn peek_result(private: &Path) -> Option<InstallOutcome> {
    let text = fs::read(private.join("installer-result.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&text).ok()?;
    serde_json::from_value(value).ok()
}

/// A request in a temporary folder whose package exists.
fn prepared(root: &Path, scope: Scope) -> InstallRequest {
    let request = request(root, scope);
    let request = InstallRequest {
        folder: root.join("Programs").join("Midnight Explorer (MSI)"),
        package: root
            .join("updates")
            .join("download")
            .join("0.6.3")
            .join("midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi"),
        ..request
    };
    fs::create_dir_all(request.package.parent().unwrap()).unwrap();
    fs::write(&request.package, b"msi").unwrap();
    fs::create_dir_all(&request.folder).unwrap();
    request
}

#[test]
fn the_hand_off_checks_the_package_under_its_pin_before_msiexec_and_relaunches_after() {
    let root = tempfile::tempdir().unwrap();
    for scope in [Scope::User, Scope::Machine] {
        let request = prepared(root.path(), scope);
        let host = FakeHost::new(|| Ok(0));
        let outcome = installer::install(&request, &host);
        assert_eq!(outcome, InstallOutcome::Installed { to: "0.6.3".into() });
        assert_eq!(
            host.events(),
            [
                "wait".to_owned(),
                "pin".into(),
                format!("msiexec elevated={}", scope == Scope::Machine),
                "unpin".into(),
                "relaunch".into(),
            ]
        );
        assert_eq!(*host.result_at_relaunch.borrow(), Some(Some(outcome)));
        // The used-up package is removed; the next start reads the outcome once.
        assert!(!request.package.exists());
        assert_eq!(
            installer::startup(&request.private_folder, &request.folder),
            Some(InstallOutcome::Installed { to: "0.6.3".into() })
        );
        assert_eq!(
            installer::startup(&request.private_folder, &request.folder),
            None
        );
    }
}

#[test]
fn a_changed_package_or_an_open_window_never_reaches_msiexec() {
    let root = tempfile::tempdir().unwrap();
    let request = prepared(root.path(), Scope::Machine);
    let mut host = FakeHost::new(|| panic!("msiexec must not start"));
    host.pin = Err("The downloaded installer does not match its signed checksum.".into());
    let outcome = installer::install(&request, &host);
    assert!(
        matches!(&outcome, InstallOutcome::Failed { reason, .. } if reason.contains("signed checksum")),
        "{outcome:?}"
    );
    assert_eq!(host.events(), ["wait", "pin", "relaunch"]);
    // Kept: a failed attempt can be retried without downloading again.
    assert!(request.package.exists());

    let mut host = FakeHost::new(|| panic!("msiexec must not start"));
    host.instances_closed = false;
    let outcome = installer::install(&request, &host);
    assert!(
        matches!(&outcome, InstallOutcome::Failed { reason, .. } if reason.contains("still open")),
        "{outcome:?}"
    );
    assert_eq!(host.events(), ["wait", "relaunch"]);
}

#[test]
fn a_declined_prompt_and_windows_installer_failures_change_nothing() {
    let root = tempfile::tempdir().unwrap();
    let request = prepared(root.path(), Scope::Machine);
    // UAC declined: ShellExecuteEx fails with ERROR_CANCELLED.
    let host = FakeHost::new(|| Err(io::Error::from_raw_os_error(1223)));
    assert_eq!(
        installer::install(&request, &host),
        InstallOutcome::Cancelled { to: "0.6.3".into() }
    );
    assert_eq!(
        host.events(),
        ["wait", "pin", "msiexec elevated=true", "unpin", "relaunch"]
    );
    let host = FakeHost::new(|| Ok(1618));
    assert_eq!(
        installer::install(&request, &host),
        InstallOutcome::Busy { to: "0.6.3".into() }
    );
    assert!(request.package.exists());
    let host = FakeHost::new(|| Ok(3010));
    assert_eq!(
        installer::install(&request, &host),
        InstallOutcome::RestartRequired { to: "0.6.3".into() }
    );
    // The previous version starts again and says so; another copy's record waits.
    assert_eq!(
        installer::startup(&request.private_folder, &root.path().join("other")),
        None
    );
    assert!(
        installer::startup(&request.private_folder, &request.folder)
            .unwrap()
            .notice()
            .contains("Restart Windows")
    );
}

#[test]
fn install_now_runs_the_hand_off_from_a_copy_in_the_private_folder() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("updates");
    let folder = installation(root.path(), false);
    let package = private.join("download").join("0.6.3").join("x.msi");
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    fs::write(&package, b"msi").unwrap();
    let ready = ReadyInstaller {
        copy: InstalledCopy {
            folder: folder.clone(),
            product_code: PRODUCT.into(),
            scope: Scope::User,
            version: current_version(),
        },
        package: package.clone(),
        sha256: [7; 32],
        version: "0.6.3".into(),
        page_url: String::new(),
    };
    let (helper, request) = installer::prepare(
        &ready,
        &folder.join(EXECUTABLE),
        &private,
        root.path().to_owned(),
        vec![OsString::from("--left")],
    )
    .unwrap();
    assert_eq!(helper, private.join("installer").join(EXECUTABLE));
    assert_eq!(fs::read(&helper).unwrap(), b"executable");
    assert_eq!(request.package, package);
    assert_eq!(request.folder, folder);
    assert_eq!(request.scope, Scope::User);
    // The next start removes the copy.
    assert_eq!(installer::startup(&private, &folder), None);
    assert!(!helper.exists());
    // Without the downloaded package nothing is prepared.
    fs::remove_file(&package).unwrap();
    assert!(
        installer::prepare(
            &ready,
            &folder.join(EXECUTABLE),
            &private,
            root.path().to_owned(),
            Vec::new()
        )
        .is_err()
    );
}

#[test]
fn the_hand_off_waits_for_every_running_copy_of_the_installation() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("updates");
    let folder = root.path().join("Midnight Explorer (MSI)");
    let other = root.path().join("Other");
    let running = installer::InstanceLock::acquire(&private, &folder).unwrap();
    let second = installer::InstanceLock::acquire(&private, &folder).unwrap();
    // Another installation's copies do not hold this one.
    assert!(installer::wait_for_instances(&private, &other, Duration::ZERO).is_some());
    let started = Instant::now();
    assert!(installer::wait_for_instances(&private, &folder, Duration::from_millis(300)).is_none());
    assert!(started.elapsed() >= Duration::from_millis(300));
    drop(running);
    drop(second);
    let held = installer::wait_for_instances(&private, &folder, Duration::ZERO).unwrap();
    // While the hand-off holds it, a starting copy runs without the lock.
    assert!(installer::InstanceLock::acquire(&private, &folder).is_none());
    drop(held);
    assert_eq!(
        installer::instance_lock_path(&private, &folder),
        installer::instance_lock_path(&private, &PathBuf::from(format!("{}\\", folder.display())))
    );
}

// The controller with an installed copy: the MSI is fetched and checked, never the
// archive, and "Install now" gets the verified package.

struct Release {
    version: String,
    msi_name: String,
    msi: Vec<u8>,
    sums: Vec<u8>,
    signature: String,
}

fn windows_platform() -> Platform {
    Platform {
        target: "x86_64-pc-windows-msvc",
        archive_suffix: "zip",
        installer: true,
    }
}

fn next_version() -> String {
    let mut version = current_version();
    version.patch += 1;
    version.pre = semver::Prerelease::EMPTY;
    version.to_string()
}

impl Release {
    fn new(key: &TestKey, list_msi: bool) -> Self {
        let version = next_version();
        let parsed = Version::parse(&version).unwrap();
        let msi_name = windows_platform().installer_name(&parsed).unwrap();
        let msi = format!("MSI package {version}").into_bytes();
        let mut sums = format!(
            "{}  {}\n",
            sha256_hex(b"zip"),
            windows_platform().archive_name(&parsed)
        );
        if list_msi {
            sums.push_str(&format!("{}  {msi_name}\n", sha256_hex(&msi)));
        }
        let signature = key.sign(
            sums.as_bytes(),
            &format!("timestamp:1 file:SHA256SUMS release:midnight-explorer-{version}"),
        );
        Self {
            version,
            msi_name,
            msi,
            sums: sums.into_bytes(),
            signature,
        }
    }

    fn serve(&self, transport: &FakeTransport, with_msi: bool) {
        let zip = format!(
            "midnight-explorer-{}-x86_64-pc-windows-msvc.zip",
            self.version
        );
        let mut assets = vec![
            serde_json::json!({"name": zip, "size": 3, "browser_download_url": format!("{BASE}/download/{zip}")}),
            serde_json::json!({"name": "SHA256SUMS", "size": self.sums.len(), "browser_download_url": format!("{BASE}/download/SHA256SUMS")}),
            serde_json::json!({"name": "SHA256SUMS.minisig", "size": self.signature.len(), "browser_download_url": format!("{BASE}/download/SHA256SUMS.minisig")}),
        ];
        if with_msi {
            assets.push(serde_json::json!({"name": self.msi_name, "size": self.msi.len(), "browser_download_url": format!("{BASE}/download/{}", self.msi_name)}));
        }
        let listing = serde_json::json!([{
            "tag_name": format!("v{}", self.version), "prerelease": true,
            "html_url": format!("{BASE}/tag/v{}", self.version), "assets": assets,
        }]);
        transport.answer(API, Answer::ok(serde_json::to_vec(&listing).unwrap()));
        transport.answer(
            &format!("{BASE}/download/{}", self.msi_name),
            Answer::ok(self.msi.clone()),
        );
        transport.answer(
            &format!("{BASE}/download/SHA256SUMS"),
            Answer::ok(self.sums.clone()),
        );
        transport.answer(
            &format!("{BASE}/download/SHA256SUMS.minisig"),
            Answer::ok(self.signature.clone().into_bytes()),
        );
    }
}

struct Fixture {
    _folder: tempfile::TempDir,
    private: PathBuf,
    transport: Arc<FakeTransport>,
    controller: UpdateController,
}

fn fixture(key: &TestKey, startup: StartupReport, serve: impl FnOnce(&FakeTransport)) -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let private = folder.path().join("private");
    let transport = FakeTransport::new();
    serve(&transport);
    let published = Arc::new(Mutex::new(Vec::<Snapshot>::new()));
    let controller = UpdateController::start(
        Environment {
            client: UpdateClient::new(transport.clone(), Endpoints::github()),
            anchor: Some(key.anchor()),
            current: current_version(),
            platform: Some(windows_platform()),
            installation: Err(NotUpdatable::NotAReleaseFolder),
            installer: Some(InstalledCopy {
                folder: folder.path().join("install"),
                product_code: PRODUCT.into(),
                scope: Scope::Machine,
                version: current_version(),
            }),
            private_folder: private.clone(),
            startup,
        },
        Box::new(move |snapshot| published.lock().unwrap().push(snapshot)),
    );
    Fixture {
        _folder: folder,
        private,
        transport,
        controller,
    }
}

fn wait_for(fixture: &Fixture, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = fixture.controller.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "timed out: {snapshot:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn settled(snapshot: &Snapshot) -> bool {
    matches!(snapshot.phase, Phase::Ready | Phase::Failed { .. })
}

#[test]
fn an_installed_copy_downloads_and_verifies_the_msi_and_never_the_archive() {
    let key = TestKey::generate();
    let release = Release::new(&key, true);
    let fixture = fixture(&key, StartupReport::default(), |transport| {
        release.serve(transport, true)
    });
    let snapshot = wait_for(&fixture, |snapshot| snapshot.installer.is_some());
    assert_eq!(snapshot.installer, Some(Scope::Machine));
    assert_eq!(snapshot.unavailable, None);
    fixture.controller.check(true, true);
    let snapshot = wait_for(&fixture, |snapshot| snapshot.offer.is_some());
    assert_eq!(snapshot.offer.unwrap().blocked, None);
    fixture.controller.download();
    let snapshot = wait_for(&fixture, settled);
    assert_eq!(snapshot.phase, Phase::Ready, "{snapshot:?}");
    let ready = fixture.controller.installer_ready().unwrap();
    assert_eq!(
        ready.package,
        fixture
            .private
            .join("download")
            .join(&release.version)
            .join(&release.msi_name)
    );
    assert_eq!(fs::read(&ready.package).unwrap(), release.msi);
    let digest: String = ready
        .sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(digest, sha256_hex(&release.msi));
    assert_eq!(ready.version, release.version);
    assert_eq!(ready.copy.scope, Scope::Machine);
    assert!(fixture.controller.staged().is_none());
    let requests = fixture.transport.requests.lock().unwrap();
    assert!(!requests.iter().any(|request| request.url.ends_with(".zip")));
}

#[test]
fn an_msi_that_differs_from_its_signed_line_or_is_not_listed_is_refused() {
    let key = TestKey::generate();
    let mut tampered = Release::new(&key, true);
    tampered.msi.push(0);
    let unlisted = Release::new(&key, false);
    for (release, expected) in [
        (tampered, "does not match its signed checksum"),
        (unlisted, "does not name"),
    ] {
        let fixture = fixture(&key, StartupReport::default(), |transport| {
            release.serve(transport, true)
        });
        fixture.controller.check(true, true);
        wait_for(&fixture, |snapshot| snapshot.offer.is_some());
        fixture.controller.download();
        let snapshot = wait_for(&fixture, settled);
        assert!(
            matches!(&snapshot.phase, Phase::Failed { message } if message.contains(expected)),
            "{snapshot:?}"
        );
        assert!(fixture.controller.installer_ready().is_none());
    }
}

#[test]
fn a_release_without_an_msi_offers_its_page_to_an_installed_copy() {
    let key = TestKey::generate();
    let release = Release::new(&key, true);
    let fixture = fixture(&key, StartupReport::default(), |transport| {
        release.serve(transport, false)
    });
    fixture.controller.check(true, true);
    let snapshot = wait_for(&fixture, |snapshot| snapshot.offer.is_some());
    let blocked = snapshot.offer.unwrap().blocked.unwrap();
    assert!(blocked.contains(".msi"), "{blocked}");
}

#[test]
fn the_start_after_a_windows_installer_update_says_how_it_went() {
    let key = TestKey::generate();
    let fixture = fixture(
        &key,
        StartupReport {
            installer: Some(InstallOutcome::Cancelled { to: "0.6.3".into() }),
            ..StartupReport::default()
        },
        |_| {},
    );
    let snapshot = wait_for(&fixture, |snapshot| snapshot.notice.is_some());
    assert_eq!(
        snapshot.notice.as_deref(),
        Some("The update to version 0.6.3 was cancelled. Nothing changed.")
    );
}

/// The real pin: while it is held, the package can be neither changed, renamed nor
/// deleted, nor its folder renamed; a changed package is refused.
#[cfg(windows)]
#[test]
fn the_pinned_package_cannot_change_until_msiexec_has_finished() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("0.6.3");
    fs::create_dir_all(&folder).unwrap();
    let package = folder.join("p.msi");
    fs::write(&package, b"package").unwrap();
    let digest: [u8; 32] = Sha256::digest(b"package").into();
    let host = installer::SystemHost;
    let pin = host.pin(&package, &digest).unwrap();
    // Readers (msiexec) may open it.
    assert_eq!(fs::read(&package).unwrap(), b"package");
    assert!(fs::OpenOptions::new().append(true).open(&package).is_err());
    assert!(fs::rename(&package, folder.join("q.msi")).is_err());
    assert!(fs::remove_file(&package).is_err());
    assert!(fs::rename(&folder, root.path().join("moved")).is_err());
    drop(pin);
    fs::write(&package, b"changed").unwrap();
    let refused = host.pin(&package, &digest).err().unwrap();
    assert!(refused.contains("signed checksum"), "{refused}");
    fs::rename(&folder, root.path().join("moved")).unwrap();
}

/// msi.dll answers for an upgrade code that no product has; nothing is installed.
#[cfg(windows)]
#[test]
fn windows_installer_is_asked_without_changing_anything() {
    let products = installer::SystemProducts;
    assert!(
        products
            .related("{0D5C64A1-2F7E-4B1A-9C3D-7E8F90A1B2C3}")
            .is_empty()
    );
    assert_eq!(
        products.product("{0D5C64A1-2F7E-4B1A-9C3D-7E8F90A1B2C3}", Scope::User),
        None
    );
}
