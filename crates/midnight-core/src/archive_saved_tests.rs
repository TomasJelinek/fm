//! A folder four archives deep keeps every level in its saved address, which reads as the
//! path bar shows it and opens nothing by its display text.
use super::*;
use crate::{ArchiveMount, ArchiveNode, ArchiveParent, Cancellation, FsError, VirtualEntryId};
use std::sync::Arc;

#[derive(Debug)]
struct Named(u64, Option<ArchiveParent>);
impl ArchiveMount for Named {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        unreachable!()
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        unreachable!()
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
    fn parent(&self) -> Option<ArchiveParent> {
        self.1.clone()
    }
}

fn container() -> std::path::PathBuf {
    std::env::temp_dir().join("data").join("a.zip")
}

/// The folder `folders` of mount `id`, whose root is named `root`.
fn location(
    id: u64,
    parent: Option<ArchiveParent>,
    format: Option<ArchiveFormat>,
    root: &str,
    folders: &[&str],
) -> ArchiveLocation {
    let mut trail = vec![(1, root.to_owned())];
    trail.extend(
        folders
            .iter()
            .enumerate()
            .map(|(index, name)| (index as u64 + 2, (*name).to_owned())),
    );
    ArchiveLocation {
        mount: Some(Arc::new(Named(id, parent))),
        address: ArchiveAddress {
            provider: "midnight.fs.archive".into(),
            format,
            container: container(),
            source_version: vec![7],
            components: folders
                .iter()
                .map(|name| name.as_bytes().to_vec())
                .collect(),
            nested: Vec::new(),
        },
        directory: trail.len() as u64,
        trail,
    }
}

/// Open member `name` of `folder` as archive `id` and show its folders `folders`.
fn open(
    folder: ArchiveLocation,
    id: u64,
    name: &str,
    format: ArchiveFormat,
    folders: &[&str],
) -> ArchiveLocation {
    let parent = ArchiveParent {
        member: VirtualEntryId {
            mount: folder.mount_id().unwrap(),
            entry: 40 + id,
        },
        location: folder,
        name: name.into(),
        raw_name: name.as_bytes().to_vec(),
    };
    location(id, Some(parent), Some(format), name, folders)
}

/// `a.zip\inner\b.7z\deeper\c.tar.gz\folder\d.zip\last`: the deepest folder that opens.
fn deepest() -> ArchiveLocation {
    let outer = location(1, None, Some(ArchiveFormat::Zip), "a.zip", &["inner"]);
    let second = open(outer, 2, "b.7z", ArchiveFormat::SevenZip, &["deeper"]);
    let third = open(second, 3, "c.tar.gz", ArchiveFormat::TarGzip, &["folder"]);
    open(third, 4, "d.zip", ArchiveFormat::Zip, &["last"])
}

#[test]
fn the_deepest_folder_keeps_every_archive_and_folder_in_its_saved_address() {
    let shown = deepest();
    assert_eq!(shown.depth(), crate::MAX_ARCHIVE_DEPTH);
    let saved = shown.saved_address();
    saved.validate().unwrap();
    assert_eq!(saved.container, container());
    assert_eq!(saved.source_version, [7]);
    assert_eq!(saved.components, [b"inner".to_vec()]);
    assert_eq!(
        saved.nested,
        [
            NestedArchiveAddress {
                member: b"b.7z".to_vec(),
                format: ArchiveFormat::SevenZip,
                components: vec![b"deeper".to_vec()],
            },
            NestedArchiveAddress {
                member: b"c.tar.gz".to_vec(),
                format: ArchiveFormat::TarGzip,
                components: vec![b"folder".to_vec()],
            },
            NestedArchiveAddress {
                member: b"d.zip".to_vec(),
                format: ArchiveFormat::Zip,
                components: vec![b"last".to_vec()],
            },
        ]
    );
    // Without its sessions it reads exactly as the shown location does.
    let unavailable = ArchiveLocation::unavailable(saved.clone());
    assert_eq!(unavailable.display_text(), shown.display_text());
    let separator = std::path::MAIN_SEPARATOR;
    assert!(shown.display_text().ends_with(&format!(
        "a.zip{separator}inner{separator}b.7z{separator}deeper{separator}c.tar.gz\
         {separator}folder{separator}d.zip{separator}last"
    )));
    assert_eq!(unavailable.trail.len(), saved.step_count() + 1);
    // A saved address is saved already.
    assert_eq!(unavailable.saved_address(), saved);
}

#[test]
fn steps_truncate_to_enclosing_folders_and_archive_roots() {
    let saved = deepest().saved_address();
    assert_eq!(saved.step_count(), 7);
    let mut root_of_c = saved.clone();
    root_of_c.truncate_steps(4);
    assert_eq!(
        root_of_c.steps(),
        [
            ArchiveStep::Folder(b"inner"),
            ArchiveStep::Archive(b"b.7z", ArchiveFormat::SevenZip),
            ArchiveStep::Folder(b"deeper"),
            ArchiveStep::Archive(b"c.tar.gz", ArchiveFormat::TarGzip),
        ]
    );
    let mut outer_root = saved.clone();
    outer_root.truncate_steps(0);
    assert!(outer_root.components.is_empty() && outer_root.nested.is_empty());
    let mut all = saved.clone();
    all.truncate_steps(99);
    assert_eq!(all, saved);
}

#[test]
fn a_member_named_like_a_folder_is_not_that_folder() {
    let saved = deepest().saved_address();
    // The same names as plain folders of the outer archive name another location.
    let mut folders = saved.clone();
    folders.components = saved
        .steps()
        .into_iter()
        .map(|step| {
            let (ArchiveStep::Folder(name) | ArchiveStep::Archive(name, _)) = step;
            name.to_vec()
        })
        .collect();
    folders.nested.clear();
    assert_eq!(
        ArchiveLocation::unavailable(folders.clone()).display_text(),
        ArchiveLocation::unavailable(saved.clone()).display_text(),
        "the display text alone cannot tell them apart"
    );
    assert!(!saved.same_location(&folders));
    // A newer revision of the outer archive is the same location.
    let mut newer = saved.clone();
    newer.source_version = vec![8];
    assert!(saved.same_location(&newer));
    // Another decoder for a member is another location.
    let mut other = saved.clone();
    other.nested[1].format = ArchiveFormat::Tar;
    assert!(!saved.same_location(&other));
}

#[test]
fn unsaveable_levels_keep_the_folder_that_holds_them() {
    let outer = location(1, None, Some(ArchiveFormat::Zip), "a.zip", &["inner"]);
    let mut inner = open(outer, 2, "b.7z", ArchiveFormat::SevenZip, &["deeper"]);
    // An enclosing archive that did not say its member's raw name.
    let mut parent = inner.parent().unwrap();
    parent.raw_name.clear();
    inner.mount = Some(Arc::new(Named(2, Some(parent))));
    let saved = inner.saved_address();
    assert!(saved.nested.is_empty());
    assert_eq!(saved.components, [b"inner".to_vec()]);
}

#[test]
fn saved_addresses_refuse_more_levels_than_open_and_bad_member_names() {
    let mut saved = deepest().saved_address();
    let mut deeper = saved.clone();
    deeper.nested.push(deeper.nested[0].clone());
    assert!(deeper.validate().is_err());
    saved.nested[0].member = b"..".to_vec();
    assert!(saved.validate().is_err());
    saved.nested[0].member = b"x/y.zip".to_vec();
    assert!(saved.validate().is_err());
}
