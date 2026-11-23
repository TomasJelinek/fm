//! The controller as the UI uses it: requests return at once, snapshots tell the
//! progress, and the whole check → download → verify → stage flow runs on a scripted
//! transport.
use crate::support::{
    Answer, FakeTransport, ReleaseFiles, TestKey, platform, release_tree, write_tree,
};
use midnight_update::{
    apply::StartupReport,
    client::{Endpoints, UpdateClient},
    controller::{Environment, Phase, Snapshot, UpdateController},
    install::Installation,
    release::current_version,
};
use semver::Version;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const API: &str = "https://api.github.com/repos/TomasJelinek/fm/releases?per_page=30";
const BASE: &str = "https://github.com/TomasJelinek/fm/releases";

struct Fixture {
    folder: tempfile::TempDir,
    transport: Arc<FakeTransport>,
    controller: UpdateController,
    published: Arc<Mutex<Vec<Snapshot>>>,
}

fn next_version() -> String {
    let mut version = current_version();
    version.patch += 1;
    version.pre = semver::Prerelease::EMPTY;
    version.to_string()
}

fn listing(entries: &[serde_json::Value]) -> Answer {
    Answer::ok(serde_json::to_vec(entries).unwrap())
}

fn fixture(key: Option<&TestKey>, configure: impl FnOnce(&FakeTransport)) -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let installation = folder.path().join("install");
    write_tree(
        &installation,
        &release_tree(&current_version().to_string(), "me.exe", &[]),
    );
    let transport = FakeTransport::new();
    configure(&transport);
    let environment = Environment {
        client: UpdateClient::new(transport.clone(), Endpoints::github()),
        anchor: key.map(TestKey::anchor),
        current: current_version(),
        platform: Some(platform()),
        installation: Installation::detect(&installation.join("me.exe"), &platform()),
        private_folder: folder.path().join("private"),
        startup: StartupReport::default(),
    };
    assert!(environment.installation.is_ok());
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = published.clone();
    let controller = UpdateController::start(
        environment,
        Box::new(move |snapshot| sink.lock().unwrap().push(snapshot)),
    );
    Fixture {
        folder,
        transport,
        controller,
        published,
    }
}

fn wait_for(fixture: &Fixture, what: &str, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = fixture.controller.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn settled(snapshot: &Snapshot) -> bool {
    !matches!(
        snapshot.phase,
        Phase::Checking | Phase::Downloading { .. } | Phase::Verifying
    )
}

/// A check has answered: an offer, up to date, or a failure.
fn answered(snapshot: &Snapshot) -> bool {
    snapshot.offer.is_some() || matches!(snapshot.phase, Phase::UpToDate | Phase::Failed { .. })
}

#[test]
fn requests_return_at_once_while_github_is_slow() {
    let key = TestKey::generate();
    let fixture = fixture(Some(&key), |transport| {
        transport.answer(API, listing(&[]));
        transport.hold();
    });
    let started = Instant::now();
    fixture.controller.check(true, true);
    fixture.controller.check(true, true);
    let snapshot = fixture.controller.snapshot();
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "the caller waited {:?}",
        started.elapsed()
    );
    assert!(matches!(snapshot.phase, Phase::Idle | Phase::Checking));
    wait_for(&fixture, "checking", |snapshot| {
        snapshot.phase == Phase::Checking
    });
    fixture.transport.release();
    wait_for(&fixture, "up to date", |snapshot| {
        snapshot.phase == Phase::UpToDate
    });
}

#[test]
fn a_newer_release_downloads_verifies_and_stages() {
    let key = TestKey::generate();
    let version = next_version();
    let files = ReleaseFiles::new(&key, &version, "me.exe");
    let fixture = fixture(Some(&key), |transport| {
        transport.answer(API, listing(&[files.listing_entry(BASE, true)]));
        files.serve(transport, BASE);
    });
    fixture.controller.check(true, true);
    let snapshot = wait_for(&fixture, "an offer", answered);
    assert_eq!(snapshot.phase, Phase::Available);
    let offer = snapshot.offer.unwrap();
    assert_eq!(offer.version, version);
    assert!(
        offer.notes.contains("• Faster scrolling"),
        "{}",
        offer.notes
    );
    assert_eq!(offer.blocked, None);
    assert!(offer.preview);
    fixture.controller.download();
    let snapshot = wait_for(&fixture, "staging", |snapshot| {
        snapshot.phase == Phase::Ready || matches!(snapshot.phase, Phase::Failed { .. })
    });
    assert_eq!(snapshot.phase, Phase::Ready, "{snapshot:?}");
    let (installation, staged) = fixture.controller.staged().unwrap();
    assert_eq!(staged.version, version);
    assert!(
        installation
            .staging()
            .join(&staged.folder)
            .join("me.exe")
            .is_file()
    );
    // The archive itself is removed once staged.
    assert!(
        !fixture
            .folder
            .path()
            .join("private/download")
            .join(&version)
            .exists()
    );
    let progress = fixture.published.lock().unwrap().iter().any(|snapshot| {
        matches!(snapshot.phase, Phase::Downloading { received, total } if received == total && total > 0)
    });
    assert!(progress, "the download reported its progress");
}

