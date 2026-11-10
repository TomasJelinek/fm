use super::*;
use crate::{image_geometry::ImageGeometry, image_worker::Event};

impl Worker {
    pub(super) fn image_input(&mut self, input: &ViewerInput) -> bool {
        let Some(image) = &mut self.image else {
            return false;
        };
        let before = image.geometry.clone();
        let page_before = image.page;
        let pan_ack = matches!(
            input,
            ViewerInput::Image {
                input: ViewerImageInput::PanTo { .. },
                ..
            }
        );
        if let ViewerInput::Image {
            input:
                ViewerImageInput::PanTo {
                    surface,
                    sequence,
                    origin,
                },
            ..
        } = input
        {
            if !image.pan_to(*surface, *sequence, *origin) {
                return true;
            }
        } else if let ViewerInput::Resize(geometry) = input {
            image.frames.resized();
            self.image_size = [geometry.width.max(1), geometry.height.max(1)];
            self.geometry = bounded_geometry(*geometry);
            if let Some(view) = &mut image.geometry {
                view.resize(self.image_size);
            }
        } else {
            let Some(view) = &mut image.geometry else {
                return !matches!(input, ViewerInput::NextFile | ViewerInput::Close);
            };
            match input {
                ViewerInput::DocumentPage { frame, delta } => {
                    if !image.document || !self.frames.accepts(*frame) {
                        return true;
                    }
                    let page = image
                        .page
                        .saturating_add_signed(*delta as isize)
                        .min(image.pages.len().saturating_sub(1));
                    if page == image.page {
                        return true;
                    }
                    let metadata = image.pages[page];
                    image.page = page;
                    view.change_page(metadata.width, metadata.height);
                    image.frames = Default::default();
                }
                ViewerInput::Scroll { x, y } => view.scroll([f64::from(*x), f64::from(*y)]),
                ViewerInput::Seek(position) => view.seek(1, *position),
                ViewerInput::Pan(position) => view.seek(0, *position),
                ViewerInput::Image { frame, input } => {
                    if !crate::image_input::apply(view, &image.frames, *frame, input) {
                        return true;
                    }
                }
                ViewerInput::NextFile | ViewerInput::Close => return false,
                _ => return true,
            }
        }
        // A clamped Fit pan does not rerender or upload an unchanged viewport.
        if image.geometry != before || image.page != page_before || pan_ack {
            image.changed();
            self.frames.invalidate();
            self.dirty = true;
        }
        true
    }

    pub(super) fn poll_image(&mut self, output: &Mailbox) {
        let Some(event) = self
            .image
            .as_ref()
            .and_then(|image| image.worker.events.try_recv().ok())
        else {
            return;
        };
        let result = match event {
            Event::Opened { pages, document } => {
                if let Some(image) = &mut self.image {
                    let Some(metadata) = pages.first().copied() else {
                        return;
                    };
                    image.pages = pages;
                    image.document = document;
                    // Each PDF source begins in Fit; page turns retain live geometry.
                    let view = if document {
                        ImageGeometry::document(metadata.width, metadata.height, self.image_size)
                    } else {
                        let mut view =
                            ImageGeometry::new(metadata.width, metadata.height, self.image_size);
                        view.restore_zoom(self.preferences.image_zoom);
                        view
                    };
                    image.geometry = Some(view);
                    image.changed();
                    self.dirty = true;
                }
                Ok(())
            }
            Event::Frame {
                revision,
                key,
                pixels,
            } => {
                let Some(image) = &mut self.image else {
                    return;
                };
                image.pending = None;
                image.validated = Some(revision);
                self.dirty = true;
                image.scene.complete(key, pixels)
            }
            Event::Validated(revision) => {
                if let Some(image) = &mut self.image {
                    image.pending = None;
                    image.validated = Some(revision);
                    self.dirty = true;
                }
                Ok(())
            }
            Event::DocumentFrame { revision, frame } => {
                if let Some(image) = &mut self.image {
                    image.pending = None;
                    if image.revision == revision {
                        image.document_frame = Some((revision, frame));
                    }
                    self.dirty = true;
                }
                Ok(())
            }
            Event::Failed(error) => Err(error),
        };
        if let Err(error) = result {
            self.frames.invalidate();
            if let Some(image) = self.image.take() {
                output.put(ViewerOutput::ImageUnavailable {
                    generation: self.positions.generation,
                    surface: image.scene.surface,
                    message: error.clone(),
                });
                image.worker.cancel();
                self.closing_image = Some(image.worker);
            }
            self.status = error;
        }
    }

    pub(super) fn paint_image(&mut self, output: &Mailbox) -> Result<(), String> {
        let Some(image) = &mut self.image else {
            return Ok(());
        };
        let Some(paint) = image.request()? else {
            self.dirty = image.waiting_budget;
            return Ok(());
        };
        let Some(view) = &image.geometry else {
            return Ok(());
        };
        let Some(plan) = &image.scene.plan else {
            return Ok(());
        };
        let (position, vertical_extent) = view.metrics(1);
        let (horizontal_position, horizontal_extent) = view.metrics(0);
        let revision = self.frames.painted()?;
        image.frames.painted(revision);
        let (width, height, pixels, tiles) = match paint {
            crate::image_view::Paint::Tiles(tiles) => (
                plan.raster_size[0],
                plan.raster_size[1],
                Arc::from([]),
                Some(tiles.into()),
            ),
            crate::image_view::Paint::Raster(frame) => {
                (frame.width, frame.height, frame.rgba.into(), None)
            }
        };
        output.put(ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: self.positions.generation,
            position_sequence: [0; 2],
            revision,
            mode: VIEWER_MODE_IMAGE,
            modes: vec![VIEWER_MODE_IMAGE],
            wrap_enabled: false,
            wrap_available: false,
            image: Some(ViewerImageInfo {
                surface: image.scene.surface,
                origin: view.origin,
                visible_source: view.viewport.map(|value| f64::from(value) / view.scale),
                raster_scale: plan.scale,
                pan_sequence: image.scene.sequence,
                width: view.width,
                height: view.height,
                zoom: view.scale,
                fitted: view.fitted,
                page: if image.document {
                    image.page as u32 + 1
                } else {
                    0
                },
                page_count: image.pages.len() as u32,
            }),
            width,
            height,
            pixels,
            tiles,
            accessible_text: (if image.document {
                format!("Document page {} of {}", image.page + 1, image.pages.len())
            } else {
                format!("Image {} × {} pixels", view.width, view.height)
            })
            .into(),
            position,
            vertical_extent,
            horizontal_position,
            horizontal_extent,
            caption: self.caption.clone(),
            status: if image.document {
                format!(
                    "Page {} / {}  {:.1}%{}",
                    image.page + 1,
                    image.pages.len(),
                    view.scale * 100.0,
                    if view.fitted { "  Fit" } else { "" }
                )
            } else {
                format!(
                    "{} × {} pixels  {:.1}%{}",
                    view.width,
                    view.height,
                    view.scale * 100.0,
                    if view.fitted { "  Fit" } else { "" }
                )
            },
        })));
        Ok(())
    }
}
