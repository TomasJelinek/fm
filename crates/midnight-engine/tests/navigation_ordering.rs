//! Exercise locale ordering through the real asynchronous engine publications.
use midnight_core::*;
use midnight_engine::Engine;
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

struct Fixture;

fn volumes() -> Vec<Volume> {
    ["i", "ch", "h"]
        .into_iter()
        .map(|label| Volume {
            id: format!("opaque-{label}"),
            label: label.into(),
            root: PathBuf::from(format!("raw/../文档/{label}")),
        })
        .collect()
}

fn locations() -> Vec<NavigationLocation> {
    let mut result: Vec<_> = volumes()
        .into_iter()
        .map(|volume| NavigationLocation {
            id: volume.id,
            label: volume.label,
            path: volume.root,
            kind: LocationKind::Volume,
            drive_letter: Some('Z'),
        })
        .collect();
    for (id, label, kind) in [
        ("docs", "h", LocationKind::Documents),
        ("home", "ch", LocationKind::Home),
        ("music", "i", LocationKind::Music),
    ] {
        result.insert(
            0,
            NavigationLocation {
                id: id.into(),
                label: label.into(),
                // Same-path aliases must remain separate navigation choices.
                path: PathBuf::from("raw/../special 👩🏽‍💻"),
                kind,
                drive_letter: None,
            },
        );
    }
    result
}

impl DirectoryService for Fixture {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("navigation discovery must not read entries")
    }
}
impl VolumeService for Fixture {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(volumes())
    }
}
impl StorageService for Fixture {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 0,
            total: 0,
        })
    }
}
impl LocationService for Fixture {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        Ok(locations())
    }
}
impl IconService for Fixture {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}
impl TerminalService for Fixture {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        unreachable!("navigation discovery must not launch terminals")
    }
}
impl PropertyService for Fixture {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        unreachable!("navigation discovery must not show properties")
    }
}
impl OpenService for Fixture {
    fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
        unreachable!("navigation discovery must not open files")
    }
}

#[derive(Default)]
struct Locale {
    calls: Mutex<Vec<std::thread::ThreadId>>,
    fail: bool,
    missing: bool,
    oversized: bool,
}
impl LocaleService for Locale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, label: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        self.calls.lock().unwrap().push(std::thread::current().id());
        if self.fail {
            return Err(FsError::from_io(
                "navigation collation",
                Path::new(""),
                io::Error::other("injected locale failure"),
            ));
        }
        if self.missing {
            return Ok(None);
        }
        if self.oversized {
            return Ok(Some(vec![0; MAX_COLLATION_KEY_BYTES + 1]));
        }
        Ok(Some(vec![match label.to_str().unwrap() {
            "h" => 1,
            "ch" => 2,
            "i" => 3,
            _ => panic!("unexpected navigation label"),
        }]))
    }
}

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn ordered_publications(locale: Option<Arc<Locale>>, expected: &[&str]) {
    let caller = std::thread::current().id();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let fixture = Arc::new(Fixture);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        fixture.clone(),
        sender,
    )
    .with_desktop(fixture);
    if let Some(locale) = &locale {
        engine = engine.with_locale(locale.clone());
    }
    let task = tokio::spawn(engine.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    let result = loop {
        match next(&mut events).await {
            Event::VolumesUpdated(volumes) => break volumes,
            Event::VolumesFailed(error) => panic!("{error}"),
            _ => {}
        }
    };
    assert_eq!(
        result
            .iter()
            .map(|volume| volume.label.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    for volume in &result {
        let original = volumes()
            .into_iter()
            .find(|item| item.id == volume.id)
            .unwrap();
        assert_eq!(volume.root.as_os_str(), original.root.as_os_str());
    }
    commands.send(Command::ListDesktopContext).await.unwrap();
    let result = loop {
        match next(&mut events).await {
            Event::DesktopContext {
                locations,
                network_position,
                ..
            } => {
                assert_eq!(
                    network_position, 0,
                    "Network follows every drive and precedes every special location"
                );
                break locations;
            }
            Event::DesktopContextFailed(error) => panic!("{error}"),
            _ => {}
        }
    };
    assert_eq!(result.len(), 6);
    assert!(
        result[..3]
            .iter()
            .all(|item| item.kind == LocationKind::Volume)
    );
    assert!(
        result[3..]
            .iter()
            .all(|item| item.kind != LocationKind::Volume)
    );
    for section in result.chunks(3) {
        assert_eq!(
            section
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    for item in &result {
        let original = locations()
            .into_iter()
            .find(|original| original.id == item.id)
            .unwrap();
        assert_eq!(item.path.as_os_str(), original.path.as_os_str());
        assert_eq!(item.kind, original.kind);
        assert_eq!(item.drive_letter, original.drive_letter);
    }
    if let Some(locale) = locale {
        let calls = locale.calls.lock().unwrap();
        assert!(calls.len() >= 9);
        assert!(
            calls.iter().all(|thread| *thread != caller),
            "locale calls belong to blocking workers"
        );
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn locale_orders_drives_and_special_sections_without_changing_raw_identity() {
    ordered_publications(Some(Arc::new(Locale::default())), &["h", "ch", "i"]).await;
}

#[tokio::test]
async fn missing_locale_uses_deterministic_raw_label_order() {
    ordered_publications(None, &["ch", "h", "i"]).await;
}

#[tokio::test]
async fn unavailable_keys_use_deterministic_raw_label_order() {
    ordered_publications(
        Some(Arc::new(Locale {
            missing: true,
            ..Default::default()
        })),
        &["ch", "h", "i"],
    )
    .await;
}

async fn rejects_invalid_collation(locale: Locale) {
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(32);
    let fixture = Arc::new(Fixture);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            fixture.clone(),
            sender,
        )
        .with_desktop(fixture)
        .with_locale(Arc::new(locale))
        .run(receiver),
    );
    commands.send(Command::ListVolumes).await.unwrap();
    loop {
        match next(&mut events).await {
            Event::VolumesFailed(error) => {
                assert!(!error.path.as_os_str().is_empty());
                break;
            }
            Event::VolumesUpdated(_) => {
                panic!("failed collation must not publish unordered volumes")
            }
            _ => {}
        }
    }
    commands.send(Command::ListDesktopContext).await.unwrap();
    loop {
        match next(&mut events).await {
            Event::DesktopContextFailed(error) => {
                assert!(!error.path.as_os_str().is_empty());
                break;
            }
            Event::DesktopContext { .. } => {
                panic!("failed collation must not publish unordered locations")
            }
            _ => {}
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn locale_failures_do_not_publish_an_unordered_navigation_inventory() {
    rejects_invalid_collation(Locale {
        fail: true,
        ..Default::default()
    })
    .await;
}

#[tokio::test]
async fn oversized_locale_keys_reject_navigation_publication() {
    rejects_invalid_collation(Locale {
        oversized: true,
        ..Default::default()
    })
    .await;
}
