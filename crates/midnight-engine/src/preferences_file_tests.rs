use super::*;
use std::fs;
use tokio::sync::mpsc;

fn fixture() -> tempfile::TempDir {
    // Native publication capabilities belong to the runner's selected filesystem.
    let root = std::env::temp_dir().join("midnight-settings-file/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("settings-file-")
        .tempdir_in(root.canonicalize().unwrap())
        .unwrap()
}

fn preferences(root: &Path) -> UserPreferences {
    UserPreferences {
        dark_theme: PreferenceTheme::Nord,
        density: UiDensity::Touch,
        font_family: "Noto Sans 文档 e\u{301}".into(),
        bookmarks: vec![Bookmark {
            id: BookmarkId(4),
            name: "Projekty 🧪".into(),
            path: root.join("projekty 文档"),
            archive: None,
            network: None,
            slot: Some(2),
        }],
        tabs: [
            PaneTabPreferences {
                paths: vec![root.join("levý ž"), root.join("文档")],
                active: 1,
                ..Default::default()
            },
            PaneTabPreferences::default(),
        ],
        ..UserPreferences::default()
    }
}

fn encoded(preferences: &UserPreferences) -> Arc<[u8]> {
    encode_settings_file(preferences.clone()).unwrap().into()
}

#[test]
fn export_and_import_roundtrip_unicode_settings_through_native_files() {
    let root = fixture();
    let service = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let destination = root.path().join("nastavení 文档 🧪.json");
    let expected = preferences(root.path());
    let bytes = encoded(&expected);
    let receipt = publish(&destination, bytes.clone(), false, &service, &cancel).unwrap();
    assert_eq!(receipt.path, destination);
    assert_eq!(receipt.bytes, bytes.len() as u64);
    assert!(receipt.recovery.is_empty());
    assert_eq!(fs::read(&destination).unwrap(), bytes.as_ref());
    assert_eq!(
        import_file(&destination, &service, &cancel).unwrap(),
        ImportedPreferences {
            preferences: expected,
            skipped_tabs: 0,
            removed_bindings: 0,
        }
    );
    // Only the published file remains: no staging residue beside it.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn existing_destination_needs_consent_and_a_replacement_keeps_the_old_file() {
    let root = fixture();
    let service = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let destination = root.path().join("settings.json");
    fs::write(&destination, b"keep me").unwrap();
    let bytes = encoded(&preferences(root.path()));
    let refused = publish(&destination, bytes.clone(), false, &service, &cancel).unwrap_err();
    assert_eq!(refused.kind, io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&destination).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    let receipt = publish(&destination, bytes.clone(), true, &service, &cancel).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes.as_ref());
    for recovery in &receipt.recovery {
        assert_eq!(fs::read(recovery).unwrap(), b"keep me");
    }
    fs::create_dir(root.path().join("folder.json")).unwrap();
    let folder = publish(
        &root.path().join("folder.json"),
        bytes,
        true,
        &service,
        &cancel,
    );
    assert!(folder.is_err());
    assert!(root.path().join("folder.json").is_dir());
}

#[test]
fn import_refuses_missing_folder_invalid_and_oversized_files() {
    let root = fixture();
    let service = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    assert!(import_file(&root.path().join("missing.json"), &service, &cancel).is_err());
    fs::create_dir(root.path().join("folder.json")).unwrap();
    assert!(import_file(&root.path().join("folder.json"), &service, &cancel).is_err());
    let invalid = root.path().join("invalid.json");
    fs::write(
        &invalid,
        br#"{"version": 34, "theme": "plaid", "density": "desktop"}"#,
    )
    .unwrap();
    let error = import_file(&invalid, &service, &cancel).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData);
    assert!(
        error.message.contains("The settings are invalid"),
        "{error}"
    );
    let oversized = root.path().join("oversized.json");
    let mut bytes = encoded(&UserPreferences::default()).to_vec();
    bytes.resize(MAX_PREFERENCES_BYTES + 1, b' ');
    fs::write(&oversized, &bytes).unwrap();
    let error = import_file(&oversized, &service, &cancel).unwrap_err();
    assert!(error.message.contains("4 MiB settings limit"), "{error}");
    for relative in ["settings.json", "."] {
        assert_eq!(
            import_file(Path::new(relative), &service, &cancel)
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput
        );
    }
    // Append raw text: pushing `..` onto a Windows verbatim path resolves it lexically.
    let mut parent_step = root.path().join("folder.json").into_os_string();
    for part in ["..", "x.json"] {
        parent_step.push(std::path::MAIN_SEPARATOR_STR);
        parent_step.push(part);
    }
    let parent_step = PathBuf::from(parent_step);
    let refused = publish(
        &parent_step,
        encoded(&UserPreferences::default()),
        false,
        &service,
        &cancel,
    );
    assert_eq!(refused.unwrap_err().kind, io::ErrorKind::InvalidInput);
    assert!(!root.path().join("x.json").exists());
}

fn controller(service: Option<Arc<dyn TransferService>>) -> (Controller, mpsc::Receiver<Event>) {
    let (sender, receiver) = mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let scope = TaskLifetime::testing(service.clone());
    (
        Controller::new(
            scope,
            service,
            crate::transfer_leases::namespace_gate(),
            outbox,
        ),
        receiver,
    )
}

async fn next(receiver: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_reports_exports_and_validated_imports_by_request() {
    let root = fixture();
    let service: Arc<dyn TransferService> = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let (files, mut events) = controller(Some(service));
    let mut tasks = JoinSet::new();
    let destination = root.path().join("export 文档.json");
    let expected = preferences(root.path());
    files
        .export(
            Box::new(PreferencesExportRequest {
                request: DesktopRequestId(7),
                destination: destination.clone(),
                preferences: expected.clone(),
                overwrite: false,
            }),
            &mut tasks,
        )
        .await;
    let Event::PreferencesExported { request, result } = next(&mut events).await else {
        panic!("expected an export result");
    };
    assert_eq!(request, DesktopRequestId(7));
    assert_eq!(result.unwrap().path, destination);
    // Admission is released only after the reply has been queued.
    tasks.join_next().await.unwrap().unwrap();
    files.import(DesktopRequestId(8), destination.clone(), &mut tasks);
    let Event::PreferencesImported { request, result } = next(&mut events).await else {
        panic!("expected an import result");
    };
    assert_eq!(request, DesktopRequestId(8));
    assert_eq!(result.unwrap().preferences, expected);
    tasks.join_next().await.unwrap().unwrap();
    files.import(DesktopRequestId(9), "relative.json".into(), &mut tasks);
    let Event::PreferencesImported { request, result } = next(&mut events).await else {
        panic!("expected an import refusal");
    };
    assert_eq!(request, DesktopRequestId(9));
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::InvalidInput);
    while tasks.join_next().await.is_some() {}
    assert_eq!(files.budget.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_without_transfer_service_refuses_both_directions() {
    let (files, mut events) = controller(None);
    let mut tasks = JoinSet::new();
    let destination = std::env::temp_dir().join("never-written-settings.json");
    files
        .export(
            Box::new(PreferencesExportRequest {
                request: DesktopRequestId(1),
                destination: destination.clone(),
                preferences: UserPreferences::default(),
                overwrite: false,
            }),
            &mut tasks,
        )
        .await;
    let Event::PreferencesExported { result, .. } = next(&mut events).await else {
        panic!("expected an export refusal");
    };
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Unsupported);
    files.import(DesktopRequestId(2), destination, &mut tasks);
    let Event::PreferencesImported { result, .. } = next(&mut events).await else {
        panic!("expected an import refusal");
    };
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Unsupported);
}
