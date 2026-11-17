//! Reparse classification through the public services, without OneDrive. A third-party
//! non-surrogate tag stands in for Cloud Files placeholders (no filter owns it, so only
//! handle-relative, no-follow access works), and a native junction for real links.
use crate::{
    WindowsFileSystem,
    test_fixtures::{
        FILTER_DIRECTORY, FILTER_FILE, build_temp, clear_filter_tag, create_junction,
        set_filter_tag, unusual_name,
    },
};
use midnight_core::{
    Cancellation, DeleteMode, EntryKind, FindChild, FindService, FindSkipReason, FindSource,
    MutationService, TransferService,
};
use std::{fs, path::Path};

/// ERROR_CANT_ACCESS_FILE: a filter-mediated open of a tag that no filter owns.
const CANT_ACCESS_FILE: i32 = 1920;

fn children(source: &mut dyn FindSource) -> Vec<FindChild> {
    let cancel = Cancellation::default();
    let mut children = Vec::new();
    while let Some(child) = source.next_child(&cancel).unwrap() {
        children.push(child);
    }
    children
}

fn find_kind(children: &[FindChild], path: &Path) -> Option<Result<EntryKind, FindSkipReason>> {
    children.iter().find_map(|child| match child {
        FindChild::Entry(source) if source.entry().path == path => Some(Ok(source.entry().kind)),
        FindChild::Skipped {
            path: skipped,
            reason,
        } if skipped == path => Some(Err(*reason)),
        _ => None,
    })
}

#[test]
fn non_surrogate_reparse_entries_are_ordinary_files_and_directories() {
    let temp = build_temp("reparse-filter-");
    let cancel = Cancellation::default();
    let service = WindowsFileSystem::new(None);
    let directory = temp.path().join(unusual_name("placeholder"));
    let file = temp.path().join("placeholder.txt");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child.txt"), b"child").unwrap();
    fs::create_dir(directory.join("sub")).unwrap();
    fs::write(directory.join("sub").join("leaf.txt"), b"leaf").unwrap();
    fs::write(&file, b"raw stub bytes").unwrap();
    set_filter_tag(&directory, FILTER_DIRECTORY).unwrap();
    set_filter_tag(&file, FILTER_FILE).unwrap();

    // Classification: ordinary entries, not links.
    for (path, kind) in [(&directory, EntryKind::Directory), (&file, EntryKind::File)] {
        assert_eq!(service.prepare(path, &cancel).unwrap().item().kind, kind);
    }

    // Find enters the directory and returns the file instead of skipping "links".
    let mut root = service.open_find_source(temp.path(), &cancel).unwrap();
    let listed = children(root.as_mut());
    assert_eq!(
        find_kind(&listed, &directory),
        Some(Ok(EntryKind::Directory))
    );
    assert_eq!(find_kind(&listed, &file), Some(Ok(EntryKind::File)));
    let mut placeholder = service.open_find_source(&directory, &cancel).unwrap();
    let nested = children(placeholder.as_mut());
    assert_eq!(
        find_kind(&nested, &directory.join("child.txt")),
        Some(Ok(EntryKind::File))
    );
    assert_eq!(
        find_kind(&nested, &directory.join("sub")),
        Some(Ok(EntryKind::Directory))
    );
    // Content is requested through the filter stack, never read raw from the stub: with
    // no filter owning the fixture tag, that open fails instead of returning stub bytes.
    let filtered = format!("(os error {CANT_ACCESS_FILE})");
    let mut content = service.open_find_source(&file, &cancel).unwrap();
    let error = content.read_chunk(&mut [0; 64], &cancel).unwrap_err();
    assert!(error.message.contains(&filtered), "{error:?}");

    // Copy classifies the file as ordinary and opens its content through the filter.
    let error = service
        .prepare_transfer_source(&file, &cancel)
        .err()
        .expect("no filter owns the fixture tag");
    assert!(error.message.contains(&filtered), "{error:?}");

    // Metadata of a filter-backed file is editable like any ordinary file.
    let prepared = service.prepare(&file, &cancel).unwrap();
    let snapshot = prepared.metadata_snapshot(&cancel).unwrap();
    assert_ne!(snapshot.capabilities.windows_attributes, 0);
    assert!(snapshot.capabilities.modified);

    // F7 creates through the filter-backed directory handle-relatively.
    let base = service.prepare_create(temp.path(), &cancel).unwrap();
    let mut created = directory.clone();
    created.push("new");
    created.push("deeper");
    let relative = created.strip_prefix(temp.path()).unwrap();
    base.create(relative, &cancel).unwrap();

    // Discovery and permanent deletion recurse into the directory.
    let prepared = service.prepare(&directory, &cancel).unwrap();
    let mut discovered = Vec::new();
    prepared
        .discover(&cancel, &mut |item| {
            discovered.push((item.path.clone(), item.kind))
        })
        .unwrap();
    assert!(discovered.contains(&(directory.join("sub").join("leaf.txt"), EntryKind::File)));
    assert!(discovered.contains(&(directory.join("new").join("deeper"), EntryKind::Directory)));
    // The directory, child.txt, sub, sub\leaf.txt, new and new\deeper.
    assert_eq!(discovered.len(), 6, "{discovered:?}");
    let mut removed = 0;
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| removed += 1)
        .unwrap();
    assert_eq!(removed, 6);
    assert!(!directory.exists());
    clear_filter_tag(&file, FILTER_FILE).unwrap();
}

