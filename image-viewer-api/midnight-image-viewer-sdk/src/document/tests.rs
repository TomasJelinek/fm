use super::*;
use std::io::Cursor;

struct Pages;
impl Document for Pages {
    fn metadata(&self) -> api::DocumentMetadata {
        api::DocumentMetadata {
            pages: vec![
                api::Metadata {
                    width: 1,
                    height: 1
                };
                2
            ],
        }
    }
    fn render(&self, page: u32, viewport: api::Viewport) -> io::Result<api::Frame> {
        Ok(api::Frame {
            width: viewport.width,
            height: viewport.height,
            rgba: vec![page as u8; 4],
        })
    }
}

#[test]
fn session_reads_once_and_serves_second_page_then_rejects_out_of_range() {
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &api::Message::Open { source_length: 1 }).unwrap();
    api::write_message(&mut bytes, &api::Message::Data(vec![7])).unwrap();
    for page in [1, 0, 2] {
        api::write_message(
            &mut bytes,
            &api::Message::PageViewport {
                id: u64::from(page) + 20,
                page,
                viewport: api::Viewport {
                    width: 1,
                    height: 1,
                    scale: 1.0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                },
            },
        )
        .unwrap();
    }
    let mut output = Vec::new();
    session(&mut Cursor::new(bytes), &mut output, |stream| {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        assert_eq!(byte, [7]);
        Ok(Pages)
    })
    .unwrap();
    let mut output = Cursor::new(output);
    assert!(matches!(
        api::read_message(&mut output).unwrap(),
        api::Message::Read { .. }
    ));
    assert_eq!(
        api::read_message(&mut output).unwrap(),
        api::Message::DocumentMetadata(Pages.metadata())
    );
    for page in [1, 0] {
        let api::Message::Rgba { id, frame } = api::read_message(&mut output).unwrap() else {
            panic!("Expected frame")
        };
        assert_eq!(id, u64::from(page) + 20);
        assert_eq!(frame.rgba, vec![page; 4]);
    }
    assert_eq!(
        api::read_message(&mut output).unwrap(),
        api::Message::Failure
    );
}
