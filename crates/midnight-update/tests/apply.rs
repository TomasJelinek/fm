//! The installation swap in temporary folders: success, rollback when the new version
//! fails its startup check, other running copies, a staged copy changed after staging,
//! and the user's settings, which live outside the installation, untouched.
use crate::support::{platform, release_tree, write_tree};
use midnight_update::{
    apply::{self, ApplyRequest, Launcher, Outcome, Started, Timings},
    install::{InstanceLock, work_folder},
};
use std::{
    cell::RefCell,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub fn fast() -> Timings {
    Timings {
        instances: Duration::from_millis(400),
        rename: Duration::from_millis(400),
        confirm: Duration::from_secs(5),
        poll: Duration::from_millis(20),
    }
}

/// What the fake new version does once started.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Writes `confirmed` like a version whose window showed.
    Confirms,
    /// Exits before its startup check.
    Crashes,
    /// Keeps running without confirming in time, like a slow first start.
    Slow,
}

/// One program the updater started.
pub struct Launch {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_folder: PathBuf,
    /// What the program is: the content of its executable file.
    pub content: String,
}

pub struct FakeLauncher {
    pub installation: PathBuf,
    pub behaviour: Behaviour,
    pub launched: RefCell<Vec<Launch>>,
    /// Whether a started program still found the staged release offered as ready.
    pub saw_ready: RefCell<bool>,
}

struct Exited(Option<i32>);

impl Started for Exited {
    fn exited(&mut self) -> io::Result<Option<i32>> {
        Ok(self.0)
    }
}

