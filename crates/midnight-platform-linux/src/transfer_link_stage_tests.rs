//! Copy of links as links: exact text, private staging, no-replace and recoverable
//! replacement, and refusal of a link that changed after preparation.
use super::*;
use crate::LinuxFileSystem;
use midnight_core::{CopyLinkPolicy, TransferLinkRepresentation};
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/linux-copy-links");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root.canonicalize().unwrap()).unwrap()
}

fn private_names(directory: &Path) -> Vec<OsString> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.as_bytes().starts_with(b".midnight-"))
        .collect()
}

/// Copy `link` into `destination` as a link, as the engine's link worker does.
fn copy_link(link: &Path, destination: &Path) -> Result<TransferPublication, FsError> {
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let source =
        filesystem.prepare_transfer_source_with_links(link, CopyLinkPolicy::Preserve, &cancel)?;
    assert_eq!(source.entry().item.kind, EntryKind::Symlink);
    let representation = source.link_representation(&cancel)?;
    let directory = filesystem.prepare_transfer_destination(destination, &cancel)?;
    directory.admit_link(&representation, &cancel)?;
    let mut stage = directory
        .stage_link(&representation, &cancel)
        .map_err(|failure| failure.error)?;
    stage.sync(&cancel)?;
    assert_eq!(stage.link_representation(&cancel)?, representation);
    let identity = stage.revalidate_stage(&representation, &cancel)?;
    let name = link.file_name().unwrap();
    let publication = stage.publish_no_replace(name, &cancel);
    if publication.is_ok() {
        assert_eq!(
            stage.revalidate_publication(&representation, &cancel)?,
            identity
        );
    }
    stage.cleanup().map_err(|failure| failure.error)?;
    publication
}

#[test]
fn every_kind_of_link_is_recreated_with_its_exact_text_and_no_staging_residue() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::create_dir(fixture.path().join("folder")).unwrap();
    fs::write(fixture.path().join("file.txt"), b"content").unwrap();
    let raw = OsString::from_vec(b"../missing-\xff/x".to_vec());
    let cases: [(&str, OsString); 5] = [
        ("to-folder", "../folder".into()),
        ("to-file", "../file.txt".into()),
        ("dangling", raw),
        ("to-self", ".".into()),
        ("loop", "loop".into()),
    ];
    for (name, target) in &cases {
        symlink(target, source.join(name)).unwrap();
        let publication = copy_link(&source.join(name), &destination).unwrap();
        assert!(
            matches!(&publication, TransferPublication::Published(path) if *path == destination.join(name)),
            "{name}: {publication:?}"
        );
        let copied = destination.join(name);
        assert!(
            fs::symlink_metadata(&copied)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&copied).unwrap().as_os_str(),
            target,
            "{name}"
        );
    }
    assert!(private_names(&destination).is_empty());
    // The targets were never touched.
    assert_eq!(
        fs::read(fixture.path().join("file.txt")).unwrap(),
        b"content"
    );
    assert_eq!(
        fs::read_dir(fixture.path().join("folder")).unwrap().count(),
        0
    );
}

#[test]
fn an_existing_name_is_never_replaced_without_consent() {
    let fixture = fixture();
    let link = fixture.path().join("link");
    let destination = fixture.path().join("destination");
    fs::create_dir(&destination).unwrap();
    symlink("target", &link).unwrap();
    fs::write(destination.join("link"), b"existing").unwrap();
    let error = copy_link(&link, &destination).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::AlreadyExists, "{error:?}");
    assert_eq!(fs::read(destination.join("link")).unwrap(), b"existing");
    assert!(
        private_names(&destination).is_empty(),
        "cleanup removed the stage"
    );
}

