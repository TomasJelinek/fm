use super::*;
use midnight_core::{ViewerFrame, ViewerSurfaceId, viewer_web::HostShortcut};

fn frame(generation: u64, revision: u64) -> ViewerOutput {
    ViewerOutput::Frame(Box::new(ViewerFrame {
        generation,
        revision,
        media_control: None,
        position_sequence: [0; 2],
        mode: 2,
        modes: vec![2],
        wrap_enabled: false,
        wrap_available: true,
        image: None,
        raster: None,
        raster_rebase: None,
        width: 1,
        height: 1,
        pixels: Arc::from([1, 2, 3, 255]),
        tiles: None,
        accessible_text: "source".into(),
        position: 0.0,
        vertical_extent: 1.0,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: String::new(),
        status: String::new(),
    }))
}

fn fence(revision: u64, generation: u64, minimum_frame_revision: Option<u64>) -> ViewerOutput {
    ViewerOutput::PaletteApplied {
        revision,
        generation,
        minimum_frame_revision,
    }
}

#[test]
fn full_mailbox_retains_layout_palette_raster_and_replacement_frame_within_eight() {
    let mailbox = Mailbox::with_layout();
    for _ in 0..100 {
        mailbox.put(ViewerOutput::WebShortcut(HostShortcut::Find));
    }
    mailbox.put(ViewerOutput::LayoutSelected {
        token: 3,
        mode: 2,
        content_generation: 7,
    });
    mailbox.put(ViewerOutput::RasterRebase {
        surface: ViewerSurfaceId(4),
        transition: 9,
    });
    mailbox.put(fence(2, 7, Some(1)));
    mailbox.put(frame(7, 1));
    mailbox.put(fence(2, 8, Some(2)));
    mailbox.put(frame(8, 2));
    let outputs = mailbox.take();
    assert_eq!(outputs.len(), 8);
    assert!(matches!(outputs[0], ViewerOutput::LayoutSelected { .. }));
    assert!(matches!(
        outputs[1],
        ViewerOutput::PaletteApplied {
            revision: 2,
            generation: 8,
            minimum_frame_revision: Some(2)
        }
    ));
    assert!(outputs.iter().any(|value| matches!(value, ViewerOutput::Frame(frame) if frame.generation == 8 && frame.revision == 2)));
    assert_eq!(
        outputs
            .iter()
            .filter(|value| matches!(value, ViewerOutput::PaletteApplied { .. }))
            .count(),
        1
    );
}

#[test]
fn palette_fence_can_drain_before_frame_and_does_not_complete_opening() {
    let mailbox = Mailbox::default();
    mailbox.put(fence(3, 9, Some(12)));
    assert!(mailbox.take_open_result().is_none());
    assert!(matches!(
        mailbox.take().as_slice(),
        [ViewerOutput::PaletteApplied {
            revision: 3,
            generation: 9,
            minimum_frame_revision: Some(12)
        }]
    ));
    mailbox.put(frame(9, 12));
    assert!(
        matches!(mailbox.take().as_slice(), [ViewerOutput::Frame(frame)] if frame.generation == 9 && frame.revision == 12)
    );
    assert!(mailbox.take_open_result().unwrap().is_ok());
}

#[test]
fn old_none_or_old_request_cannot_replace_a_new_text_generation_fence() {
    let mailbox = Mailbox::default();
    mailbox.put(fence(2, 7, None));
    mailbox.put(fence(2, 8, Some(19)));
    mailbox.put(fence(1, 6, None));
    assert!(matches!(
        mailbox.take().as_slice(),
        [ViewerOutput::PaletteApplied {
            revision: 2,
            generation: 8,
            minimum_frame_revision: Some(19)
        }]
    ));
    mailbox.put(ViewerOutput::Closed);
    mailbox.put(fence(9, 20, Some(30)));
    mailbox.put(frame(20, 30));
    assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
}
