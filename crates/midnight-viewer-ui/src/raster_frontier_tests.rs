//! Travel across the retained raster and page frontiers with the real camera, wheel route
//! and key dispatch. A worker double answers every camera request as the engine does
//! (`worker_raster.rs`): it reuses its pixels while the camera stays clear of their edges,
//! paints new ones reaching `GUARD` pixels beyond the view on each side, and answers some
//! frames later, so the camera reaches the edge of the shown pixels and waits there.
//! Where its page ends it reads the next one, or hands the camera over to a corrected
//! surface (`RasterRebase`), whose world coordinates start elsewhere. A restyled page
//! (`worker_syntax.rs`) starts at the row of the camera the worker was asked for.
use super::image_presentation_tests::{advance_time, present as present_outputs};
use super::*;

/// The engine's guard around the view, physical pixels at scale 1.
const GUARD: f64 = 256.0;
/// The engine repaints when its camera comes this close to a covered edge.
const LEAD: f64 = 64.0;
/// Rows of the generated pages, physical pixels.
const PAGE: f64 = 1600.0;
const WHEEL: f32 = 60.0;
const ARROW: f32 = 28.0;

/// How the worker double gets past its page end.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Refill {
    /// The next page continues the world coordinates of this surface.
    Continue,
    /// A corrected surface takes over at the camera the window shows.
    Rebase,
}

struct Worker {
    viewport: [u32; 2],
    surface: u64,
    pixel: u64,
    revision: u64,
    /// Covered camera range of the pixels last painted.
    coverage: [f64; 2],
    /// Where the loaded page lets the camera go, on this surface.
    page: [f64; 2],
    /// The file starts at the page start (only the first surface's world starts there).
    file_start: bool,
    /// The newest camera request: origin, sequence, settled.
    latest: Option<(f64, u64, bool)>,
    /// World y of this surface at world y 0 of the first one.
    shift: f64,
    refill: Refill,
    /// Every this many moving requests, the page is restyled around the requested camera.
    restyle: Option<u32>,
    requests: u32,
    /// Frames each answer takes.
    delay: u32,
    clock: u32,
    answers: Vec<(u32, ViewerOutput)>,
    /// A page read in progress: when it is done, and whether it reads backwards.
    reading: Option<(u32, bool)>,
    rebase: Option<u64>,
    transitions: u64,
    frontiers: usize,
    restyled: usize,
}

impl Worker {
    fn new(open: &OpenWindow, refill: Refill, delay: u32, restyle: Option<u32>) -> Self {
        Self {
            viewport: [
                open.window.get_content_width().round() as u32,
                open.window.get_content_height().round() as u32,
            ],
            surface: 70,
            pixel: 0,
            revision: 10,
            coverage: [0.0; 2],
            page: [0.0, PAGE],
            file_start: true,
            latest: None,
            shift: 0.0,
            refill,
            restyle,
            requests: 0,
            delay,
            clock: 0,
            answers: Vec::new(),
            reading: None,
            rebase: None,
            transitions: 0,
            frontiers: 0,
            restyled: 0,
        }
    }

    fn maximum(&self) -> f64 {
        self.page[1] - f64::from(self.viewport[1])
    }