#[test]
fn a_consented_replacement_keeps_the_displaced_file_in_private_recovery() {
    let fixture = fixture();
    let link = fixture.path().join("link");
    let destination = fixture.path().join("destination");
    fs::create_dir(&destination).unwrap();
    symlink("../elsewhere", &link).unwrap();
    fs::write(destination.join("link"), b"displaced").unwrap();
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let source = filesystem
        .prepare_transfer_source_with_links(&link, CopyLinkPolicy::Preserve, &cancel)
        .unwrap();
    let representation = source.link_representation(&cancel).unwrap();
    let directory = filesystem
        .prepare_transfer_destination(&destination, &cancel)
        .unwrap();
    if !directory.capabilities().link_replacement {
        eprintln!("SKIP link replacement: the fixture filesystem is not ext");
        return;
    }
    let target = directory
        .prepare_target(OsStr::new("link"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = directory.stage_link(&representation, &cancel).unwrap();
    stage.sync(&cancel).unwrap();
    stage.revalidate_stage(&representation, &cancel).unwrap();
    stage
        .prepare_replacement(
            OsStr::new("link"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    let publication = stage.publish_replace(OsStr::new("link"), &cancel).unwrap();
    let TransferPublication::Replaced { path, recovery } = publication else {
        panic!("{publication:?}");
    };
    assert_eq!(path, destination.join("link"));
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("../elsewhere"));
    assert!(recovery.path.ends_with("item"));
    assert!(
        recovery
            .path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name.as_bytes().starts_with(b".midnight-recovery-"))
    );
    assert_eq!(fs::read(&recovery.path).unwrap(), b"displaced");
    stage.cleanup().unwrap();
    let names = private_names(&destination);
    assert_eq!(
        names.len(),
        1,
        "only the recovery folder remains: {names:?}"
    );
}

/// A job's tracking of the names it changes, as the engine keeps it while no other job
/// claims them: every reservation is accepted, and its binding kept in order.
#[derive(Default)]
struct Tracking {
    bindings: std::sync::Mutex<Vec<Arc<dyn midnight_core::TransferNamespacePin>>>,
}

struct Tracked(Arc<Tracking>);
struct TrackedTurn(Arc<Tracking>);

impl midnight_platform_common::transfer_namespace::TransferNamespaceChanges for Tracked {
    fn begin(
        &self,
        _: &Cancellation,
    ) -> Result<
        Box<dyn midnight_platform_common::transfer_namespace::TransferNamespaceChange>,
        FsError,
    > {
        Ok(Box::new(TrackedTurn(self.0.clone())))
    }
}

impl midnight_platform_common::transfer_namespace::TransferNamespaceChange for TrackedTurn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn midnight_core::TransferNamespacePin>>,
        _: &Cancellation,
    ) -> Result<bool, FsError> {
        self.0.bindings.lock().unwrap().extend(pin);
        Ok(true)
    }

    fn finish(self: Box<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// What publishing a link as part of a tracked job left: its outcome, and the binding the
/// job reserved for the link's stage folder.
struct TrackedPublication {
    publication: Result<TransferPublication, FsError>,
    stage: Arc<dyn midnight_core::TransferNamespacePin>,
}

/// Copy `link` into `destination` as part of a job that tracks the names it changes, as the
/// engine's link worker does: replacing the consented entry of the same name when `replace`.
/// `None` where the fixture cannot replace (not ext).
fn tracked_publication(
    link: &Path,
    destination: &Path,
    replace: bool,
) -> Option<TrackedPublication> {
    let tracking = Arc::new(Tracking::default());
    let cancel = Cancellation::default();
    midnight_platform_common::transfer_namespace::with_transfer_namespace_changes(
        Arc::new(Tracked(tracking.clone())),
        || {
            let filesystem = LinuxFileSystem::new(None);
            let source = filesystem
                .prepare_transfer_source_with_links(link, CopyLinkPolicy::Preserve, &cancel)
                .unwrap();
            let representation = source.link_representation(&cancel).unwrap();
            let directory = filesystem
                .prepare_transfer_destination(destination, &cancel)
                .unwrap();
            if replace && !directory.capabilities().link_replacement {
                return None;
            }
            let name = link.file_name().unwrap();
            let target = replace.then(|| directory.prepare_target(name, &cancel).unwrap().unwrap());
            let before = tracking.bindings.lock().unwrap().len();
            let mut stage = directory.stage_link(&representation, &cancel).unwrap();
            // The stage folder's binding is the first one its creation reserves.
            let stage_binding = tracking.bindings.lock().unwrap()[before].clone();
            stage.sync(&cancel).unwrap();
            stage.revalidate_stage(&representation, &cancel).unwrap();
            let publication = match target {
                Some(target) => {
                    stage
                        .prepare_replacement(
                            name,
                            target,
                            TransferReplacementPolicy::RecoverableCurrentEntry,
                            &cancel,
                        )
                        .unwrap();
                    stage.publish_replace(name, &cancel)
                }
                None => stage.publish_no_replace(name, &cancel),
            };
            stage.cleanup().unwrap();
            Some(TrackedPublication {
                publication,
                stage: stage_binding,
            })
        },
    )
}

/// The engine reserves every name a publication changes. A name that holds the consented
/// link (not a regular file) is reserved for that link, so Replace completes.
#[test]
fn a_link_replaces_a_consented_link_while_its_job_tracks_the_names_it_changes() {
    let fixture = fixture();
    let link = fixture.path().join("link");
    let destination = fixture.path().join("destination");
    fs::create_dir(&destination).unwrap();
    symlink("new target", &link).unwrap();
    symlink("old target", destination.join("link")).unwrap();
    let Some(published) = tracked_publication(&link, &destination, true) else {
        eprintln!("SKIP link replacement: the fixture filesystem is not ext");
        return;
    };
    let TransferPublication::Replaced { path, recovery } = published.publication.unwrap() else {
        panic!("the link replaces the consented link");
    };
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("new target"));
    assert_eq!(
        fs::read_link(&recovery.path).unwrap(),
        Path::new("old target")
    );
}

/// The stage of a published link is removed in the same turn while this process holds no
/// other handle of it, so its binding certifies the removal instead of becoming Unknown
/// (which makes the job's names opaque to every other job).
#[test]
fn a_published_link_certifies_the_removal_of_its_stage() {
    for replace in [false, true] {
        let fixture = fixture();
        let link = fixture.path().join("link");
        let destination = fixture.path().join("destination");
        fs::create_dir(&destination).unwrap();
        symlink("target", &link).unwrap();
        if replace {
            fs::write(destination.join("link"), b"displaced").unwrap();
        }
        let Some(published) = tracked_publication(&link, &destination, replace) else {
            eprintln!("SKIP link replacement: the fixture filesystem is not ext");
            continue;
        };
        assert!(published.publication.is_ok(), "replace {replace}");
        assert!(private_names(&destination).len() <= usize::from(replace));
        match published.stage.refresh(&Cancellation::default()).unwrap() {
            midnight_core::TransferNamespaceProbe::Known(scopes) => {
                assert!(scopes.is_empty(), "replace {replace}: {scopes:?}")
            }
            midnight_core::TransferNamespaceProbe::Unknown => {
                panic!("replace {replace}: the stage's removal was not certified")
            }
        }
    }
}

#[test]
fn a_link_changed_after_preparation_is_refused_before_staging() {
    let fixture = fixture();
    let link = fixture.path().join("link");
    symlink("first", &link).unwrap();
    let cancel = Cancellation::default();
    let source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&link, CopyLinkPolicy::Preserve, &cancel)
        .unwrap();
    assert_eq!(
        source.link_representation(&cancel).unwrap(),
        TransferLinkRepresentation::linux_symbolic_link(b"first").unwrap()
    );
    fs::remove_file(&link).unwrap();
    symlink("second", &link).unwrap();
    assert!(source.link_representation(&cancel).is_err());
    assert!(source.revalidate(&cancel).is_err());
}

