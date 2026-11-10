//! One reduced viewport, retained across 1:1 inspection without another area-filter pass.
use midnight_image_viewer_sdk::api::{Frame, Viewport};
use std::{io, sync::Mutex};

#[derive(Default)]
pub(super) struct FrameCache {
    reduced: Mutex<Option<(Viewport, Frame)>>,
}

impl FrameCache {
    pub fn render(
        &self,
        viewport: Viewport,
        render: impl FnOnce() -> io::Result<Frame>,
    ) -> io::Result<Frame> {
        viewport.validate()?;
        if viewport.scale >= 1.0 {
            return render();
        }
        let mut retained = self
            .reduced
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((key, frame)) = retained.as_ref()
            && *key == viewport
        {
            return Ok(frame.clone());
        }
        let frame = render()?;
        if frame.width != viewport.width
            || frame.height != viewport.height
            || frame.rgba.len()
                != midnight_image_viewer_sdk::api::rgba_length(viewport.width, viewport.height)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid reduced image frame",
            ));
        }
        // Validated viewport dimensions cap this cache at 16 MiB. Original
        // pixels retain their independent 128 MiB limit; no scaled image of
        // arbitrary source/zoom dimensions is allocated.
        *retained = Some((viewport, frame.clone()));
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_to_exact_fit_after_pixel_inspection_reuses_filtered_pixels() {
        let cache = FrameCache::default();
        let fit = Viewport {
            width: 1,
            height: 1,
            scale: 0.25,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        let frame = Frame {
            width: 1,
            height: 1,
            rgba: vec![31, 41, 59, 255],
        };
        assert_eq!(cache.render(fit, || Ok(frame.clone())).unwrap(), frame);
        let original = Frame {
            rgba: vec![10, 20, 30, 127],
            ..frame.clone()
        };
        assert_eq!(
            cache
                .render(Viewport { scale: 1.0, ..fit }, || Ok(original.clone()))
                .unwrap(),
            original
        );
        assert_eq!(
            cache
                .render(fit, || panic!(
                    "same reduced viewport must not filter again"
                ))
                .unwrap(),
            frame
        );
        let panned = Frame {
            rgba: vec![40, 50, 60, 255],
            ..frame
        };
        assert_eq!(
            cache
                .render(
                    Viewport {
                        origin_x: 1.0,
                        ..fit
                    },
                    || Ok(panned.clone())
                )
                .unwrap(),
            panned
        );
    }
}