#[test]
fn a_tampered_archive_is_never_staged() {
    let key = TestKey::generate();
    let version = next_version();
    let mut files = ReleaseFiles::new(&key, &version, "me.exe");
    files.archive.push(0);
    let fixture = fixture(Some(&key), |transport| {
        transport.answer(API, listing(&[files.listing_entry(BASE, false)]));
        files.serve(transport, BASE);
    });
    fixture.controller.check(true, true);
    wait_for(&fixture, "an offer", answered);
    fixture.controller.download();
    let snapshot = wait_for(&fixture, "a refusal", |snapshot| {
        matches!(snapshot.phase, Phase::Failed { .. } | Phase::Ready)
    });
    match snapshot.phase {
        Phase::Failed { message } => assert!(message.contains("does not match"), "{message}"),
        other => panic!("a tampered archive was accepted: {other:?}"),
    }
    assert!(fixture.controller.staged().is_none());
}

#[test]
fn a_release_signed_with_another_key_downloads_no_archive() {
    let signer = TestKey::generate();
    let trusted = TestKey::generate();
    let version = next_version();
    let files = ReleaseFiles::new(&signer, &version, "me.exe");
    let fixture = fixture(Some(&trusted), |transport| {
        transport.answer(API, listing(&[files.listing_entry(BASE, false)]));
        files.serve(transport, BASE);
    });
    fixture.controller.check(true, true);
    wait_for(&fixture, "an offer", answered);
    fixture.controller.download();
    let snapshot = wait_for(&fixture, "a refusal", |snapshot| {
        matches!(snapshot.phase, Phase::Failed { .. } | Phase::Ready)
    });
    assert!(
        matches!(&snapshot.phase, Phase::Failed { message } if message.contains("key")),
        "{snapshot:?}"
    );
    let requests = fixture.transport.requests.lock().unwrap();
    assert!(
        !requests
            .iter()
            .any(|request| request.url.ends_with(".zip") || request.url.ends_with(".tar.gz")),
        "the archive was downloaded before the signature was checked"
    );
}

#[test]
fn a_build_without_a_key_says_so_and_installs_nothing() {
    let key = TestKey::generate();
    let version = next_version();
    let files = ReleaseFiles::new(&key, &version, "me.exe");
    let fixture = fixture(None, |transport| {
        transport.answer(API, listing(&[files.listing_entry(BASE, false)]));
        files.serve(transport, BASE);
    });
    let snapshot = wait_for(&fixture, "the reason", |snapshot| {
        snapshot.unavailable.is_some()
    });
    assert!(snapshot.unavailable.unwrap().contains("signing key"));
    fixture.controller.check(true, true);
    wait_for(&fixture, "an offer", answered);
    fixture.controller.download();
    let snapshot = wait_for(&fixture, "a refusal", |snapshot| {
        matches!(snapshot.phase, Phase::Failed { .. } | Phase::Ready)
    });
    assert!(
        matches!(snapshot.phase, Phase::Failed { .. }),
        "{snapshot:?}"
    );
    assert!(fixture.controller.staged().is_none());
}

#[test]
fn the_startup_check_asks_at_most_once_per_interval_and_respects_skip() {
    let key = TestKey::generate();
    let version = next_version();
    let files = ReleaseFiles::new(&key, &version, "me.exe");
    let fixture = fixture(Some(&key), |transport| {
        transport.answer(API, listing(&[files.listing_entry(BASE, false)]));
    });
    let interval = Duration::from_secs(3600);
    fixture.controller.startup_check(true, interval);
    let snapshot = wait_for(&fixture, "an offer", |snapshot| snapshot.offer.is_some());
    assert_eq!(snapshot.phase, Phase::Available);
    fixture.controller.skip();
    let snapshot = wait_for(&fixture, "skipped", |snapshot| {
        snapshot.offer.as_ref().is_some_and(|offer| offer.skipped)
    });
    assert_eq!(snapshot.phase, Phase::Idle);
    // Within the interval the startup check does not ask again.
    fixture.controller.startup_check(true, interval);
    fixture.controller.check(true, true);
    wait_for(&fixture, "a manual check", |snapshot| {
        snapshot.phase == Phase::Available
    });
    assert_eq!(fixture.transport.request_count(), 2);
    // A manual check still shows the skipped version, marked as skipped.
    assert!(fixture.controller.snapshot().offer.unwrap().skipped);
    // A startup check that is due stays quiet about the skipped version.
    fixture.controller.startup_check(true, Duration::ZERO);
    let deadline = Instant::now() + Duration::from_secs(20);
    while fixture.transport.request_count() < 3 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot = wait_for(&fixture, "quiet", settled);
    assert_eq!(snapshot.phase, Phase::Idle);
}

