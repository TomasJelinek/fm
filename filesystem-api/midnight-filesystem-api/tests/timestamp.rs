use midnight_filesystem_api::*;

fn entries(modified: Option<Timestamp>) -> Message {
    Message::Entries {
        request: 1,
        entries: vec![Node {
            id: 2,
            parent: 1,
            name: b"a".to_vec(),
            display: "a".into(),
            kind: Kind::File,
            size: Some(1),
            readable: true,
            modified,
        }],
        more: false,
    }
}

#[test]
fn timestamps_round_trip_only_in_v2_and_legacy_bytes_do_not_change() {
    for timestamp in [
        None,
        Some(Timestamp::Utc {
            seconds: -1,
            nanoseconds: 999_999_999,
        }),
        Some(Timestamp::Local {
            year: 2024,
            month: 2,
            day: 29,
            hour: 23,
            minute: 59,
            second: 58,
        }),
    ] {
        let mut encoded = Vec::new();
        write_message_with_protocol(&mut encoded, Protocol::V2, &entries(timestamp)).unwrap();
        assert_eq!(
            read_message_with_protocol(&mut encoded.as_slice(), Protocol::V2).unwrap(),
            entries(timestamp)
        );
        encoded.clear();
        write_message(&mut encoded, &entries(timestamp)).unwrap();
        let mut legacy = Vec::new();
        write_message(&mut legacy, &entries(None)).unwrap();
        assert_eq!(encoded, legacy);
        assert_eq!(
            read_message(&mut encoded.as_slice()).unwrap(),
            entries(None)
        );
    }
}

#[test]
fn timestamps_reject_invalid_nanoseconds_and_civil_dates() {
    for timestamp in [
        Timestamp::Utc {
            seconds: 0,
            nanoseconds: 1_000_000_000,
        },
        Timestamp::Local {
            year: 1900,
            month: 2,
            day: 29,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Timestamp::Local {
            year: 2024,
            month: 4,
            day: 31,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Timestamp::Local {
            year: 2024,
            month: 1,
            day: 1,
            hour: 24,
            minute: 0,
            second: 0,
        },
    ] {
        let mut encoded = Vec::new();
        assert!(
            write_message_with_protocol(&mut encoded, Protocol::V2, &entries(Some(timestamp)))
                .is_err()
        );
        assert!(encoded.is_empty());
    }
    assert!(
        Timestamp::Local {
            year: 2000,
            month: 2,
            day: 29,
            hour: 0,
            minute: 0,
            second: 0
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn streaming_nodes_reject_reserved_or_self_parented_ids() {
    for (id, parent) in [(1, 2), (2, 0), (2, 2)] {
        let mut message = entries(None);
        if let Message::Entries { entries, .. } = &mut message {
            entries[0].id = id;
            entries[0].parent = parent;
        }
        assert!(write_message_with_protocol(&mut Vec::new(), Protocol::V2, &message).is_err());
        // Legacy can encode it, but changing only its version cannot bypass v2 validation.
        let mut encoded = Vec::new();
        write_message(&mut encoded, &message).unwrap();
        encoded[4] = 2;
        let last = encoded.len() - 1;
        encoded.insert(last, 0); // v2 unavailable timestamp
        let length = (encoded.len() as u32 - 4).to_le_bytes();
        encoded[..4].copy_from_slice(&length);
        assert!(read_message_with_protocol(&mut encoded.as_slice(), Protocol::V2).is_err());
    }
}