#[test]
fn preserve_lists_links_in_a_folder_as_links_and_never_opens_targets() {
    let fixture = fixture();
    let folder = fixture.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("plain.txt"), b"plain").unwrap();
    symlink("plain.txt", folder.join("to-file")).unwrap();
    symlink(".", folder.join("to-self")).unwrap();
    symlink("missing", folder.join("dangling")).unwrap();
    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::Preserve, &cancel)
        .unwrap();
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| {
            discovered.push((item.path.file_name().unwrap().to_owned(), item.kind))
        })
        .unwrap();
    discovered.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        discovered,
        [
            ("dangling".into(), EntryKind::Symlink),
            ("folder".into(), EntryKind::Directory),
            ("plain.txt".into(), EntryKind::File),
            ("to-file".into(), EntryKind::Symlink),
            ("to-self".into(), EntryKind::Symlink),
        ]
    );
    let mut links = Vec::new();
    while let Some(child) = source.next_child(&cancel).unwrap() {
        if child.entry().item.kind == EntryKind::Symlink {
            let representation = child.link_representation(&cancel).unwrap();
            links.push(representation.linux_target().unwrap().to_vec());
            assert!(child.link_not_followed().is_none());
        }
    }
    links.sort();
    assert_eq!(
        links,
        [b".".to_vec(), b"missing".to_vec(), b"plain.txt".to_vec()]
    );
}
