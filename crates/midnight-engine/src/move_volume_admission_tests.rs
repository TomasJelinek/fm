//! A Move job's admission decides as the Copy/Move dialog does (O-1): a Move from a
//! folder on D: to a place that refuses a Move from another volume (the drive's own
//! loopback share among them) does not start, in the dialog's words and before any
//! source is captured; wherever the dialog offers the Move, the job goes on to capture
//! its sources.
use super::*;
use crate::transfer_destination::places::{ADMITTED, Places, cases, source_folder};

#[test]
fn a_move_job_is_admitted_exactly_where_the_dialog_offers_the_move() {
    for case in cases() {
        let request = request(1, &[source_folder().join("report.txt")], &case.destination);
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
        let error = worker::Job::prepare_move_with_options(
            request,
            &Places,
            Cancellation::default(),
            status,
            TransferMetadataPolicy::ContentOnly,
            CopyLinkPolicy::Reject,
        )
        .err()
        .unwrap_or_else(|| panic!("{}: no source is ever captured here", case.name));
        match case.refusal {
            Some(refusal) => {
                assert_eq!(error.message, refusal, "{}", case.name);
                assert_eq!(error.kind, std::io::ErrorKind::Unsupported, "{}", case.name);
            }
            None => assert!(error.message.contains(ADMITTED), "{}: {error:?}", case.name),
        }
    }
}
