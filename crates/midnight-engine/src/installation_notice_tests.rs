use super::*;
use midnight_core::{ComparatorValidationProof, FsError, UserPreferences};
use std::time::Duration;
use tokio::sync::mpsc;

/// In-memory dismissed notices; preferences are not used here.
#[derive(Default)]
struct Notices(Mutex<Vec<String>>);

impl SettingsService for Notices {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences_prepared(
        &self,
        _: UserPreferences,
        _: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn load_dismissed_notices(&self) -> Result<Vec<String>, FsError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save_dismissed_notices(&self, keys: &[String]) -> Result<(), FsError> {
        *self.0.lock().unwrap() = keys.to_vec();
        Ok(())
    }
}

/// A fixture folder: below `HOST_TEST_INSTALL_ROOT` when set (private ancestors).
fn base() -> tempfile::TempDir {
    match std::env::var_os("HOST_TEST_INSTALL_ROOT") {
        Some(root) => tempfile::tempdir_in(root).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

/// Let other accounts change `folder` and everything created in it later.
fn share(folder: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    #[cfg(windows)]
    {
        // S-1-5-11 is Authenticated Users; Modify, inherited by files created later.
        let status = std::process::Command::new("icacls")
            .arg(folder)
            .args(["/grant", "*S-1-5-11:(OI)(CI)M", "/Q"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}

struct Fixture {
    _base: tempfile::TempDir,
    folder: PathBuf,
    profile: PathBuf,
    notices: Arc<Notices>,
    launched: Arc<Mutex<Vec<PathBuf>>>,
}

impl Fixture {
    fn new() -> Self {
        let base = base();
        let folder = base.path().join("extracted");
        std::fs::create_dir(&folder).unwrap();
        share(&folder);
        std::fs::write(folder.join("midnight-explorer.exe"), b"application").unwrap();
        std::fs::create_dir(folder.join("viewers")).unwrap();
        std::fs::write(folder.join("viewers/LICENSE.txt"), b"licence").unwrap();
        let profile = base.path().join("profile").join("Midnight Explorer");
        Self {
            _base: base,
            folder,
            profile,
            notices: Arc::default(),
            launched: Arc::default(),
        }
    }

    fn start(&self) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
        let (sender, events) = mpsc::channel(4);
        let mut tasks = JoinSet::new();
        let folder = self.folder.clone();
        let launched = self.launched.clone();
        let controller = Controller::new(
            TaskLifetime::testing(()),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            Environment {
                installation: Arc::new(move || Ok(folder.clone())),
                profile: Some(self.profile.clone()),
                settings: Some(self.notices.clone()),
                launch: Arc::new(move |folder| {
                    launched.lock().unwrap().push(folder.to_owned());
                    Ok(())
                }),
            },
            &mut tasks,
        );
        (controller, tasks, events)
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> InstallationEvent {
    let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::Installation(event) = event else {
        panic!("expected an installation event")
    };
    *event
}

#[tokio::test]
async fn a_shared_installation_is_reported_until_its_notice_is_dismissed() {
    let fixture = Fixture::new();
    let (controller, mut tasks, mut events) = fixture.start();
    let InstallationEvent::Checked(Some(status)) = next(&mut events).await else {
        panic!("a shared installation is reported")
    };
    assert_eq!(status.folder, fixture.folder);
    assert!(status.shared_count >= 1, "{status:?}");
    controller.request(InstallationRequest::Dismiss, &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert_eq!(
        *fixture.notices.0.lock().unwrap(),
        [dismissal_key(&fixture.folder)]
    );
    let (_controller, _tasks, mut events) = fixture.start();
    assert_eq!(next(&mut events).await, InstallationEvent::Checked(None));
}

#[tokio::test]
async fn copying_to_the_profile_asks_before_replacing_and_restarts_only_a_private_copy() {
    let fixture = Fixture::new();
    let (controller, mut tasks, mut events) = fixture.start();
    assert!(matches!(
        next(&mut events).await,
        InstallationEvent::Checked(Some(_))
    ));
    controller.request(InstallationRequest::RestartFromCopy, &mut tasks);
    assert!(matches!(
        next(&mut events).await,
        InstallationEvent::RestartFailed(_)
    ));
    controller.request(
        InstallationRequest::CopyToProfile { replace: false },
        &mut tasks,
    );
    let InstallationEvent::Copied {
        folder,
        files,
        status,
        ..
    } = next(&mut events).await
    else {
        panic!("copied")
    };
    assert_eq!(files, 2);
    assert_eq!(
        std::fs::read(folder.join("viewers/LICENSE.txt")).unwrap(),
        b"licence"
    );
    controller.request(
        InstallationRequest::CopyToProfile { replace: false },
        &mut tasks,
    );
    assert_eq!(
        next(&mut events).await,
        InstallationEvent::CopyExists(fixture.profile.clone())
    );
    controller.request(InstallationRequest::RestartFromCopy, &mut tasks);
    let restarted = next(&mut events).await;
    // The copy is private when the fixture folder's ancestors are (HOST_TEST_INSTALL_ROOT).
    if status.is_private() {
        assert_eq!(restarted, InstallationEvent::Restarting);
        assert_eq!(*fixture.launched.lock().unwrap(), [folder]);
    } else {
        assert!(matches!(restarted, InstallationEvent::RestartFailed(_)));
        assert!(fixture.launched.lock().unwrap().is_empty());
    }
}
