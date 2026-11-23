//! The Wayland glue's decisions and bounded pipe transfers, without a compositor.
use super::*;

#[test]
fn surface_coordinates_scale_to_window_pixels() {
    assert_eq!(physical(10.5, 20.25, 1000), Some((10, 20)));
    assert_eq!(physical(10.5, 20.25, 2000), Some((21, 40)));
    assert_eq!(physical(100.0, 7.0, 1250), Some((125, 8)));
    assert_eq!(physical(-1.0, 3.0, 1500), Some((-2, 4)));
    assert_eq!(physical(f64::NAN, 3.0, 1000), None);
    assert_eq!(physical(1.0, 1.0, 0), None);
}

#[test]
fn only_uri_lists_that_permit_copy_are_taken() {
    let types = |list: &[&str]| list.iter().map(|kind| kind.to_string()).collect::<Vec<_>>();
    assert!(copyable(&types(&["text/plain", URI_LIST]), None));
    assert!(copyable(
        &types(&[URI_LIST]),
        Some(DndAction::Copy | DndAction::Move)
    ));
    assert!(!copyable(&types(&[URI_LIST]), Some(DndAction::Move)));
    assert!(!copyable(&types(&[URI_LIST]), Some(DndAction::Ask)));
    assert!(!copyable(
        &types(&["text/plain", "UTF8_STRING"]),
        Some(DndAction::Copy)
    ));
    assert!(!copyable(&[], None));
}

#[test]
fn an_outgoing_drag_ends_with_the_compositors_report() {
    let base = Outgoing::default();
    assert_eq!(base.outcome(false), None);
    let finished = Outgoing {
        dropped: Some(Instant::now()),
        sent: true,
        finished: true,
        ..base
    };
    assert_eq!(finished.outcome(false), Some(DragFinish::Copied));
    let sent = Outgoing { sent: true, ..base };
    assert_eq!(
        sent.outcome(false),
        None,
        "version 3 waits for dnd_finished"
    );
    assert_eq!(
        sent.outcome(true),
        Some(DragFinish::Copied),
        "older devices end on send"
    );
    let cancelled = Outgoing {
        cancelled: true,
        ..base
    };
    assert_eq!(cancelled.outcome(false), Some(DragFinish::Cancelled));
    // Released before the earliest drop time: the list was withheld.
    for early in [
        Outgoing {
            too_soon: true,
            cancelled: true,
            ..base
        },
        Outgoing {
            too_soon: true,
            finished: true,
            ..base
        },
    ] {
        assert_eq!(early.outcome(false), Some(DragFinish::ReleasedTooSoon));
    }
    assert_eq!(
        Outgoing {
            too_soon: true,
            ..base
        }
        .outcome(false),
        None
    );
}

#[test]
fn transfers_are_bounded_in_size_and_time() {
    // A complete list arrives whole.
    let (reader, writer) = std::io::pipe().unwrap();
    let sender = std::thread::spawn(move || {
        write_bounded(writer.into(), b"file:///tmp/a\r\n", Duration::from_secs(2))
    });
    assert_eq!(
        read_bounded(reader.into(), Duration::from_secs(2)).unwrap(),
        b"file:///tmp/a\r\n"
    );
    assert!(sender.join().unwrap());

    // A source that never closes its end runs into the timeout.
    let (reader, writer) = std::io::pipe().unwrap();
    let started = Instant::now();
    let error = read_bounded(reader.into(), Duration::from_millis(150)).unwrap_err();
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(error.to_string().contains(SOURCE_LIST_MISSING), "{error}");
    drop(writer);

    // More than the file list limit is refused while reading, not after.
    let (reader, writer) = std::io::pipe().unwrap();
    let sender = std::thread::spawn(move || {
        let chunk = vec![b'a'; 1024 * 1024];
        let mut writer = std::fs::File::from(OwnedFd::from(writer));
        for _ in 0..=(MAX_FILE_CLIPBOARD_BYTES / chunk.len()) {
            if writer.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    let error = read_bounded(reader.into(), Duration::from_secs(5)).unwrap_err();
    assert!(error.to_string().contains(LIST_TOO_LARGE), "{error}");
    sender.join().unwrap();

    // A target that stops reading cannot hold the actor past the timeout.
    let (reader, writer) = std::io::pipe().unwrap();
    let started = Instant::now();
    assert!(!write_bounded(
        writer.into(),
        &vec![0u8; 4 * 1024 * 1024],
        Duration::from_millis(150)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(reader);
}
