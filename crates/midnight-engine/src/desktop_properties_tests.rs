use super::*;
use midnight_core::LocalizedDateTime;
use std::io;

fn metadata(unix_mode: Option<u32>, size: Option<u64>) -> EntryMetadata {
    EntryMetadata {
        size,
        unix_mode,
        ..EntryMetadata::default()
    }
}

fn grouped() -> NumberLocale {
    NumberLocale {
        decimal_separator: ".".into(),
        group_separator: ",".into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

fn summarize(
    paths: &[&str],
    entry_metadata: &dyn Fn(&Path) -> Result<EntryMetadata, FsError>,
    details: &dyn Fn(&Path) -> Option<Details>,
) -> Summary {
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    summary(
        &paths,
        &grouped(),
        &Cancellation::default(),
        entry_metadata,
        details,
    )
    .unwrap()
}

fn text(summary: &Summary) -> (String, String) {
    match &summary.result {
        DesktopActionResult::PropertiesInfo { title, body } => (title.clone(), body.clone()),
        _ => panic!("expected the metadata summary"),
    }
}

fn shown(
    paths: &[&str],
    entry_metadata: &dyn Fn(&Path) -> Result<EntryMetadata, FsError>,
) -> (String, String) {
    text(&summarize(paths, entry_metadata, &|_| None))
}

fn time(value: SystemTime) -> String {
    format!("t{}", value.duration_since(UNIX_EPOCH).unwrap().as_secs())
}

#[test]
fn one_folder_shows_its_type_time_and_permissions_and_counts_its_contents_next() {
    let folder = EntryMetadata {
        localized_modified: Some(LocalizedDateTime {
            date: "20.11.2026".into(),
            time: "10:31".into(),
        }),
        ..metadata(Some(0o040_755), None)
    };
    let summary = summarize(&["/data/photos"], &|_| Ok(folder.clone()), &|_| None);
    let (title, body) = text(&summary);
    assert_eq!(title, "/data/photos");
    assert_eq!(
        body,
        "Type: Folder\nModified: 20.11.2026 10:31\nPermissions: rwxr-xr-x (0755)\n\
         Contents: Calculating…\n\nRead-only metadata."
    );
    let folder = summary
        .folder
        .expect("a single folder's contents are counted");
    assert_eq!(folder.path, PathBuf::from("/data/photos"));
    let totals = Totals {
        files: 1_234,
        folders: 1,
        bytes: 5_678_901,
        links: 2,
        unreadable: 1,
        ..Totals::default()
    };
    assert_eq!(
        folder.body(Some(Contents::Counted(&totals))),
        "Type: Folder\nModified: 20.11.2026 10:31\nPermissions: rwxr-xr-x (0755)\n\
         Contents: 1,234 files, 1 folder, 5,678,901 bytes\n\
         Not counted: 2 links (not followed), 1 unreadable item\n\nRead-only metadata."
    );
    assert!(
        folder
            .body(Some(Contents::Calculating(Some(&totals))))
            .contains("Contents: Calculating… 1,234 files, 1 folder so far")
    );
    assert!(
        folder
            .body(Some(Contents::Failed("Permission denied")))
            .contains("Contents: could not be counted (Permission denied)")
    );
}

#[test]
fn native_details_name_owner_and_group_and_show_every_time() {
    let details = PropertyDetails {
        owner: Some(PropertyAccount {
            id: 1000,
            name: Some("ada".into()),
        }),
        group: Some(PropertyAccount {
            id: 4242,
            name: None,
        }),
        created: None,
        modified: UNIX_EPOCH.checked_add(std::time::Duration::from_secs(20)),
        accessed: UNIX_EPOCH.checked_add(std::time::Duration::from_secs(30)),
        unix_mode: Some(0o100_640),
    };
    let summary = summarize(
        &["/data/report.txt"],
        &|_| Ok(metadata(Some(0o100_640), Some(12))),
        &|_| Some(Details::format(details.clone(), &time)),
    );
    assert!(summary.folder.is_none(), "files have no contents to count");
    assert_eq!(
        text(&summary).1,
        "Type: File\nSize: 12 bytes\nOwner: ada (1000)\nGroup: 4242\n\
         Permissions: rw-r----- (0640)\nCreated: not available\nModified: t20\n\
         Accessed: t30\n\nRead-only metadata."
    );
    let created = PropertyDetails {
        created: UNIX_EPOCH.checked_add(std::time::Duration::from_secs(10)),
        ..details
    };
    let summary = summarize(
        &["/data/folder"],
        &|_| Ok(metadata(Some(0o040_750), None)),
        &|_| Some(Details::format(created.clone(), &time)),
    );
    assert!(
        text(&summary)
            .1
            .contains("Created: t10\nModified: t20\nAccessed: t30\nContents: Calculating…")
    );
}

#[test]
fn utc_fallback_text_is_exact_on_both_sides_of_the_epoch() {
    assert_eq!(utc_text(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
    let later = UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
    assert_eq!(utc_text(later), "2023-11-14 22:13:20 UTC");
    let leap = UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(951_782_400))
        .unwrap();
    assert_eq!(utc_text(leap), "2000-02-29 00:00:00 UTC");
    let before = UNIX_EPOCH
        .checked_sub(std::time::Duration::from_millis(500))
        .unwrap();
    assert_eq!(utc_text(before), "1969-12-31 23:59:59 UTC");
}

#[cfg(target_os = "linux")]
#[test]
fn folder_contents_count_files_folders_and_bytes_without_following_links() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("photos 文档");
    std::fs::create_dir_all(folder.join("2026/november")).unwrap();
    std::fs::write(folder.join("a.jpg"), vec![1; 1000]).unwrap();
    std::fs::write(folder.join("2026/b.jpg"), vec![2; 234]).unwrap();
    std::fs::write(folder.join("2026/november/c.jpg"), b"").unwrap();
    // Neither the outside file nor the folder cycle is followed.
    std::fs::write(root.path().join("outside.bin"), vec![3; 99_999]).unwrap();
    symlink("../outside.bin", folder.join("outside link")).unwrap();
    symlink("..", folder.join("2026/november/up")).unwrap();
    let filesystem = midnight_platform::LocalFileSystem::new(None);
    let totals = count_contents(&filesystem, &folder, &Cancellation::default(), &mut |_| {
        true
    })
    .unwrap();
    assert_eq!(
        totals,
        Totals {
            files: 3,
            folders: 2,
            bytes: 1234,
            links: 2,
            ..Totals::default()
        }
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    let error = count_contents(&filesystem, &folder, &cancel, &mut |_| true).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
}

#[test]
fn one_file_shows_its_size_and_special_permission_bits() {
    let (_, body) = shown(&["/bin/tool"], &|_| {
        Ok(metadata(Some(0o104_754), Some(1_234_567)))
    });
    assert_eq!(
        body,
        "Type: File\nSize: 1,234,567 bytes\nPermissions: rwsr-xr-- (4754)\n\nRead-only metadata."
    );
    let (_, body) = shown(&["/srv/drop"], &|_| Ok(metadata(Some(0o103_640), Some(1))));
    assert!(
        body.contains("Size: 1 byte\nPermissions: rw-r-S--T (3640)"),
        "{body}"
    );
}

#[test]
fn a_link_shows_no_permissions_and_other_types_are_named() {
    let (_, body) = shown(&["/data/latest"], &|_| Ok(metadata(Some(0o120_777), None)));
    assert_eq!(body, "Type: Link\n\nRead-only metadata.");
    let (_, body) = shown(&["/run/pipe"], &|_| Ok(metadata(Some(0o010_600), None)));
    assert!(body.starts_with("Type: Pipe\n"), "{body}");
    // Without POSIX modes, a Windows directory attribute or a file length decides.
    let windows_folder = EntryMetadata {
        native_attributes: Some(0x10),
        ..metadata(None, None)
    };
    let (_, body) = shown(&["C:/Data"], &|_| Ok(windows_folder.clone()));
    assert!(body.starts_with("Type: Folder\n"), "{body}");
    let (_, body) = shown(&["C:/Data/a.txt"], &|_| Ok(metadata(None, Some(2))));
    assert_eq!(body, "Type: File\nSize: 2 bytes\n\nRead-only metadata.");
}

#[test]
fn a_selection_counts_each_kind_and_sums_only_file_lengths() {
    let (title, body) = shown(
        &["/d/a.txt", "/d/b.txt", "/d/folder", "/d/missing", "/d/pipe"],
        &|path| match path.file_name().and_then(|name| name.to_str()) {
            Some("a.txt") => Ok(metadata(Some(0o100_644), Some(1_000))),
            Some("b.txt") => Ok(metadata(Some(0o100_600), Some(234))),
            Some("folder") => Ok(metadata(Some(0o040_700), None)),
            Some("pipe") => Ok(metadata(Some(0o010_644), None)),
            _ => Err(FsError::from_io(
                "read metadata",
                path,
                io::ErrorKind::NotFound.into(),
            )),
        },
    );
    assert_eq!(title, "5 selected items");
    assert_eq!(
        body,
        "2 files, 1 folder, 1 other item\nSize: 1,234 bytes in 2 files\n\
         1 item could not be read\n\nRead-only metadata. Folder contents are not included."
    );
    let (_, body) = shown(&["/d/one", "/d/two"], &|_| {
        Ok(metadata(Some(0o040_755), None))
    });
    assert_eq!(
        body,
        "2 folders\n\nRead-only metadata. Folder contents are not included."
    );
}

#[test]
fn one_unreadable_entry_is_an_error_and_cancellation_reads_nothing() {
    let paths = [PathBuf::from("/d/gone")];
    let missing = |path: &Path| -> Result<EntryMetadata, FsError> {
        Err(FsError::from_io(
            "read metadata",
            path,
            io::ErrorKind::NotFound.into(),
        ))
    };
    let Err(error) = summary(
        &paths,
        &grouped(),
        &Cancellation::default(),
        &missing,
        &|_| None,
    ) else {
        panic!("an unreadable single entry is an error");
    };
    assert_eq!(error.kind, io::ErrorKind::NotFound);
    let cancel = Cancellation::default();
    cancel.cancel();
    let selection = [PathBuf::from("/d/a"), PathBuf::from("/d/b")];
    assert!(
        summary(
            &selection,
            &grouped(),
            &cancel,
            &|_| panic!("a cancelled request must not read metadata"),
            &|_| panic!("a cancelled request must not read details"),
        )
        .is_err()
    );
}
