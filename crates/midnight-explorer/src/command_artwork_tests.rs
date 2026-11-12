use super::*;
use midnight_core::PathPickerOwner;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Native {
    source: PathBuf,
    output: PathBuf,
    selected: Mutex<Vec<PathPickerKind>>,
    stored: AtomicUsize,
    cancel_at_selection: bool,
    fail_store: bool,
    import_resource: bool,
}
impl Native {
    fn fixture() -> Arc<Self> {
        let root = std::env::temp_dir();
        Arc::new(Self {
            source: root.join("original image 文档.png"),
            output: root.join("configuration/icons/managed.png"),
            selected: Mutex::default(),
            stored: AtomicUsize::new(0),
            cancel_at_selection: false,
            fail_store: false,
            import_resource: false,
        })
    }
}
impl PathPickerService for Native {
    fn command_icon_resource_requires_import(&self) -> bool {
        self.import_resource
    }
    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        self.selected.lock().unwrap().push(request.kind);
        if self.cancel_at_selection {
            cancel.cancel();
        }
        Ok(Some(self.source.clone()))
    }
    fn store_command_icon(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        _: &Cancellation,
    ) -> Result<PathBuf, FsError> {
        assert_eq!((width, height, rgba.len()), (128, 64, 128 * 64 * 4));
        self.stored.fetch_add(1, Ordering::Relaxed);
        if self.fail_store {
            Err(FsError::from_io(
                "store icon",
                &self.output,
                io::ErrorKind::PermissionDenied.into(),
            ))
        } else {
            Ok(self.output.clone())
        }
    }
}
fn request(kind: PathPickerKind) -> PathPickerRequest {
    PathPickerRequest {
        kind,
        title: "Artwork".into(),
        initial_directory: None,
        owner: PathPickerOwner::None,
    }
}
fn pixels() -> Pixels {
    Pixels {
        width: 128,
        height: 64,
        rgba: vec![255; 128 * 64 * 4],
    }
}

#[test]
fn image_selection_returns_only_the_managed_copy_and_preserves_generic_pickers() {
    let native = Native::fixture();
    let source = native.source.clone();
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(move |path, _| {
            assert_eq!(path, source);
            Ok(pixels())
        }),
    };
    let result = picker
        .pick_path(
            &request(PathPickerKind::CommandIconImage),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(result, Some(native.output.clone()));
    assert_ne!(result, Some(native.source.clone()));
    assert_eq!(
        native.selected.lock().unwrap().as_slice(),
        &[PathPickerKind::Image]
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 1);
    assert_eq!(
        picker
            .pick_path(&request(PathPickerKind::File), &Cancellation::default())
            .unwrap(),
        Some(native.source.clone())
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 1);
}

#[test]
fn cancelled_selection_or_decode_never_publishes_artwork() {
    let mut native = Native::fixture();
    Arc::get_mut(&mut native).unwrap().cancel_at_selection = true;
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(|_, _| panic!("cancelled selection decoded")),
    };
    assert!(
        picker
            .pick_path(
                &request(PathPickerKind::CommandIconImage),
                &Cancellation::default()
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 0);
    let native = Native::fixture();
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(|_, cancel| {
            cancel.cancel();
            Ok(pixels())
        }),
    };
    assert!(
        picker
            .pick_path(
                &request(PathPickerKind::CommandIconImage),
                &Cancellation::default()
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 0);
}

#[test]
fn decode_and_storage_failures_never_fall_back_to_the_original_path() {
    let native = Native::fixture();
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(|_, _| Err(io::ErrorKind::InvalidData.into())),
    };
    assert!(
        picker
            .pick_path(
                &request(PathPickerKind::CommandIconImage),
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 0);
    let mut native = Native::fixture();
    Arc::get_mut(&mut native).unwrap().fail_store = true;
    let picker = Picker {
        native,
        decode: Arc::new(|_, _| Ok(pixels())),
    };
    assert!(
        picker
            .pick_path(
                &request(PathPickerKind::CommandIconImage),
                &Cancellation::default()
            )
            .is_err()
    );
}

#[test]
fn platform_resource_import_keeps_the_resource_chooser_and_publishes_a_copy() {
    let mut native = Native::fixture();
    Arc::get_mut(&mut native).unwrap().import_resource = true;
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(|_, _| Ok(pixels())),
    };
    let result = picker
        .pick_path(
            &request(PathPickerKind::CommandIconResource),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(result, Some(native.output.clone()));
    assert_eq!(
        native.selected.lock().unwrap().as_slice(),
        &[PathPickerKind::CommandIconResource]
    );
    assert_eq!(native.stored.load(Ordering::Relaxed), 1);
    let native = Native::fixture();
    let picker = Picker {
        native: native.clone(),
        decode: Arc::new(|_, _| panic!("already managed native resource decoded twice")),
    };
    picker
        .pick_path(
            &request(PathPickerKind::CommandIconResource),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(native.stored.load(Ordering::Relaxed), 0);
}