    /// The frame for a camera at `origin`, settled when the camera asked so. New pixels
    /// cover the guards; otherwise the frame reuses the last ones.
    fn frame(
        &mut self,
        origin: f64,
        sequence: u64,
        settled: bool,
        ack: Option<u64>,
    ) -> ViewerOutput {
        let maximum = self.maximum();
        let near =
            |edge: f64, limit: f64| (edge - origin).abs() < LEAD && (edge - limit).abs() > 1.0;
        if ack.is_some()
            || origin < self.coverage[0]
            || origin > self.coverage[1]
            || near(self.coverage[0], self.page[0])
            || near(self.coverage[1], maximum)
        {
            self.pixel += 1;
            self.coverage = [
                (origin - GUARD).max(self.page[0]),
                (origin + GUARD).min(maximum),
            ];
        }
        self.revision += 1;
        let height = (self.coverage[1] - self.coverage[0]).ceil() as u32 + self.viewport[1];
        let width = self.viewport[0];
        let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(self.revision, 0, 0.5)
        else {
            unreachable!()
        };
        frame.mode = 2;
        frame.modes = vec![1, 2];
        frame.width = width;
        frame.height = height;
        frame.pixels = vec![255; width as usize * height as usize * 4].into();
        frame.raster_rebase = ack;
        frame.raster = Some(ViewerRasterInfo {
            surface: ViewerSurfaceId(self.surface),
            pixel_id: self.pixel,
            origin: [0.0, origin],
            raster_origin: [0.0, self.coverage[0]],
            viewport: self.viewport,
            coverage_min: [0.0, self.coverage[0]],
            coverage_max: [0.0, self.coverage[1]],
            limit_min: [Some(0.0), self.file_start.then_some(0.0)],
            limit_max: [Some(0.0), None],
            pan_sequence: sequence,
            scroll_per_pixel: [0.0, 1e-6],
            scroll_map: None,
            settled,
            gutter: None,
            bands: None,
        });
        ViewerOutput::Frame(frame)
    }

    /// Paint for a requested camera inside the page, and read the next page when the
    /// camera's guard reaches past the page.
    fn answer(&mut self, target: f64, sequence: u64, settled: bool) {
        if self.reading.is_none() {
            if target + GUARD > self.maximum() {
                self.frontiers += 1;
                self.reading = Some((self.clock + self.delay * 3, false));
            } else if !self.file_start && target - GUARD < self.page[0] {
                self.frontiers += 1;
                self.reading = Some((self.clock + self.delay * 3, true));
            }
        }
        if !settled && (self.page[0]..=self.maximum()).contains(&target) {
            self.requests += 1;
            if self
                .restyle
                .is_some_and(|every| self.requests.is_multiple_of(every))
            {
                // Highlighting replaces the page with one starting at the requested
                // camera's row: the rows above it are no longer loaded.
                self.restyled += 1;
                self.page[0] = target;
                self.file_start = false;
                self.coverage = [f64::MAX, f64::MIN];
            }
        }
        if (self.page[0]..=self.maximum()).contains(&target) {
            let frame = self.frame(target, sequence, settled, None);
            self.answers.push((self.clock + self.delay, frame));
        }
    }

    /// Take the window's requests and queue the answers.
    fn receive(&mut self, inputs: Vec<ViewerInput>) {
        for input in inputs {
            match input {
                ViewerInput::Raster {
                    surface,
                    input:
                        ViewerRasterInput::PanTo {
                            sequence,
                            origin,
                            settled,
                        },
                } if surface.0 == self.surface && self.rebase.is_none() => {
                    self.latest = Some((origin[1], sequence, settled));
                    self.answer(origin[1], sequence, settled);
                }
                ViewerInput::RasterRebase { transition, origin }
                    if self.rebase == Some(transition) =>
                {
                    // The corrected page shows the displayed camera on a new surface.
                    self.rebase = None;
                    self.surface += 1;
                    self.file_start = false;
                    self.latest = None;
                    let moved = 200.0;
                    self.shift += moved;
                    self.page = [
                        origin[1] - moved - GUARD - PAGE,
                        self.page[1] - moved + PAGE,
                    ];
                    self.coverage = [f64::MAX, f64::MIN];
                    let frame = self.frame(origin[1] - moved, 0, true, Some(transition));
                    self.answers.push((self.clock + self.delay, frame));
                }
                _ => {}
            }
        }
    }

