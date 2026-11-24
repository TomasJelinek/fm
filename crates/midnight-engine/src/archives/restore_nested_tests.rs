//! A saved location inside archives inside an archive (a Hot Path, a saved tab or a
//! history entry) opens every level again from raw member names; a level that does not
//! open is named in the error, counted from the archive opened from the native file.
use super::*;

/// The archive opened from the native file: `inner` (2) holds `b.7z` (3) and a folder
/// named like the member, `b.7z.d` (4).
#[derive(Debug)]
struct Outer;
impl ArchiveMount for Outer {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        Ok(())
    }
    fn id(&self) -> u64 {
        70
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let node = |entry, name: &str, kind| ArchiveNode {
            id: VirtualEntryId { mount: 70, entry },
            parent: directory,
            raw_name: Arc::from(name.as_bytes()),
            display_name: name.into(),
            kind,
            size: Some(1),
            readable: true,
            modified: None,
            access: Default::default(),
        };
        Ok(match directory {
            1 => vec![node(2, "inner", EntryKind::Directory)],
            2 => vec![
                node(3, "b.7z", EntryKind::File),
                node(4, "b.7z.d", EntryKind::Directory),
            ],
            _ => Vec::new(),
        })
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("restoring reads no member")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

fn level(member: &str, format: ArchiveFormat, folders: &[&str]) -> NestedArchiveAddress {
    NestedArchiveAddress {
        member: member.as_bytes().to_vec(),
        format,
        components: folders
            .iter()
            .map(|name| name.as_bytes().to_vec())
            .collect(),
    }
}

/// `a.zip\inner\<member>\deeper\c.tar.gz\folder\d.zip\last`: four archives deep.
fn saved(member: &str) -> ArchiveAddress {
    ArchiveAddress {
        provider: "test.native".into(),
        format: Some(ArchiveFormat::Zip),
        container: std::env::temp_dir().join("a.zip"),
        source_version: vec![1],
        components: vec![b"inner".to_vec()],
        nested: vec![
            level(member, ArchiveFormat::SevenZip, &["deeper"]),
            level("c.tar.gz", ArchiveFormat::TarGzip, &["folder"]),
            level("d.zip", ArchiveFormat::Zip, &["last"]),
        ],
    }
}

async fn restored(address: ArchiveAddress, opens: bool) -> Result<ArchiveListing, FsError> {
    let (sender, mut events) = tokio::sync::mpsc::channel(8);
    let mut controller = Controller::testing(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        crate::test_snapshots(Default::default()),
        Arc::new(Semaphore::new(MAX_WORKERS)),
    );
    controller.opener = native::Opener::Test(Arc::new(move |_, path| {
        if opens {
            Ok(Arc::new(Outer) as Arc<dyn ArchiveMount>)
        } else {
            Err(FsError::from_io(
                "open archive",
                path,
                std::io::Error::new(std::io::ErrorKind::NotFound, "The file is missing"),
            ))
        }
    }));
    let mut tasks = JoinSet::new();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(2),
            ArchiveAction::List(ArchiveLocation::unavailable(address)),
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let result = loop {
        if let Some(Event::ArchiveListed { result, .. }) = events.recv().await {
            break result;
        }
    };
    tasks.join_next().await.unwrap().unwrap();
    result
}

#[tokio::test]
async fn each_level_of_a_saved_path_is_found_by_its_raw_member_name() {
    // The outermost archive and its folder open; the archive inside it is found by its
    // member name (not the folder named like it) and its decoder is chosen next, which
    // this test catalog does not have: level 2 of 4 is named.
    let error = restored(saved("b.7z"), true).await.unwrap_err();
    assert!(
        error.message.starts_with(
            "Archive 2 of 4 on the saved path (b.7z) cannot be opened: \
             Compatible archive filesystem plugin is unavailable"
        ),
        "{error}"
    );
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        error
            .path
            .to_string_lossy()
            .ends_with(&format!("a.zip{separator}inner{separator}b.7z")),
        "{error}"
    );
}

#[tokio::test]
async fn a_member_that_is_gone_names_its_level() {
    let error = restored(saved("gone.7z"), true).await.unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::NotFound);
    assert!(
        error.message.starts_with(
            "Archive 2 of 4 on the saved path (gone.7z) cannot be opened: \
             The archive is no longer in the folder that held it"
        ),
        "{error}"
    );
    // A folder named like a member is not that member.
    let error = restored(saved("b.7z.d"), true).await.unwrap_err();
    assert!(error.message.starts_with("Archive 2 of 4"), "{error}");
}

#[tokio::test]
async fn a_missing_outermost_archive_names_the_first_level() {
    let error = restored(saved("b.7z"), false).await.unwrap_err();
    assert!(
        error
            .message
            .starts_with("Archive 1 of 4 on the saved path (a.zip) cannot be opened: "),
        "{error}"
    );
    // A saved location in one archive keeps its error as it was.
    let mut single = saved("b.7z");
    single.nested.clear();
    let error = restored(single, false).await.unwrap_err();
    assert_eq!(error.message, "The file is missing");
}
