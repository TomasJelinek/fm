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

/// A destination whose volume refuses a Move from another drive (here a network
/// location), on device 7 as `Devices` places "target".
struct RefusesMoves;
impl TransferService for RefusesMoves {
    fn native_drop_device(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<u64>, FsError> {
        Devices.native_drop_device(path, cancel)
    }
    fn link_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<midnight_core::TransferLinkDestination, FsError> {
        Ok(midnight_core::TransferLinkDestination {
            network: true,
            refuses_cross_volume_move: true,
            ..Default::default()
        })
    }
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("the Move admission must not prepare a source")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("the Move admission must not prepare a destination")
    }
}

/// A drag that moves does not start, before any job: with more than 64 items (the
/// sentence every Move entry point gives), or from another drive to a destination that
/// refuses such a Move (in words that say why and suggest Copy). On the same drive, or
/// where nothing tells, it may; the Move job still checks.
#[test]
fn a_move_drop_is_refused_up_front_beyond_64_items_or_across_drives_to_a_refusing_destination() {
    let cancel = Cancellation::default();
    let files = |names: Vec<&str>| FileClipboard {
        paths: names.into_iter().map(std::path::PathBuf::from).collect(),
        intent: FileClipboardIntent::Move,
    };
    let target = Path::new("target");
    let error = internal::admit_move(Some(&RefusesMoves), &files(vec!["other"]), target, &cancel)
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(
        error.message,
        "Items from another drive cannot be moved to a network location: Move cannot verify \
         their copies there before removing the originals. Copy them instead."
    );
    internal::admit_move(Some(&RefusesMoves), &files(vec!["same"]), target, &cancel).unwrap();
    internal::admit_move(
        Some(&RefusesMoves),
        &files(vec!["other"]),
        Path::new("unknown"),
        &cancel,
    )
    .unwrap();
    // A destination that can receive such a Move (the default knows nothing to refuse).
    internal::admit_move(Some(&Devices), &files(vec!["other"]), target, &cancel).unwrap();
    for service in [None, Some(&RefusesMoves as &dyn TransferService)] {
        let error =
            internal::admit_move(service, &files(vec!["same"; 65]), target, &cancel).unwrap_err();
        assert_eq!(error.message, midnight_core::MOVE_ROOTS_REFUSAL);
    }
    internal::admit_move(None, &files(vec!["same"; 64]), target, &cancel).unwrap();
}
