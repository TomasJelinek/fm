use midnight_image_viewer_api::{self as api, DocumentMetadata, Message, Metadata, Viewport};
use std::io::Cursor;

#[test]
fn documents_preserve_ordered_page_sizes_and_page_viewports() {
    for message in [
        Message::DocumentMetadata(DocumentMetadata {
            pages: vec![
                Metadata {
                    width: 600,
                    height: 800,
                },
                Metadata {
                    width: 800,
                    height: 600,
                },
            ],
        }),
        Message::PageViewport {
            id: 39,
            page: 1,
            viewport: Viewport {
                width: 80,
                height: 60,
                scale: 2.0,
                origin_x: 7.0,
                origin_y: 9.0,
            },
        },
    ] {
        let mut bytes = Vec::new();
        api::write_message(&mut bytes, &message).unwrap();
        for end in 0..bytes.len() {
            assert!(api::read_message(&mut Cursor::new(&bytes[..end])).is_err());
        }
        assert_eq!(api::read_message(&mut Cursor::new(bytes)).unwrap(), message);
    }
}

#[test]
fn empty_oversized_and_invalid_document_metadata_is_rejected() {
    for pages in [
        vec![],
        vec![
            Metadata {
                width: 1,
                height: 1
            };
            api::MAX_DOCUMENT_PAGES + 1
        ],
        vec![Metadata {
            width: 0,
            height: 1,
        }],
    ] {
        let mut bytes = Vec::new();
        assert!(
            api::write_message(
                &mut bytes,
                &Message::DocumentMetadata(DocumentMetadata { pages })
            )
            .is_err()
        );
        assert!(bytes.is_empty());
    }
    let bytes = b"MIV1\x0d\0\0\0\x08\xff\xff\xff\xff\x01\0\0\0\x01\0\0\0";
    assert!(api::read_message(&mut Cursor::new(bytes)).is_err());
}
