//! One bounded, cancellable image actor per viewer window. Owned and reaped by the backend.
use crate::image_tiles::{Charge, EDGE, Key};
use midnight_core::ViewerPixels;
use midnight_image_viewer_host::{ImageSession, Metadata, Provider, Source, Viewport};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

pub(crate) enum Event {
    Opened {
        pages: Vec<Metadata>,
        document: bool,
    },
    Frame {
        revision: u64,
        key: Key,
        pixels: Arc<ViewerPixels>,
    },
    Validated(u64),
    DocumentFrame {
        revision: u64,
        frame: midnight_image_viewer_host::Frame,
    },
    Failed(String),
}

enum Render {
    Tile {
        revision: u64,
        key: Key,
        charge: Arc<Charge>,
    },
    Validate(u64),
    Document {
        revision: u64,
        page: usize,
        viewport: Viewport,
    },
}

pub(crate) struct ImageWorker {
    sender: SyncSender<Render>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ImageWorker {
    #[cfg(test)]
    pub(crate) fn opened_for_test(pages: Vec<Metadata>, document: bool) -> Self {
        let (sender, _) = mpsc::sync_channel(1);
        let (output, events) = mpsc::sync_channel(1);
        output.send(Event::Opened { pages, document }).unwrap();
        Self {
            sender,
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn open(
        provider: Provider,
        source: Arc<dyn Source>,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        startup: Option<crate::image_startup::Startup>,
    ) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<Render>(1);
        let (output, events) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-image".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // The reservation precedes the session so reverse local destruction
                    // reaps the native child before releasing its lifetime capacity.
                    let acquired = startup
                        .as_ref()
                        .map(|startup| startup.acquire(&provider, &stopped))
                        .transpose()
                        .map_err(|error| error.to_string())?;
                    let (_permit, prepared) = match acquired {
                        Some(crate::image_startup::Acquired { permit, prepared }) => {
                            (Some(permit), prepared)
                        }
                        None => (None, None),
                    };
                    let mut session = if let Some(prepared) = prepared {
                        // The manager already rejected stale unused spares. The host performs
                        // the final identity/source guard immediately before the consuming Open.
                        ImageSession::open_prepared(
                            &provider,
                            prepared,
                            source,
                            &stopped,
                            Duration::from_secs(30),
                        )
                    } else {
                        ImageSession::open(&provider, source, &stopped, Duration::from_secs(30))
                    }
                    .map_err(|error| error.to_string())?;
                    if let Some(startup) = &startup {
                        startup.opened(&provider);
                    }
                    let pages = (0..session.page_count())
                        .filter_map(|page| session.page_metadata(page))
                        .collect();
                    if output
                        .try_send(Event::Opened {
                            pages,
                            document: session.is_document(),
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                    wake();
                    while !stopped.load(Ordering::Acquire) {
                        match requests.recv_timeout(Duration::from_millis(10)) {
                            Ok(request) => {
                                let event = match request {
                                    Render::Tile {
                                        revision,
                                        key,
                                        charge,
                                    } => {
                                        let scale = f64::from_bits(key.scale);
                                        let mut frame = session
                                            .render_page(
                                                key.page,
                                                Viewport {
                                                    width: EDGE,
                                                    height: EDGE,
                                                    scale,
                                                    origin_x: f64::from(key.x) * f64::from(EDGE)
                                                        / scale,
                                                    origin_y: f64::from(key.y) * f64::from(EDGE)
                                                        / scale,
                                                },
                                                &stopped,
                                                Duration::from_secs(15),
                                            )
                                            .map_err(|error| error.to_string())?;
                                        crate::image_pixels::premultiply(&mut frame.rgba);
                                        let pixels =
                                            Arc::new(ViewerPixels::new(frame.rgba, charge)?);
                                        Event::Frame {
                                            revision,
                                            key,
                                            pixels,
                                        }
                                    }
                                    Render::Validate(revision) => {
                                        session
                                            .validate(&stopped)
                                            .map_err(|error| error.to_string())?;
                                        Event::Validated(revision)
                                    }
                                    Render::Document {
                                        revision,
                                        page,
                                        viewport,
                                    } => {
                                        if !session.is_document() {
                                            return Err(
                                                "Unexpected document render request.".into()
                                            );
                                        }
                                        let mut frame = session
                                            .render_page(
                                                page,
                                                viewport,
                                                &stopped,
                                                Duration::from_secs(15),
                                            )
                                            .map_err(|error| error.to_string())?;
                                        crate::image_pixels::premultiply(&mut frame.rgba);
                                        Event::DocumentFrame { revision, frame }
                                    }
                                };
                                if stopped.load(Ordering::Acquire)
                                    || output.try_send(event).is_err()
                                {
                                    break;
                                }
                                wake();
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Ok::<_, String>(())
                }))
                .unwrap_or_else(|_| Err("Image viewer stopped after an internal error.".into()));
                if let Err(error) = result
                    && output.try_send(Event::Failed(error)).is_ok()
                {
                    wake();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender,
            events,
            cancel,
            thread: Some(thread),
        })
    }

    pub fn render(&self, revision: u64, key: Key, charge: Arc<Charge>) -> bool {
        self.sender
            .try_send(Render::Tile {
                revision,
                key,
                charge,
            })
            .is_ok()
    }

    pub fn validate(&self, revision: u64) -> bool {
        self.sender.try_send(Render::Validate(revision)).is_ok()
    }
    pub fn document(&self, revision: u64, page: usize, viewport: Viewport) -> bool {
        self.sender
            .try_send(Render::Document {
                revision,
                page,
                viewport,
            })
            .is_ok()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for ImageWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        image_geometry::ImageGeometry,
        image_view::{ImageView, Paint},
    };
    use std::sync::atomic::AtomicU64;

    #[test]
    fn transparent_scenes_still_require_current_source_validation_before_publication() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender,
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        let mut geometry = ImageGeometry::new(607, 204, [1200, 800]);
        geometry.origin = [4096.0; 2];
        view.geometry = Some(geometry);
        view.changed();
        view.validated = Some(view.revision - 1);
        assert!(view.request().unwrap().is_none());
        let Render::Validate(revision) = requests.try_recv().unwrap() else {
            panic!("transparent scenes must validate without a decoder request")
        };
        assert_eq!(revision, view.revision);
        assert!(view.scene.tiles().is_some());
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
        view.pending = None;
        view.validated = Some(revision);
        let Some(Paint::Tiles(tiles)) = view.request().unwrap() else {
            panic!("validated transparent scene must remain presentable")
        };
        assert!(
            tiles
                .iter()
                .all(|tile| tile.pixels.rgba().iter().all(|byte| *byte == 0))
        );
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn magnification_reuses_tiles_but_revalidates_each_new_camera_revision() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender,
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        view.geometry = Some(ImageGeometry::new(607, 204, [1200, 800]));
        view.changed();
        while let Some((key, charge)) = view.scene.next().unwrap() {
            let pixels = ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap();
            view.scene.complete(key, Arc::new(pixels)).unwrap();
        }
        view.validated = Some(view.revision);
        assert!(matches!(view.request().unwrap(), Some(Paint::Tiles(_))));
        let geometry = view.geometry.as_mut().unwrap();
        geometry.zoom(1.25, geometry.center());
        view.changed();
        assert!(view.request().unwrap().is_none());
        let Render::Validate(revision) = requests.try_recv().unwrap() else {
            panic!("cached magnification must validate without resampling source pixels")
        };
        assert_eq!(revision, view.revision);
        assert!(view.request().unwrap().is_none());
        view.pending = None;
        view.validated = Some(revision);
        assert!(matches!(view.request().unwrap(), Some(Paint::Tiles(_))));
        assert!(requests.try_recv().is_err());
    }
    #[test]
    fn documents_request_one_bounded_viewport_without_tile_or_guard_amplification() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender,
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        view.document = true;
        let mut geometry = ImageGeometry::document(2400, 1800, [1000, 700]);
        assert!(geometry.zoom_from(
            midnight_core::ViewerZoomPreference::ScaleBasisPoints(20000),
            [1200.0, 900.0],
            [500.0, 350.0],
        ));
        view.geometry = Some(geometry);
        view.changed();
        assert!(view.request().unwrap().is_none());
        let Render::Document {
            revision,
            page,
            viewport,
        } = requests.try_recv().unwrap()
        else {
            panic!("document must use whole bounded viewport")
        };
        assert_eq!((page, viewport.width, viewport.height), (0, 1000, 700));
        assert_eq!(viewport.scale, 2.0);
        assert_eq!(view.scene.plan.as_ref().unwrap().scale, viewport.scale);
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
        view.pending = None;
        view.document_frame = Some((
            revision,
            midnight_image_viewer_host::Frame {
                width: 1000,
                height: 700,
                rgba: vec![255; 1000 * 700 * 4],
            },
        ));
        assert!(matches!(view.request().unwrap(), Some(Paint::Raster(_))));
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
    }
}
