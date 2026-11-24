use super::*;
use std::{collections::HashMap, ffi::OsStr, path::PathBuf, sync::Mutex};

fn path(relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(std::env::temp_dir().join("feed-mutations"), |base, part| {
            base.join(part)
        })
}

fn target(relative: &str, kind: EntryKind) -> CapturedPath {
    CapturedPath {
        path: path(relative),
        kind,
        size: (kind == EntryKind::File).then_some(12),
    }
}

struct Pin(MutationItem);
impl PreparedMutation for Pin {
    fn item(&self) -> &MutationItem {
        &self.0
    }
    fn rename(&self, _: &OsStr, _: &Cancellation) -> Result<RenameCompletion, FsError> {
        panic!("preparation must not rename")
    }
    fn delete(
        &self,
        _: DeleteMode,
        _: &Cancellation,
        _: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
        panic!("preparation must not delete")
    }
}

#[derive(Default)]
struct Service {
    items: HashMap<PathBuf, MutationItem>,
    opened: Mutex<Vec<PathBuf>>,
}
impl Service {
    fn with(targets: &[CapturedPath]) -> Self {
        Self {
            items: targets
                .iter()
                .map(|t| {
                    (
                        t.path.clone(),
                        MutationItem {
                            path: t.path.clone(),
                            kind: t.kind,
                            metadata: EntryMetadata {
                                size: t.size,
                                ..Default::default()
                            },
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }
}
impl MutationService for Service {
    fn prepare(&self, path: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.opened.lock().unwrap().push(path.to_owned());
        let item = self.items.get(path).cloned().ok_or_else(|| {
            FsError::from_io("prepare", path, std::io::ErrorKind::NotFound.into())
        })?;
        Ok(Arc::new(Pin(item)))
    }
}

#[test]
fn metadata_pins_exact_cross_folder_roots_and_marks_recursive_coverage() {
    let targets = vec![
        target("a", EntryKind::Directory),
        target("b/file", EntryKind::File),
        target("a/file", EntryKind::File),
    ];
    let service = Service::with(&targets);
    let capture = CapturedPathsMetadata::new(PaneId::Right, 7, targets).unwrap();
    assert_eq!(capture.root_count(), 3);
    assert_eq!(capture.covered_roots(), [true, false, false]);
    let pins = capture.prepare(&service, &Cancellation::default()).unwrap();
    let paths: Vec<_> = pins.iter().map(|pin| pin.item().path.clone()).collect();
    assert_eq!(paths, [path("a/file"), path("b/file"), path("a")]);
    assert_eq!(*service.opened.lock().unwrap(), paths);
    assert_eq!(capture.stamp().pane, PaneId::Right);
    assert_eq!(capture.request(), TreeRequestId(7));
}

#[test]
fn mutation_capture_rejects_changed_path_kind_or_file_size() {
    let original = target("original.txt", EntryKind::File);
    for changed in [
        MutationItem {
            path: path("replacement.txt"),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(12),
                ..Default::default()
            },
        },
        MutationItem {
            path: original.path.clone(),
            kind: EntryKind::Directory,
            metadata: EntryMetadata {
                size: Some(12),
                ..Default::default()
            },
        },
        MutationItem {
            path: original.path.clone(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(13),
                ..Default::default()
            },
        },
    ] {
        let mut service = Service::with(std::slice::from_ref(&original));
        service.items.insert(original.path.clone(), changed);
        let rename = CapturedPathRename::new(PaneId::Left, 1, original.clone()).unwrap();
        assert!(
            matches!(rename.prepare(&service, &Cancellation::default()), Err(error) if error.kind == std::io::ErrorKind::InvalidData)
        );
        let metadata = CapturedPathsMetadata::new(PaneId::Left, 2, vec![original.clone()]).unwrap();
        assert!(
            matches!(metadata.prepare(&service, &Cancellation::default()), Err(error) if error.kind == std::io::ErrorKind::InvalidData)
        );
    }
}

#[test]
fn mutation_captures_validate_bounds_paths_and_cancellation_before_native_open() {
    let original = target("a.txt", EntryKind::File);
    assert!(CapturedPathRename::new(PaneId::Left, 0, original.clone()).is_err());
    assert!(CapturedPathsMetadata::new(PaneId::Left, 1, vec![]).is_err());
    assert!(
        CapturedPathsMetadata::new(PaneId::Left, 1, vec![original.clone(), original.clone()])
            .is_err()
    );
    let too_many = (0..=MAX_METADATA_TARGETS)
        .map(|i| target(&i.to_string(), EntryKind::File))
        .collect();
    assert!(CapturedPathsMetadata::new(PaneId::Left, 1, too_many).is_err());
    for invalid in [
        CapturedPath {
            path: "relative".into(),
            ..original.clone()
        },
        CapturedPath {
            path: path("a/../b"),
            ..original.clone()
        },
        CapturedPath {
            path: path("bad\0name"),
            ..original.clone()
        },
        CapturedPath {
            kind: EntryKind::Symlink,
            ..original.clone()
        },
    ] {
        assert!(CapturedPathRename::new(PaneId::Left, 1, invalid.clone()).is_err());
        assert!(CapturedPathsMetadata::new(PaneId::Left, 1, vec![invalid]).is_err());
    }
    let service = Service::with(std::slice::from_ref(&original));
    let cancel = Cancellation::default();
    cancel.cancel();
    let rename = CapturedPathRename::new(PaneId::Left, 1, original.clone()).unwrap();
    assert!(
        matches!(rename.prepare(&service, &cancel), Err(error) if error.kind == std::io::ErrorKind::Interrupted)
    );
    let metadata = CapturedPathsMetadata::new(PaneId::Left, 2, vec![original]).unwrap();
    assert!(
        matches!(metadata.prepare(&service, &cancel), Err(error) if error.kind == std::io::ErrorKind::Interrupted)
    );
    assert!(service.opened.lock().unwrap().is_empty());
}
