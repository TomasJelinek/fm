//! Actual engine navigation using the production app's native archive-capture callback.
use super::*;
use midnight_core::{
    ArchiveAction, ArchiveAddress, ArchiveFormat, ArchiveListing, ArchiveLocation, Cancellation,
    Command, Event, FsError, NavigationId, PaneId, SortSpec,
};
use midnight_engine::Engine;
use tokio::sync::mpsc;

fn installation() -> PathBuf {
    std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
        .unwrap()
        .into()
}
fn fixture() -> PathBuf {
    PathBuf::from(std::env::var_os("MIDNIGHT_NATIVE_MEDIA_TEST_FILES").unwrap())
        .join("restore 文档.gz")
}
fn start(
    provider: Arc<InstalledProvider>,
) -> (
    mpsc::Sender<Command>,
    mpsc::Receiver<Event>,
    tokio::task::JoinHandle<()>,
) {
    let (commands, receiver) = mpsc::channel(16);
    let (sender, events) = mpsc::channel(16);
    let task = tokio::spawn(
        Engine::new(provider.clone(), sender)
            .with_archive_capture(archive(provider))
            .run(receiver),
    );
    (commands, events, task)
}
async fn navigate(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    action: ArchiveAction,
    id: u64,
) -> Result<ArchiveListing, FsError> {
    let navigation = NavigationId(id);
    commands
        .send(Command::ArchiveNavigate {
            pane: PaneId::Left,
            navigation,
            action,
            sort: SortSpec::default(),
        })
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let Event::ArchiveListed {
                navigation: found,
                result,
                ..
            } = event
                && found == navigation
            {
                return result;
            }
        }
        panic!("Engine exited without archive navigation result");
    })
    .await
    .expect("Archive navigation timed out")
}
async fn shutdown(
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
) {
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "run inside protected runtime; requires native/archive packages and owned restore 文档.gz"]
async fn archive_navigation_and_legacy_restore_share_installed_native_authority() {
    let path = fixture();
    let legacy = midnight_plugin_host::CapturedFile::capture(&path)
        .unwrap()
        .stable_version();
    let provider = Arc::new(InstalledProvider::from_installation(installation(), None));
    let (commands, mut events, task) = start(provider.clone());
    let listing = navigate(
        &commands,
        &mut events,
        ArchiveAction::Open {
            path: path.clone(),
            format: ArchiveFormat::Gzip,
        },
        1,
    )
    .await
    .unwrap();
    assert_eq!(listing.location.address.container, path);
    assert_eq!(listing.entries.len(), 1);
    let current = listing.location.address.source_version.clone();
    assert_ne!(
        current, legacy,
        "Exercise the real legacy-to-plugin revision boundary"
    );
    let mut saved = listing.location.address.clone();
    saved.source_version = legacy;
    let restored = navigate(
        &commands,
        &mut events,
        ArchiveAction::List(ArchiveLocation::unavailable(saved.clone())),
        2,
    )
    .await
    .unwrap();
    assert_eq!(restored.location.address.source_version, current);
    assert_eq!(restored.location.address.container, path);
    let mount = restored.location.mount.as_ref().unwrap();
    let members = mount.list(1, &Cancellation::default()).unwrap();
    assert_eq!(
        mount
            .read(members[0].id.entry, 1, 3, &Cancellation::default())
            .unwrap(),
        b"ell"
    );
    std::fs::write(&path, b"changed owned archive fixture").unwrap();
    assert!(mount.list(1, &Cancellation::default()).is_err());
    let failure = navigate(
        &commands,
        &mut events,
        ArchiveAction::List(ArchiveLocation::unavailable(saved)),
        3,
    )
    .await
    .unwrap_err();
    assert!(
        failure.to_string().to_lowercase().contains("revision"),
        "{failure}"
    );
    drop(restored);
    drop(listing);
    shutdown(commands, events, task).await;
}

#[tokio::test]
#[ignore = "run inside protected runtime; requires archive package and existing owned gzip fixture"]
async fn missing_native_plugin_rejects_new_and_restored_archive_open() {
    let path = fixture();
    assert!(path.is_file());
    let provider = Arc::new(InstalledProvider::from_installation(
        installation().join("missing-native-provider"),
        None,
    ));
    let (commands, mut events, task) = start(provider);
    for (index, action) in [
        ArchiveAction::Open {
            path: path.clone(),
            format: ArchiveFormat::Gzip,
        },
        ArchiveAction::List(ArchiveLocation::unavailable(ArchiveAddress {
            provider: "midnight.fs.archive".into(),
            format: Some(ArchiveFormat::Gzip),
            container: path,
            source_version: vec![1],
            components: vec![],
        })),
    ]
    .into_iter()
    .enumerate()
    {
        let failure = navigate(&commands, &mut events, action, index as u64 + 1)
            .await
            .unwrap_err();
        assert!(
            failure.to_string().contains("Native filesystem"),
            "{failure}"
        );
    }
    shutdown(commands, events, task).await;
}

#[tokio::test]
#[ignore = "read-only timing; requires MIDNIGHT_ARCHIVE_TEST_SOURCE and protected runtime"]
async fn measure_supplied_archive_through_production_capture() {
    let path = PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_TEST_SOURCE").unwrap());
    let format = midnight_core::archive_format(&path).expect("known archive suffix");
    let provider = Arc::new(InstalledProvider::from_installation(installation(), None));
    let (commands, mut events, task) = start(provider);
    // Exclude package discovery from navigation, as in an already-started app.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let Event::ArchiveAvailability(formats) = event {
                assert!(formats.contains(&format));
                return;
            }
        }
        panic!("Missing archive availability");
    })
    .await
    .unwrap();
    for iteration in 0..3 {
        let start = std::time::Instant::now();
        let listing = navigate(
            &commands,
            &mut events,
            ArchiveAction::Open {
                path: path.clone(),
                format,
            },
            iteration + 1,
        )
        .await
        .unwrap();
        eprintln!(
            "iteration={iteration} production_open_and_list={:?} entries={}",
            start.elapsed(),
            listing.entries.len()
        );
        let start = std::time::Instant::now();
        let listed = listing
            .location
            .mount
            .as_ref()
            .unwrap()
            .list(listing.location.directory, &Cancellation::default())
            .unwrap();
        eprintln!("relist={:?} entries={}", start.elapsed(), listed.len());
        assert_eq!(listing.location.address.container, path);
        drop(listing);
        // The previous supervisor reaps on its own backend thread.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    shutdown(commands, events, task).await;
}