/// Read-only: enumeration and metadata only, so no placeholder hydrates or changes.
/// Cloud Files disguises placeholders (no reparse attribute) for most third-party
/// processes by default; Windows inbox processes and threads running Shell code that
/// opts in see them exposed with their cloud tag. Both views must classify as ordinary.
#[test]
#[ignore = "opt-in qualification against the host's OneDrive sync root (%OneDrive%)"]
fn onedrive_placeholders_are_ordinary_entries_on_this_host() {
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            RtlQueryProcessPlaceholderCompatibilityMode, RtlSetThreadPlaceholderCompatibilityMode,
        },
        Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
    };
    const EXPOSE_PLACEHOLDERS: i8 = 2;
    let root = std::path::PathBuf::from(
        std::env::var_os("OneDrive").expect("OneDrive sync root (%OneDrive%) is required"),
    );
    let cancel = Cancellation::default();
    let service = WindowsFileSystem::new(None);
    // SAFETY: a pure query of this process's placeholder compatibility mode.
    let process_mode = unsafe { RtlQueryProcessPlaceholderCompatibilityMode() };
    let survey = |label: &str| {
        assert_eq!(
            service.prepare(&root, &cancel).unwrap().item().kind,
            EntryKind::Directory
        );
        let mut source = service.open_find_source(&root, &cancel).unwrap();
        let root_attributes = source
            .entry()
            .metadata
            .native_attributes
            .unwrap_or_default();
        let (mut entries, mut placeholders, mut links) = (0, 0, 0);
        for child in children(source.as_mut()).into_iter().take(500) {
            match child {
                FindChild::Entry(child) => {
                    entries += 1;
                    if child.entry().metadata.native_attributes.unwrap_or_default()
                        & FILE_ATTRIBUTE_REPARSE_POINT
                        != 0
                    {
                        placeholders += 1;
                    }
                }
                FindChild::Skipped {
                    reason: FindSkipReason::Link,
                    ..
                } => links += 1,
                _ => {}
            }
        }
        eprintln!(
            "{label}: process mode {process_mode}, root reparse attribute {}, \
             {entries} ordinary entries ({placeholders} exposed placeholders), {links} links",
            root_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        );
        (
            root_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
            entries,
            placeholders,
        )
    };
    let (_, disguised_entries, _) = survey("default");
    assert!(disguised_entries > 0);
    // SAFETY: changes only this test thread's mode; the previous mode is restored below.
    let previous = unsafe { RtlSetThreadPlaceholderCompatibilityMode(EXPOSE_PLACEHOLDERS) };
    let exposed = survey("exposed");
    // SAFETY: restores the mode returned by the matching call above.
    unsafe { RtlSetThreadPlaceholderCompatibilityMode(previous) };
    assert!(
        exposed.0,
        "an exposed sync root carries its cloud reparse tag"
    );
    assert!(
        exposed.2 > 0,
        "exposed placeholders are ordinary Find entries"
    );
    assert_eq!(exposed.1, disguised_entries);
}

#[test]
fn junctions_stay_unfollowed_links_for_every_operation() {
    let temp = build_temp("reparse-junction-");
    let cancel = Cancellation::default();
    let service = WindowsFileSystem::new(None);
    let target = temp.path().join("target");
    let link = temp.path().join(unusual_name("junction"));
    fs::create_dir(&target).unwrap();
    fs::write(target.join("inner.txt"), b"keep target intact").unwrap();
    fs::create_dir(&link).unwrap();
    create_junction(&link, &target).unwrap();
    assert_eq!(
        fs::read(link.join("inner.txt")).unwrap(),
        b"keep target intact"
    );

    let prepared = service.prepare(&link, &cancel).unwrap();
    assert_eq!(prepared.item().kind, EntryKind::DirectoryLink);
    let mut discovered = Vec::new();
    prepared
        .discover(&cancel, &mut |item| discovered.push(item.kind))
        .unwrap();
    assert_eq!(discovered, [EntryKind::DirectoryLink], "never recursed");

    let mut root = service.open_find_source(temp.path(), &cancel).unwrap();
    let listed = children(root.as_mut());
    assert_eq!(find_kind(&listed, &link), Some(Err(FindSkipReason::Link)));
    assert!(service.open_find_source(&link, &cancel).is_err());
    let refused = service
        .prepare_transfer_source(&link, &cancel)
        .err()
        .unwrap();
    assert!(refused.message.contains("links"), "{refused:?}");
    let base = service.prepare_create(temp.path(), &cancel).unwrap();
    let through = link
        .join("new")
        .strip_prefix(temp.path())
        .unwrap()
        .to_owned();
    assert!(base.create(&through, &cancel).is_err());
    assert!(!target.join("new").exists());

    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
        .unwrap();
    assert!(!link.exists());
    assert_eq!(
        fs::read(target.join("inner.txt")).unwrap(),
        b"keep target intact"
    );
}
