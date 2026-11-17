use midnight_core::*;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};

fn target(pane: PaneId, x: i32) -> FileDropTarget {
    FileDropTarget {
        pane,
        navigation: NavigationId(3),
        directory: Arc::new(std::env::current_dir().unwrap().join("literal 文档")),
        rect: DropRect {
            x,
            y: 20,
            width: 100,
            height: 80,
        },
    }
}

#[test]
fn native_hit_test_is_half_open_and_requires_current_host_and_valid_map() {
    let targets = DropTargets {
        host_generation: 9,
        revision: 4,
        targets: vec![target(PaneId::Left, 10), target(PaneId::Right, 110)],
    };
    targets.validate().unwrap();
    assert_eq!(targets.target_at(9, 10, 20).unwrap().pane, PaneId::Left);
    assert_eq!(targets.target_at(9, 110, 20).unwrap().pane, PaneId::Right);
    assert!(targets.target_at(8, 10, 20).is_none());
    assert!(targets.target_at(9, 210, 20).is_none());
    assert!(targets.target_at(9, 10, 100).is_none());
}

#[test]
fn overlapping_or_unbounded_targets_are_refused_instead_of_guessing_a_destination() {
    let mut targets = DropTargets {
        host_generation: 1,
        revision: 1,
        targets: vec![target(PaneId::Left, 0), target(PaneId::Right, 50)],
    };
    assert!(targets.validate().is_err());
    assert!(targets.target_at(1, 75, 20).is_none());
    targets.targets = vec![target(PaneId::Left, 0); 3];
    assert!(targets.validate().is_err());
    targets.targets.clear();
    targets.validate().unwrap();
    assert!(targets.target_at(1, 75, 20).is_none());
}

#[test]
fn rectangle_arithmetic_does_not_wrap_and_drop_requests_are_copy_only() {
    let rect = DropRect {
        x: i32::MAX - 1,
        y: -5,
        width: 10,
        height: 5,
    };
    assert!(rect.contains(i32::MAX, -1));
    assert!(!rect.contains(i32::MIN, -1));
    let mut request = FileDropRequest {
        files: FileClipboard {
            paths: vec![std::env::current_dir().unwrap().join("source")],
            intent: FileClipboardIntent::Move,
        },
        target: target(PaneId::Left, 0),
        targets_revision: 1,
    };
    assert!(request.validate().is_err());
    request.files.intent = FileClipboardIntent::Copy;
    request.validate().unwrap();
    request.target.directory = Arc::new("relative".into());
    assert!(request.validate().is_err());
}

/// A sink written before rejection reports existed: admission and hover only.
#[derive(Default)]
struct AdmissionOnly {
    admitted: Mutex<Vec<FileDropRequest>>,
    hovered: Mutex<Vec<Option<PaneId>>>,
}
impl FileDropSink for AdmissionOnly {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        self.admitted.lock().unwrap().push(request);
        Ok(())
    }
    fn hover(&self, pane: Option<PaneId>) {
        self.hovered.lock().unwrap().push(pane);
    }
}

#[test]
fn drop_sinks_without_a_rejection_handler_ignore_refused_drop_reports() {
    let sink = Arc::new(AdmissionOnly::default());
    let native: Arc<dyn FileDropSink> = sink.clone();
    native.reject(FsError::from_io(
        "file drag/drop",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidData, "refused native file list"),
    ));
    assert!(sink.admitted.lock().unwrap().is_empty());
    assert!(sink.hovered.lock().unwrap().is_empty());
}