#[test]
fn rate_limits_and_offline_checks_back_off_quietly_at_startup() {
    let fixture = fixture(None, |transport| {
        transport.answer(
            API,
            Answer {
                head: midnight_update::transport::Head {
                    status: 429,
                    retry_after: Some(3600),
                    ..Default::default()
                },
                body: Vec::new(),
                cut_after: None,
            },
        );
    });
    fixture.controller.startup_check(true, Duration::ZERO);
    let deadline = Instant::now() + Duration::from_secs(20);
    while fixture.transport.request_count() < 1 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot = wait_for(&fixture, "quiet", settled);
    assert_eq!(
        snapshot.phase,
        Phase::Idle,
        "a startup check failure stays quiet"
    );
    let state = midnight_update::controller::check_state(&fixture.folder.path().join("private"));
    assert!(state.retry_after > 0);
    // The next startup check waits for GitHub's limit.
    fixture.controller.startup_check(true, Duration::ZERO);
    fixture.controller.check(true, true);
    let snapshot = wait_for(&fixture, "a manual failure", |snapshot| {
        matches!(snapshot.phase, Phase::Failed { .. })
    });
    assert_eq!(
        fixture.transport.request_count(),
        2,
        "only the manual check asked"
    );
    assert!(matches!(&snapshot.phase, Phase::Failed { message } if message.contains("limits")));

    let offline = self::fixture(None, |transport| transport.set_offline(true));
    offline.controller.check(true, true);
    let snapshot = wait_for(&offline, "offline", |snapshot| {
        matches!(snapshot.phase, Phase::Failed { .. })
    });
    assert!(
        matches!(&snapshot.phase, Phase::Failed { message } if message.contains("cannot be reached"))
    );
}

#[test]
fn nothing_newer_is_up_to_date_and_a_staged_update_survives_a_restart() {
    let fixture = fixture(None, |transport| transport.answer(API, listing(&[])));
    fixture.controller.check(true, true);
    wait_for(&fixture, "up to date", |snapshot| {
        snapshot.phase == Phase::UpToDate
    });
    // A release staged earlier is offered as ready at the next start.
    let installation = Installation::detect(
        &fixture.folder.path().join("install").join("me.exe"),
        &platform(),
    )
    .unwrap();
    let version = Version::parse(&next_version()).unwrap();
    let root = platform().archive_root(&version);
    std::fs::create_dir_all(installation.staging().join(&root)).unwrap();
    std::fs::write(
        installation.staging().join("ready.json"),
        serde_json::to_vec(&serde_json::json!({"version": version.to_string(), "folder": root, "page_url": "https://github.com/x"})).unwrap(),
    )
    .unwrap();
    let again = UpdateController::start(
        Environment {
            client: UpdateClient::new(FakeTransport::new(), Endpoints::github()),
            anchor: None,
            current: current_version(),
            platform: Some(platform()),
            installation: Ok(installation),
            private_folder: fixture.folder.path().join("private"),
            startup: StartupReport::default(),
        },
        Box::new(|_| {}),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while again.snapshot().phase != Phase::Ready {
        assert!(Instant::now() < deadline, "{:?}", again.snapshot());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(again.staged().is_some());
}

#[test]
fn the_start_after_an_update_says_how_it_went() {
    let start = |startup: StartupReport| {
        let folder = tempfile::tempdir().unwrap();
        let controller = UpdateController::start(
            Environment {
                client: UpdateClient::new(FakeTransport::new(), Endpoints::github()),
                anchor: None,
                current: current_version(),
                platform: Some(platform()),
                installation: Err(midnight_update::install::NotUpdatable::NotAReleaseFolder),
                private_folder: folder.path().to_path_buf(),
                startup,
            },
            Box::new(|_| {}),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while controller.snapshot().unavailable.is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        controller.snapshot().notice
    };
    // The new version, confirming its start.
    assert_eq!(
        start(StartupReport {
            confirm: Some("0.5.1".into()),
            outcome: None
        })
        .as_deref(),
        Some("Updated to version 0.5.1.")
    );
    // The previous version, started again after a rollback.
    let notice = start(StartupReport {
        confirm: None,
        outcome: Some(midnight_update::apply::Outcome::RolledBack {
            from: "0.5.0".into(),
            to: "0.5.1".into(),
            reason: "The new version exited during its startup check (status 3).".into(),
        }),
    })
    .unwrap();
    assert!(
        notice.starts_with("Version 0.5.1 did not start correctly"),
        "{notice}"
    );
    // A later start does not repeat a successful update.
    assert_eq!(
        start(StartupReport {
            confirm: None,
            outcome: Some(midnight_update::apply::Outcome::Updated {
                from: "0.5.0".into(),
                to: "0.5.1".into()
            })
        }),
        None
    );
}
