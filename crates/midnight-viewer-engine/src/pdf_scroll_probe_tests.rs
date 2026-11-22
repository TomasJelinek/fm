//! Native PDF scrolling measurement over the sandboxed plugin; ignored by default, and
//! without both variables it returns at once. `MIDNIGHT_PDF_VIEWER_TEST_APPLICATION` names
//! a trusted private installation and `MIDNIGHT_PDF_SCROLL_FIXTURES` a directory holding
//! `report-3.pdf` and `report-300.pdf` (written by the PDF plugin's `render_probe
//! --write`). See docs/developer/VIEWER_SCROLLING.md.
//!
//! A wheel flick and a scrollbar drag are paced at 60 Hz for 1.5 s each. With retained
//! tiles the probe plays the UI's tiled camera: the view moves to the wanted position on a
//! display frame only when the tiles it shows have arrived, and asks the backend for that
//! camera (`PanTo`). Without tiles (the former whole-view rasters) every input is sent to
//! the backend and the view changes when a frame arrives.
use super::*;
use midnight_core::{DocumentLayout, ViewerImageInfo, ViewerImageInput, ViewerSurfaceId};
use std::{collections::HashSet, sync::atomic::Ordering};

struct Scenario {
    file: &'static str,
    device: f32,
    /// Physical pixels per PDF point; None keeps the opening Fit.
    zoom: Option<f64>,
}

#[derive(Default)]
struct Report {
    /// Display frames on which the view showed the wanted position.
    shown: usize,
    frames: usize,
    /// Time the view stood still (or stayed behind) beyond one 16 ms display frame.
    stalled_ms: f64,
    longest_stall_ms: f64,
    /// Time after the input until the view shows its final position.
    settle_ms: f64,
}

#[derive(Clone, Copy)]
enum Gesture {
    /// Physical pixels per display frame.
    Wheel(f64),
    /// Thumb positions from, to.
    Drag(f64, f64),
}

/// The UI's tiled camera, reduced to what decides whether the view can move.
struct Camera {
    info: ViewerImageInfo,
    grid: HashSet<[i32; 2]>,
    sequence: u64,
    /// The latest frame received, which coordinate inputs (a zoom step) refer to.
    revision: u64,
}

impl Camera {
    fn receive(&mut self, frame: &ViewerFrame) {
        let (Some(info), Some(tiles)) = (&frame.image, &frame.tiles) else {
            return;
        };
        if info.surface != self.info.surface {
            self.grid.clear();
            self.sequence = 0;
        }
        self.revision = frame.revision;
        for tile in tiles.iter() {
            self.grid.insert(std::array::from_fn(|axis| {
                ((f64::from(tile.position[axis]) + info.origin[axis] * info.raster_scale) / 256.0)
                    .round() as i32
            }));
        }
        self.info = info.clone();
    }

    fn covered(&self, origin: [f64; 2]) -> bool {
        let info = &self.info;
        let low: [i32; 2] =
            std::array::from_fn(|axis| (origin[axis] * info.raster_scale / 256.0).floor() as i32);
        let high: [i32; 2] = std::array::from_fn(|axis| {
            ((origin[axis] + info.visible_source[axis]) * info.raster_scale / 256.0).ceil() as i32
        });
        (low[1]..high[1]).all(|y| (low[0]..high[0]).all(|x| self.grid.contains(&[x, y])))
    }

    fn clamp(&self, origin: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| {
            let maximum = self.info.extent[axis] - self.info.visible_source[axis];
            if maximum <= 0.0 {
                maximum / 2.0
            } else {
                origin[axis].clamp(0.0, maximum)
            }
        })
    }

    fn surface(&self) -> ViewerSurfaceId {
        self.info.surface
    }
}

fn take_frames(session: &WindowSession, sink: &mut Vec<(Instant, ViewerFrame)>) {
    for output in session.mailbox.take() {
        match output {
            ViewerOutput::Frame(frame) => sink.push((Instant::now(), *frame)),
            ViewerOutput::ImageUnavailable { message, .. } => panic!("unavailable: {message}"),
            _ => {}
        }
    }
}

