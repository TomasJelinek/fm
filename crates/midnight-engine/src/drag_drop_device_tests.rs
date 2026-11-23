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

/// A drag that moves does not start, before any job, from another drive to a destination
/// that refuses such a Move (in words that say why and suggest Copy; every place is below,
/// in `a_move_drops_admission_and_the_dialogs_facts_agree_on_every_place`). On the same
/// drive it may, of any number of items, and where the destination's facts cannot be read
/// the job decides.
#[test]
fn a_move_drop_is_refused_up_front_only_across_drives_to_a_refusing_destination() {
    use crate::transfer_destination::places::{Places, place, source_folder};
    let cancel = Cancellation::default();
    let files = |count: usize| FileClipboard {
        paths: (0..count)
            .map(|index| source_folder().join(format!("file {index}")))
            .collect(),
        intent: FileClipboardIntent::Move,
    };
    let share = place("server").join("inbox");
    let same_drive = place("drive-d").join("inbox");
    let error = internal::admit_move(Some(&Places), &files(1), &share, &cancel).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(
        error.message,
        "Items from another drive cannot be moved to a network location: Move cannot verify \
         their copies there before removing the originals. Copy them instead."
    );
    internal::admit_move(Some(&Places), &files(1), &same_drive, &cancel).unwrap();
    internal::admit_move(Some(&Places), &files(1), &place("unreadable"), &cancel).unwrap();
    // A destination that can receive such a Move (the default knows nothing to refuse).
    internal::admit_move(Some(&Devices), &files(1), &share, &cancel).unwrap();
    for service in [None, Some(&Places as &dyn TransferService)] {
        internal::admit_move(service, &files(65_536), &same_drive, &cancel).unwrap();
    }
    internal::admit_move(None, &files(65_536), &share, &cancel).unwrap();
}

/// A drop that moves from a folder whose drive cannot be read, to a place that refuses a
/// Move from another drive, reports that read's error rather than the other-drive refusal;
/// a check cancelled while it reads the selection's folder or the destination ends
/// cancelled instead of refusing or admitting the Move.
#[test]
fn a_move_drop_reports_an_unreadable_folder_and_a_cancelled_check_as_they_are() {
    use crate::transfer_destination::places::{Places, place, source_folder};
    let from = |folder: std::path::PathBuf| FileClipboard {
        paths: vec![folder.join("report.txt")],
        intent: FileClipboardIntent::Move,
    };
    let share = place("server").join("inbox");
    let admit = |files: &FileClipboard, destination: &Path| {
        internal::admit_move(Some(&Places), files, destination, &Cancellation::default())
            .unwrap_err()
    };
    let unreadable = admit(&from(place("unreadable").join("work")), &share);
    assert_eq!(unreadable.kind, io::ErrorKind::NotFound, "{unreadable:?}");
    let cancelled = admit(&from(place("cancelled").join("work")), &share);
    assert_eq!(cancelled.kind, io::ErrorKind::Interrupted, "{cancelled:?}");
    let cancelled = admit(&from(source_folder()), &place("cancelled").join("inbox"));
    assert_eq!(cancelled.kind, io::ErrorKind::Interrupted, "{cancelled:?}");
}

/// A drop that moves and the Copy/Move dialog decide alike wherever a Move from a folder
/// on D: can go (`transfer_destination::places`), so the dialog never offers a Move that
/// a drop's admission refuses, nor the reverse: the drive's own loopback share, which
/// reports D:'s volume serial number, is another volume for both.
#[test]
fn a_move_drops_admission_and_the_dialogs_facts_agree_on_every_place() {
    use crate::transfer_destination::places::{Places, cases, source_folder};
    let cancel = Cancellation::default();
    let files = FileClipboard {
        paths: vec![source_folder().join("report.txt")],
        intent: FileClipboardIntent::Move,
    };
    for case in cases() {
        let dialog = crate::transfer_destination::move_facts(
            Some(&Places),
            Some(&source_folder()),
            &case.destination,
            &cancel,
        )
        .unwrap()
        .move_refusal();
        let drop = internal::admit_move(Some(&Places), &files, &case.destination, &cancel)
            .err()
            .map(|error| error.message);
        assert_eq!(dialog.as_deref(), case.refusal, "the dialog, {}", case.name);
        assert_eq!(drop.as_deref(), case.refusal, "a drop, {}", case.name);
    }
}