    /// One frame of the worker: due answers and a finished page read.
    fn due(&mut self) -> Vec<ViewerOutput> {
        self.clock += 1;
        let mut outputs = Vec::new();
        if let Some((at, backwards)) = self.reading
            && at <= self.clock
        {
            self.reading = None;
            if backwards {
                self.page[0] -= PAGE;
                if self.surface == 70 && self.page[0] <= 0.0 {
                    self.page[0] = 0.0;
                    self.file_start = true;
                }
            } else if self.refill == Refill::Continue {
                self.page[1] += PAGE;
            } else {
                self.transitions += 1;
                self.rebase = Some(self.transitions);
                self.answers.clear();
                outputs.push(ViewerOutput::RasterRebase {
                    surface: ViewerSurfaceId(self.surface),
                    transition: self.transitions,
                });
            }
            // The newest request is painted on the page read for it.
            if self.rebase.is_none()
                && let Some((target, sequence, settled)) = self.latest
            {
                self.answer(target, sequence, settled);
            }
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.answers)
            .into_iter()
            .partition(|(at, _)| *at <= self.clock);
        self.answers = later;
        outputs.extend(due.into_iter().map(|(_, output)| output));
        outputs
    }

    fn idle(&self) -> bool {
        self.answers.is_empty() && self.reading.is_none() && self.rebase.is_none()
    }
}

/// Fields drop in order: the windows close before their runtime stops.
struct Fixture {
    worker: Worker,
    sent: Rc<RefCell<Vec<ViewerInput>>>,
    service: Service,
    open: Rc<OpenWindow>,
    manager: image_presentation_tests::TestWindows,
    _runtime: image_presentation_tests::TestRuntime,
}

impl Fixture {
    fn new(refill: Refill, delay: u32, restyle: Option<u32>) -> Self {
        let (runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        open.image_redraw.stop();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let sink = sent.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            sink.borrow_mut().push(input);
            true
        });
        let mut worker = Worker::new(&open, refill, delay, restyle);
        let first = worker.frame(400.0, 0, true, None);
        present_outputs(&open, &service, &|_| {}, vec![first]);
        assert!(open.window.get_retained_raster());
        Self {
            worker,
            sent,
            service,
            open,
            manager,
            _runtime: runtime,
        }
    }

    /// The camera's position on the first surface's world axis.
    fn shown(&self) -> f64 {
        self.open.retained_raster.borrow().camera.origin[1] + self.worker.shift
    }

    /// One 8 ms frame: the window's motion step, then the worker's answers.
    fn frame(&mut self) {
        advance_time(Duration::from_millis(8));
        raster_motion::step(&self.open);
        let inputs = std::mem::take(&mut *self.sent.borrow_mut());
        self.worker.receive(inputs);
        let outputs = self.worker.due();
        if !outputs.is_empty() {
            present_outputs(&self.open, &self.service, &|_| {}, outputs);
        }
        let inputs = std::mem::take(&mut *self.sent.borrow_mut());
        self.worker.receive(inputs);
    }

    /// Whether everything shown and asked for came to rest within 2000 frames.
    fn rest(&mut self) -> bool {
        for _ in 0..2000 {
            self.frame();
            if self.worker.idle()
                && !self.open.raster_redraw.running()
                && !raster_rebase::waiting(&self.open)
                && self.sent.borrow().is_empty()
            {
                return true;
            }
        }
        false
    }

    /// Run `act` `times`, wait until everything is shown and answered, and return the
    /// travel, or where the camera stopped short.
    fn moved(
        &mut self,
        act: impl Fn(&OpenWindow, usize),
        times: usize,
        burst: bool,
    ) -> Result<f64, String> {
        if !self.rest() {
            return Err(format!("did not come to rest at {}", self.shown()));
        }
        let before = self.shown();
        for index in 0..times {
            act(&self.open, index);
            if !burst {
                self.frame();
            }
        }
        if !self.rest() {
            return Err(format!(
                "stopped at {:.1} after {:.1} of travel",
                self.shown(),
                self.shown() - before
            ));
        }
        Ok(self.shown() - before)
    }
}

fn notch(open: &OpenWindow, _: usize) {
    assert!(raster_motion::push(open, 0.0, WHEEL, false));
}

fn key(action: &'static str) -> impl Fn(&OpenWindow, usize) {
    move |open, _| assert!(dispatch_key(open, action), "{action}")
}