/// Former pipeline: every step goes to the backend; the view changes with each frame.
fn paced_backend(session: &WindowSession, gesture: Gesture, ticks: usize) -> Report {
    let mut frames = Vec::new();
    let began = Instant::now();
    let mut next = began;
    for tick in 0..ticks {
        let input = match gesture {
            Gesture::Wheel(step) => ViewerInput::Scroll {
                x: 0.0,
                y: step as f32,
            },
            Gesture::Drag(from, to) => {
                ViewerInput::Seek(from + (to - from) * (tick + 1) as f64 / ticks as f64)
            }
        };
        assert!(session.send(input));
        next += Duration::from_millis(16);
        while Instant::now() < next {
            take_frames(session, &mut frames);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let input_end = Instant::now();
    let mut quiet = Instant::now();
    while quiet.elapsed() < Duration::from_millis(1500) {
        let before = frames.len();
        take_frames(session, &mut frames);
        if frames.len() != before {
            quiet = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let during: Vec<_> = frames.iter().filter(|(at, _)| *at <= input_end).collect();
    let mut report = Report {
        shown: during.len(),
        frames: frames.len(),
        ..Default::default()
    };
    let mut previous = began;
    for (at, _) in during {
        let gap = at.duration_since(previous).as_secs_f64() * 1000.0;
        report.stalled_ms += (gap - 16.7).max(0.0);
        report.longest_stall_ms = report.longest_stall_ms.max(gap);
        previous = *at;
    }
    let gap = input_end.duration_since(previous).as_secs_f64() * 1000.0;
    report.stalled_ms += (gap - 16.7).max(0.0);
    report.longest_stall_ms = report.longest_stall_ms.max(gap);
    report.settle_ms = frames.last().map_or(f64::NAN, |(at, _)| {
        at.saturating_duration_since(input_end).as_secs_f64() * 1000.0
    });
    report
}

/// Retained tiles: the camera moves over received tiles and asks the backend to follow.
fn paced_camera(
    session: &WindowSession,
    camera: &mut Camera,
    gesture: Gesture,
    ticks: usize,
) -> Report {
    let mut report = Report::default();
    let mut frames = Vec::new();
    let mut shown = camera.info.origin;
    let mut wanted = shown;
    let mut last_request = None;
    let began = Instant::now();
    let mut next = began;
    let mut stall_began: Option<Instant> = None;
    let mut settled: Option<Instant> = None;
    let deadline = began + Duration::from_millis(16 * ticks as u64 + 5000);
    let mut tick = 0;
    loop {
        if tick < ticks {
            wanted = camera.clamp(match gesture {
                Gesture::Wheel(step) => [wanted[0], wanted[1] + step / camera.info.zoom],
                Gesture::Drag(from, to) => {
                    let position = from + (to - from) * (tick + 1) as f64 / ticks as f64;
                    let maximum = camera.info.extent[1] - camera.info.visible_source[1];
                    [wanted[0], maximum.max(0.0) * position]
                }
            });
        }
        take_frames(session, &mut frames);
        for (_, frame) in frames.drain(..) {
            report.frames += 1;
            camera.receive(&frame);
        }
        if camera.covered(wanted) {
            shown = wanted;
        }
        if last_request != Some(wanted) {
            camera.sequence += 1;
            last_request = Some(wanted);
            assert!(session.send(ViewerInput::Image {
                frame: 0,
                input: ViewerImageInput::PanTo {
                    surface: camera.surface(),
                    sequence: camera.sequence,
                    origin: wanted,
                },
            }));
        }
        let now = Instant::now();
        if shown == wanted {
            if tick < ticks {
                report.shown += 1;
            }
            if let Some(stall) = stall_began.take() {
                let stall = now.duration_since(stall).as_secs_f64() * 1000.0;
                report.stalled_ms += (stall - 16.7).max(0.0);
                report.longest_stall_ms = report.longest_stall_ms.max(stall);
            }
            if tick >= ticks {
                settled.get_or_insert(now);
                break;
            }
        } else {
            stall_began.get_or_insert(now);
        }
        if now > deadline {
            break;
        }
        tick += 1;
        next += Duration::from_millis(16);
        while Instant::now() < next {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let input_end = began + Duration::from_millis(16 * ticks as u64);
    report.settle_ms = settled.map_or(f64::NAN, |at| {
        at.saturating_duration_since(input_end).as_secs_f64() * 1000.0
    });
    report
}

#[test]
#[ignore = "measurement with MIDNIGHT_PDF_SCROLL_FIXTURES; see the module documentation"]
fn pdf_scroll_probe() {
    // A measurement, not a check: a pass over the ignored tests without its fixtures (CI's)
    // has nothing to measure, so it returns without opening anything.
    let (Some(application), Some(fixtures)) = (
        std::env::var_os("MIDNIGHT_PDF_VIEWER_TEST_APPLICATION"),
        std::env::var_os("MIDNIGHT_PDF_SCROLL_FIXTURES"),
    ) else {
        eprintln!(
            "pdf_scroll_probe measured nothing: set MIDNIGHT_PDF_SCROLL_FIXTURES and \
             MIDNIGHT_PDF_VIEWER_TEST_APPLICATION to run it"
        );
        return;
    };
    let application = std::path::PathBuf::from(application);
    let fixtures = std::path::PathBuf::from(fixtures);
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::new(executor.handle().clone(), resident);
    let runtime = Arc::new(Runtime::discover(application.parent().unwrap()).unwrap());
    let images = midnight_image_viewer_host::discover(&application).unwrap();
    *service.runtime.lock().unwrap() = Some(runtime);
    *service.images.lock().unwrap() = Some(images.into());
    *service.web.lock().unwrap() = Some(Arc::from([]));
    let mut scenarios = Vec::new();
    for file in ["report-3.pdf", "report-300.pdf"] {
        for device in [1.0, 2.0] {
            for zoom in [None, Some(1.0), Some(2.0), Some(4.0)] {
                scenarios.push(Scenario { file, device, zoom });
            }
        }
    }
    for scenario in scenarios {
        let geometry = ViewerGeometry {
            width: (2000.0 * scenario.device) as u32,
            height: (1300.0 * scenario.device) as u32,
            scale: scenario.device,
        };
        let session = service
            .open(
                launch(&fixtures, &[scenario.file], false),
                appearance(),
                geometry,
            )
            .unwrap();
        let opened = Instant::now();
        let mut displayed = frame(&session, |frame| frame.image.is_some());
        let open_ms = opened.elapsed().as_secs_f64() * 1000.0;
        assert!(session.send(ViewerInput::DocumentLayout {
            frame: displayed.revision,
            layout: DocumentLayout::Continuous,
        }));
        displayed = frame(&session, |frame| {
            frame
                .image
                .as_ref()
                .is_some_and(|image| image.document_layout == Some(DocumentLayout::Continuous))
        });
        if let Some(zoom) = scenario.zoom {
            let current = displayed.image.as_ref().unwrap().zoom;
            assert!(session.send(ViewerInput::Image {
                frame: displayed.revision,
                input: ViewerImageInput::Zoom {
                    factor: zoom / current,
                    anchor: Some([0.0, 0.0]),
                },
            }));
            displayed = frame(&session, |frame| {
                frame
                    .image
                    .as_ref()
                    .is_some_and(|image| (image.zoom - zoom).abs() < 1e-6)
            });
        }
        let zoom = displayed.image.as_ref().unwrap().zoom;
        let raster = displayed.image.as_ref().map_or([0, 0], |image| {
            image
                .visible_source
                .map(|extent| (extent * image.raster_scale).round() as u32)
        });
        // Let the prefetch settle so the measurement starts from a quiet viewer.
        std::thread::sleep(Duration::from_millis(500));
        let renders_before = crate::document_render::PAGE_RENDERS.load(Ordering::Relaxed);
        let tiled = displayed.tiles.is_some();
        let mut camera = Camera {
            info: displayed.image.clone().unwrap(),
            grid: HashSet::new(),
            sequence: 0,
            revision: displayed.revision,
        };
        camera.receive(&displayed);
        for (_, frame) in {
            let mut frames = Vec::new();
            take_frames(&session, &mut frames);
            frames
        } {
            camera.receive(&frame);
        }
        // A steady 1,200 logical px/s flick for 1.5 s, then a 1.5 s thumb drag over half
        // of the document.
        let step = 20.0 * f64::from(scenario.device);
        let gestures = [
            ("wheel", Gesture::Wheel(step)),
            ("drag", Gesture::Drag(0.1, 0.6)),
        ];
        for (name, gesture) in gestures {
            let report = if tiled {
                paced_camera(&session, &mut camera, gesture, 90)
            } else {
                paced_backend(&session, gesture, 90)
            };
            println!(
                "probe file={} device={} zoom={:.1}% raster={}x{} tiled={tiled} open_ms={open_ms:.0} \
                 gesture={name} shown_frames={}/90 stalled_ms={:.0} longest_stall_ms={:.0} \
                 settle_ms={:.0} backend_frames={}",
                scenario.file,
                scenario.device,
                zoom * 100.0,
                raster[0],
                raster[1],
                report.shown,
                report.stalled_ms,
                report.longest_stall_ms,
                report.settle_ms,
                report.frames,
            );
        }
        println!(
            "probe file={} device={} zoom={:.1}% page_renders={}",
            scenario.file,
            scenario.device,
            zoom * 100.0,
            crate::document_render::PAGE_RENDERS.load(Ordering::Relaxed) - renders_before
        );
        if tiled {
            // One zoom step (x1.25 around the view's centre): the time until the new
            // surface's visible tiles have arrived. Meanwhile the UI shows the retained
            // tiles scaled to the new zoom (`Textures::preview_zoom`).
            let surface = camera.surface();
            let began = Instant::now();
            assert!(session.send(ViewerInput::Image {
                frame: camera.revision,
                input: ViewerImageInput::Zoom {
                    factor: 1.25,
                    anchor: None,
                },
            }));
            let mut zoom_step_ms = f64::NAN;
            while began.elapsed() < Duration::from_secs(10) {
                let mut frames = Vec::new();
                take_frames(&session, &mut frames);
                for (_, frame) in frames {
                    camera.receive(&frame);
                }
                if camera.surface() != surface && camera.covered(camera.info.origin) {
                    zoom_step_ms = began.elapsed().as_secs_f64() * 1000.0;
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            println!(
                "probe file={} device={} zoom={:.1}% zoom_step_ms={zoom_step_ms:.0}",
                scenario.file,
                scenario.device,
                zoom * 100.0,
            );
        }
        session.close();
    }
}
