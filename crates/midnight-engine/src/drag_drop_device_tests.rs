//! Default drag action uses metadata evidence, never source mutation preparation.
use super::*;

struct Devices;
impl TransferService for Devices {
    fn native_drop_device(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        assert!(!cancel.is_cancelled());
        match path.file_name().and_then(|name| name.to_str()) {
            Some("same" | "target") => Ok(Some(7)),
            Some("other") => Ok(Some(8)),
            Some("denied") => Err(error(io::ErrorKind::PermissionDenied, "metadata denied")),
            _ => Ok(None),
        }
    }
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("default-action classification must not read or prepare a source")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("default-action classification must not prepare a destination")
    }
}

#[test]
fn automatic_drop_moves_only_when_all_native_devices_are_known_equal() {
    let cancel = Cancellation::default();
    let service = Devices;
    for (sources, expected) in [
        (vec!["same"], TransferIntent::Move),
        (vec!["same", "same"], TransferIntent::Move),
        (vec!["other"], TransferIntent::Copy),
        (vec!["same", "other"], TransferIntent::Copy),
        (vec!["same", "unknown"], TransferIntent::Copy),
        (vec!["denied"], TransferIntent::Copy),
    ] {
        let files = FileClipboard {
            paths: sources.into_iter().map(std::path::PathBuf::from).collect(),
            intent: FileClipboardIntent::Copy,
        };
        assert_eq!(
            internal::resolve_intent(
                InternalDropIntent::Auto,
                Some(&service),
                &files,
                Path::new("target"),
                &cancel
            )
            .unwrap(),
            expected
        );
    }
    let files = FileClipboard {
        paths: vec!["same".into()],
        intent: FileClipboardIntent::Copy,
    };
    assert_eq!(
        internal::resolve_intent(
            InternalDropIntent::Auto,
            None,
            &files,
            Path::new("target"),
            &cancel
        )
        .unwrap(),
        TransferIntent::Copy
    );
    assert_eq!(
        internal::resolve_intent(
            InternalDropIntent::Auto,
            Some(&service),
            &files,
            Path::new("unknown"),
            &cancel
        )
        .unwrap(),
        TransferIntent::Copy
    );
    assert_eq!(
        internal::resolve_intent(
            InternalDropIntent::Copy,
            Some(&service),
            &files,
            Path::new("target"),
            &cancel
        )
        .unwrap(),
        TransferIntent::Copy
    );
    assert_eq!(
        internal::resolve_intent(
            InternalDropIntent::Move,
            Some(&service),
            &files,
            Path::new("other"),
            &cancel
        )
        .unwrap(),
        TransferIntent::Move
    );
    cancel.cancel();
    assert_eq!(
        internal::resolve_intent(
            InternalDropIntent::Auto,
            Some(&service),
            &files,
            Path::new("target"),
            &cancel
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::Interrupted
    );
}