/// Wheel notches, PageDown/PageUp, a held Down and arrows four at a time; the problems
/// with the travel each one showed.
fn gestures(fixture: &mut Fixture, burst: bool, rounds: usize) -> Vec<String> {
    let scale = f64::from(fixture.worker.viewport[1])
        / f64::from(fixture.open.window.get_raster_viewport_height());
    let screen = f64::from(fixture.open.window.get_content_height()) * scale;
    let wheel = f64::from(WHEEL) * scale;
    let arrow = f64::from(ARROW) * scale;
    let mut problems = Vec::new();
    for round in 0..rounds {
        let mut check = |name: &str, moved: Result<f64, String>, expected: f64| match moved {
            Ok(moved) if (moved - expected).abs() <= 0.5 => true,
            Ok(moved) => {
                problems.push(format!("round {round} {name}: {moved:.1} of {expected:.1}"));
                true
            }
            Err(stopped) => {
                problems.push(format!("round {round} {name}: {stopped} of {expected:.1}"));
                false
            }
        };
        let moved = fixture.moved(notch, 5, burst);
        if !check("5 notches", moved, 5.0 * wheel) {
            break;
        }
        let moved = fixture.moved(key("viewer.page-down"), 1, burst);
        if !check("PageDown", moved, screen) {
            break;
        }
        let moved = fixture.moved(key("viewer.down"), 20, burst);
        if !check("20 × Down", moved, 20.0 * arrow) {
            break;
        }
        let moved = fixture.moved(key("viewer.page-up"), 1, burst);
        if !check("PageUp", moved, -screen) {
            break;
        }
        let mut up = Ok(0.0);
        for _ in 0..3 {
            up = match (up, fixture.moved(key("viewer.up"), 4, burst)) {
                (Ok(total), Ok(moved)) => Ok(total + moved),
                (Err(stopped), _) | (_, Err(stopped)) => Err(stopped),
            };
        }
        if !check("3 × 4 Up", up, -12.0 * arrow) {
            break;
        }
        let moved = fixture.moved(notch, 1, burst);
        if !check("notch", moved, wheel) {
            break;
        }
    }
    problems
}

/// Every gesture over `rounds` rounds keeps its whole travel. Each test owns one window:
/// the test platform belongs to its thread.
fn travel(refill: Refill, delay: u32, burst: bool, restyle: Option<u32>, rounds: usize) {
    let mut fixture = Fixture::new(refill, delay, restyle);
    let problems = gestures(&mut fixture, burst, rounds);
    assert!(
        problems.is_empty(),
        "{refill:?} delay {delay} burst {burst}: {problems:?}"
    );
    assert!(
        fixture.worker.frontiers > 0,
        "the gestures must cross the page frontier"
    );
    if restyle.is_some() {
        assert!(fixture.worker.restyled > 0, "the page must be restyled");
    }
    fixture.manager.close_all();
}

/// Every gesture keeps its whole travel while the camera waits at the edges of the
/// retained pixels and of the page.
#[test]
fn keys_and_wheel_keep_their_travel_across_raster_and_page_frontiers() {
    travel(Refill::Continue, 2, true, None, 6);
}

#[test]
fn keys_and_wheel_keep_their_travel_at_slow_page_reads() {
    travel(Refill::Continue, 5, false, None, 6);
}

/// The same across a corrected surface whose world coordinates start elsewhere.
#[test]
fn keys_and_wheel_keep_their_travel_across_a_corrected_surface() {
    travel(Refill::Rebase, 2, true, None, 6);
}

#[test]
fn keys_and_wheel_keep_their_travel_across_corrected_surfaces_at_slow_reads() {
    travel(Refill::Rebase, 4, false, None, 6);
}

/// A page restyled for the camera the worker was asked for starts at that camera's row,
/// above or below which the shown camera waits by the request's lookahead. Its pixels do
/// not reach the shown camera and are refused; the camera then asks for itself, and the
/// gesture goes on. It used to wait for pixels that nothing would paint, with the rest of
/// a wheel burst never shown (single inputs each asked again and did not stop).
#[test]
fn a_page_restyled_around_the_lookahead_does_not_stop_a_burst() {
    travel(Refill::Continue, 1, true, Some(5), 4);
}
