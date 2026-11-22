//! The Copy/Move dialog's facts (O-1) for a Move from a folder on D: to each place of
//! `places`. The drive's own loopback share reports D:'s volume serial number, yet a
//! rename from D: cannot reach it: a Move there is a Move to another volume, refused
//! before any job, as the Move job itself would refuse it.
use super::places::{Places, cases, place, source_folder};
use super::*;
use midnight_core::DesktopRequestId;

/// Whether the dialog finds the selection's folder on `destination`'s volume, and why it
/// refuses a Move there.
fn facts(destination: &std::path::Path) -> (bool, Option<String>) {
    let facts = link_facts(
        Some(&Places),
        &TransferLinksRequest {
            request: DesktopRequestId(1),
            destination: destination.to_owned(),
            source: Some(source_folder()),
            cancel: Cancellation::default(),
        },
    )
    .unwrap();
    (facts.same_volume, facts.move_refusal())
}

#[test]
fn a_share_of_the_selections_drive_is_another_volume_though_it_reports_the_drives_serial() {
    let (same_volume, refusal) = facts(&place("loopback-d").join("inbox"));
    assert!(!same_volume, "a rename from D: cannot reach D:'s own share");
    assert_eq!(
        refusal.as_deref(),
        Some(
            "Items from another drive cannot be moved to a network location: Move cannot \
             verify their copies there before removing the originals. Copy them instead."
        )
    );
}

#[test]
fn the_dialog_refuses_exactly_the_moves_that_cannot_start() {
    for case in cases() {
        assert_eq!(
            facts(&case.destination).1.as_deref(),
            case.refusal,
            "{}",
            case.name
        );
    }
}