impl Launcher for FakeLauncher {
    fn launch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        working_folder: &Path,
    ) -> io::Result<Box<dyn Started>> {
        // What the started program is: the content of the executable file.
        let content = std::fs::read_to_string(executable)?;
        if work_folder(&self.installation)
            .join("staging")
            .join("ready.json")
            .exists()
        {
            *self.saw_ready.borrow_mut() = true;
        }
        self.launched.borrow_mut().push(Launch {
            executable: executable.to_path_buf(),
            arguments: arguments.to_vec(),
            working_folder: working_folder.to_path_buf(),
            content: content.clone(),
        });
        let version = content.trim_start_matches("executable ").to_owned();
        // As the application does at start, without consuming `result.json`.
        let pending = std::fs::read_to_string(work_folder(&self.installation).join("pending.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .filter(|pending| pending["to"] == version.as_str())
            .map(|_| version);
        match (pending, self.behaviour) {
            (Some(version), Behaviour::Confirms) => {
                apply::confirm(&self.installation, &version)?;
                Ok(Box::new(Exited(None)))
            }
            (Some(_), Behaviour::Crashes) => Ok(Box::new(Exited(Some(3)))),
            (Some(_), Behaviour::Slow) => Ok(Box::new(Exited(None))),
            // The previous version started again after a rollback.
            (None, _) => Ok(Box::new(Exited(None))),
        }
    }
}

pub struct Setup {
    pub folder: tempfile::TempDir,
    pub installation: PathBuf,
    pub request: ApplyRequest,
    pub settings: PathBuf,
}

/// An installation of 0.5.0 with a staged 0.5.1, and a settings file elsewhere.
pub fn setup(new_binary: &str) -> Setup {
    let folder = tempfile::tempdir().unwrap();
    let installation = folder.path().join("Midnight Explorer");
    write_tree(
        &installation,
        &release_tree(
            "0.5.0",
            "me.exe",
            &[("viewers/old/plugin.dll", b"old plugin")],
        ),
    );
    std::fs::write(installation.join("obsolete.txt"), b"only in 0.5.0").unwrap();
    let root = platform().archive_root(&semver::Version::parse("0.5.1").unwrap());
    let staged = work_folder(&installation).join("staging").join(&root);
    write_tree(
        &staged,
        &release_tree(
            "0.5.1",
            new_binary,
            &[("viewers/old/plugin.dll", b"new plugin")],
        ),
    );
    std::fs::write(
        work_folder(&installation)
            .join("staging")
            .join("ready.json"),
        format!(r#"{{"version":"0.5.1","folder":"{root}","page_url":""}}"#),
    )
    .unwrap();
    let settings = folder.path().join("profile").join("preferences.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, br#"{"version":39,"tabs":["left","right"]}"#).unwrap();
    let request = ApplyRequest {
        installation: installation.clone(),
        staged,
        working_folder: folder.path().to_path_buf(),
        arguments: vec!["--left".into(), "C:/data".into()],
    };
    Setup {
        folder,
        installation,
        request,
        settings,
    }
}

fn launcher(setup: &Setup, behaviour: Behaviour) -> FakeLauncher {
    FakeLauncher {
        installation: setup.installation.clone(),
        behaviour,
        launched: RefCell::new(Vec::new()),
        saw_ready: RefCell::new(false),
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn a_confirmed_update_replaces_the_installation() {
    let setup = setup("me.exe");
    let launcher = launcher(&setup, Behaviour::Confirms);
    let outcome = apply::apply(&setup.request, &launcher, fast());
    assert_eq!(
        outcome,
        Outcome::Updated {
            from: "0.5.0".into(),
            to: "0.5.1".into()
        }
    );
    assert_eq!(read(&setup.installation.join("me.exe")), "executable 0.5.1");
    assert_eq!(
        read(&setup.installation.join("viewers/old/plugin.dll")),
        "new plugin"
    );
    assert!(!setup.installation.join("obsolete.txt").exists());
    let work = work_folder(&setup.installation);
    assert!(
        !work.join("backup").exists(),
        "the backup stays only until the start is confirmed"
    );
    assert!(!work.join("pending.json").exists());
    // The new version starts without the used-up staged copy on offer.
    assert!(!*launcher.saw_ready.borrow());
    assert!(!setup.request.staged.exists());
    // Started once, as the new version, with the original arguments and folder.
    let launched = launcher.launched.borrow();
    assert_eq!(launched.len(), 1);
    assert_eq!(launched[0].executable, setup.installation.join("me.exe"));
    assert_eq!(launched[0].arguments, setup.request.arguments);
    assert_eq!(launched[0].working_folder, setup.request.working_folder);
    assert_eq!(
        read(&setup.settings),
        r#"{"version":39,"tabs":["left","right"]}"#
    );
    // The next start reads the outcome once.
    let report = apply::startup(&setup.installation, "0.5.1");
    assert_eq!(report.outcome, Some(outcome));
    assert_eq!(apply::startup(&setup.installation, "0.5.1").outcome, None);
}

#[test]
fn a_renamed_executable_is_started_by_its_new_name() {
    let setup = setup("midnight-explorer.exe");
    let launcher = launcher(&setup, Behaviour::Confirms);
    assert!(matches!(
        apply::apply(&setup.request, &launcher, fast()),
        Outcome::Updated { .. }
    ));
    assert!(!setup.installation.join("me.exe").exists());
    assert_eq!(
        launcher.launched.borrow()[0].executable,
        setup.installation.join("midnight-explorer.exe")
    );
}

#[test]
fn a_new_version_that_fails_its_startup_check_is_rolled_back() {
    let setup = setup("me.exe");
    let launcher = launcher(&setup, Behaviour::Crashes);
    let outcome = apply::apply(&setup.request, &launcher, fast());
    match &outcome {
        Outcome::RolledBack { from, to, reason } => {
            assert_eq!((from.as_str(), to.as_str()), ("0.5.0", "0.5.1"));
            assert!(reason.contains("startup check"), "{reason}");
        }
        other => panic!("expected a rollback, got {other:?}"),
    }
    assert_eq!(read(&setup.installation.join("me.exe")), "executable 0.5.0");
    assert_eq!(
        read(&setup.installation.join("viewers/old/plugin.dll")),
        "old plugin"
    );
    assert_eq!(
        read(&setup.installation.join("obsolete.txt")),
        "only in 0.5.0"
    );
    // The new version first, then the previous one again with the same arguments.
    let launched = launcher.launched.borrow();
    assert_eq!(launched.len(), 2);
    assert_eq!(launched[0].content, "executable 0.5.1");
    assert_eq!(launched[1].content, "executable 0.5.0");
    assert_eq!(launched[1].arguments, setup.request.arguments);
    assert!(
        read(&setup.settings).contains("tabs"),
        "settings are not touched"
    );
    let report = apply::startup(&setup.installation, "0.5.0");
    assert!(matches!(report.outcome, Some(Outcome::RolledBack { .. })));
    assert!(!work_folder(&setup.installation).join("failed").exists());
}

#[test]
fn another_running_copy_postpones_the_update() {
    let setup = setup("me.exe");
    let other = InstanceLock::acquire(&setup.installation).expect("a release folder");
    let launcher = launcher(&setup, Behaviour::Confirms);
    let outcome = apply::apply(&setup.request, &launcher, fast());
    match outcome {
        Outcome::NotApplied { reason } => assert!(reason.contains("still open"), "{reason}"),
        other => panic!("expected no change, got {other:?}"),
    }
    assert_eq!(read(&setup.installation.join("me.exe")), "executable 0.5.0");
    drop(other);
    // The previous version is started again so the user is not left without a window.
    assert_eq!(launcher.launched.borrow()[0].content, "executable 0.5.0");
}

#[test]
fn a_staged_copy_changed_after_staging_is_not_installed() {
    let setup = setup("me.exe");
    std::fs::write(
        setup.request.staged.join("viewers/old/plugin.dll"),
        b"tampered",
    )
    .unwrap();
    let launcher = launcher(&setup, Behaviour::Confirms);
    match apply::apply(&setup.request, &launcher, fast()) {
        Outcome::NotApplied { reason } => assert!(reason.contains("plugin.dll"), "{reason}"),
        other => panic!("expected no change, got {other:?}"),
    }
    assert_eq!(read(&setup.installation.join("me.exe")), "executable 0.5.0");
}

#[test]
fn a_staged_folder_outside_the_update_folder_is_refused() {
    let mut setup = setup("me.exe");
    let elsewhere = setup.folder.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    setup.request.staged = elsewhere;
    let launcher = launcher(&setup, Behaviour::Confirms);
    assert!(matches!(
        apply::apply(&setup.request, &launcher, fast()),
        Outcome::NotApplied { .. }
    ));
}

#[test]
fn updater_arguments_round_trip() {
    let setup = setup("me.exe");
    let arguments = setup.request.to_arguments();
    assert_eq!(arguments[0], "--apply-update");
    assert_eq!(
        ApplyRequest::from_arguments(&arguments).unwrap().unwrap(),
        setup.request
    );
    assert!(ApplyRequest::from_arguments(&["--left".into()]).is_none());
    assert!(
        ApplyRequest::from_arguments(&["--apply-update".into()])
            .unwrap()
            .is_err()
    );
}

#[test]
fn a_start_after_an_interrupted_update_leaves_the_updater_alone() {
    let setup = setup("me.exe");
    let work = work_folder(&setup.installation);
    std::fs::write(
        work.join("pending.json"),
        br#"{"from":"0.5.0","to":"0.5.1"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(work.join("backup").join("x")).unwrap();
    // The previous version starting meanwhile neither confirms nor cleans up.
    let report = apply::startup(&setup.installation, "0.5.0");
    assert_eq!(report.confirm, None);
    assert!(work.join("backup").exists());
    // The new version confirms.
    let report = apply::startup(&setup.installation, "0.5.1");
    assert_eq!(report.confirm.as_deref(), Some("0.5.1"));
}

/// The updater stops waiting for a slow first start (`Unconfirmed`) and keeps the backup.
/// Once that start confirms, the next start finishes the update: it removes the backup
/// and no longer reports the update as new.
#[test]
fn a_late_confirmation_finishes_the_update_at_the_next_start() {
    let setup = setup("me.exe");
    let launcher = launcher(&setup, Behaviour::Slow);
    let timings = Timings {
        confirm: Duration::from_millis(100),
        ..fast()
    };
    let outcome = apply::apply(&setup.request, &launcher, timings);
    assert!(
        matches!(outcome, Outcome::Unconfirmed { .. }),
        "{outcome:?}"
    );
    let work = work_folder(&setup.installation);
    assert!(
        work.join("backup").exists(),
        "kept until the start confirms"
    );
    // The slow first start read the update in progress at its own start, and confirms.
    apply::confirm(&setup.installation, "0.5.1").unwrap();
    let report = apply::startup(&setup.installation, "0.5.1");
    assert_eq!(
        report.confirm, None,
        "the update was confirmed by an earlier start"
    );
    assert!(!work.join("backup").exists());
    assert!(!work.join("pending.json").exists());
    assert!(!work.join("confirmed").exists());
    assert_eq!(apply::startup(&setup.installation, "0.5.1").confirm, None);
}

/// Until the slow start confirms, a later start of the new version confirms instead.
#[test]
fn an_unconfirmed_update_is_confirmed_by_a_later_start() {
    let setup = setup("me.exe");
    let launcher = launcher(&setup, Behaviour::Slow);
    let timings = Timings {
        confirm: Duration::from_millis(100),
        ..fast()
    };
    assert!(matches!(
        apply::apply(&setup.request, &launcher, timings),
        Outcome::Unconfirmed { .. }
    ));
    let report = apply::startup(&setup.installation, "0.5.1");
    assert_eq!(report.confirm.as_deref(), Some("0.5.1"));
    assert!(work_folder(&setup.installation).join("backup").exists());
}
