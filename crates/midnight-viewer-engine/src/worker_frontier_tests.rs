//! Travel across the retained raster and source page frontiers of a long Rust file: a
//! deterministic co-simulation of the worker, its plugin and the viewer window.
//!
//! The real worker shows the acceptance scenario's generated Rust source in the Source
//! mode, or rows of a given width. A simulated Source or basic viewer answers its viewport
//! requests as the plugins select them (`plugins/midnight-viewer-source/src/paging.rs`,
//! `plugins/midnight-viewer-basic/src/window.rs`): at most about 8 KiB and the request's
//! line budget from the anchor, or the same before it, some frames later. It also prepares
//! highlighting and answers the refresh that restyles the shown page (`worker_syntax.rs`).
//! The viewer window's side is a model of the retained camera of `midnight-viewer-ui`
//! (`raster_camera.rs`): it shows covered pixels at once, waits at a coverage edge until a
//! refill arrives, asks the worker for its camera with `PanTo`, refuses pixels that do not
//! cover it, and hands its travel over across a `RasterRebase`. A slow window presents
//! every `ui_period` worker turns, while the worker's mailbox keeps only the newest frame.
//! Every gesture is measured as the document distance the shown content moved, from the
//! row and offset of the first whole row of the worker's page.
use super::text_position::edge_page;
use super::*;
use midnight_plugin_host::HostEvent;
use std::fmt::Write as _;

/// The Source viewer's page budget (`max_bytes.min(8192)` in its paging).
const SOURCE_BYTES: usize = 8192;
/// A wheel notch, as Slint's winit backend delivers a line (`image_motion::route`).
const NOTCH: f64 = 60.0;
/// An arrow key (`scrolling::keyboard`).
const ARROW: f64 = 28.0;
/// The retained camera's request lookahead (`raster_camera::PREFETCH`).
const PREFETCH: f64 = 128.0;
/// Most pixels an eased wheel frame moves here; the real glide is finer.
const EASED_STEP: f64 = 12.0;

/// The acceptance scenario's generated Rust source (`viewers_fixtures::rust`).
pub(super) fn rust(lines: usize) -> String {
    let mut text = String::from("// Generated for the viewer scenarios.\n");
    let mut line = 1;
    let mut function = 0;
    while line < lines {
        let _ = writeln!(
            text,
            "/// Function {function}.\nfn function_{function}() -> u64 {{"
        );
        line += 2;
        for step in 0..12 {
            let _ = writeln!(
                text,
                "    let value_{step} = compute(\"item {line}\", {line} * {step}); // line {line}"
            );
            line += 1;
        }
        text.push_str("    0\n}\n");
        line += 2;
        function += 1;
    }
    text
}

/// What the Source viewer answers to `viewport` over `source` (ASCII here).
pub(super) fn source_reply(source: &str, viewport: &messages::Viewport) -> viewer::TextPage {
    let limit = (viewport.max_bytes as usize).min(SOURCE_BYTES);
    let anchor = viewport.source_anchor as usize;
    let max_lines = viewport.max_lines as usize;
    let (start, end) = if viewport.backwards {
        // `decode::suffix_start`: a trailing newline ends its row, not another line.
        let mut start = anchor;
        let mut lines = 0;
        for (at, character) in source[..anchor].char_indices().rev() {
            if anchor - at > limit || max_lines == 0 {
                break;
            }
            if character == '\n' && at + 1 != anchor {
                lines += 1;
                if lines >= max_lines {
                    break;
                }
            }
            start = at;
        }
        (start, anchor)
    } else {
        let mut end = anchor;
        let mut lines = 0;
        for character in source[anchor..].chars() {
            if end - anchor + character.len_utf8() > limit || lines >= max_lines {
                break;
            }
            end += character.len_utf8();
            if character == '\n' {
                lines += 1;
            }
        }
        (anchor, end)
    };
    let mut page = edge_page(source, start, end);
    page.partial_line = (start > 0 && !source[..start].ends_with('\n'))
        || (end < source.len() && !source[..end].ends_with('\n'));
    page
}

/// A request the simulated plugin answers.
#[derive(Clone, Debug)]
enum Request {
    /// A viewport, or a highlighted page that refreshes the shown one.
    Page(messages::Viewport),
    /// Syntax preparation for the source the view shows.
    Prepare,
}

/// The basic viewer's page budget (`READ_BYTES - 16`).
const BASIC_BYTES: usize = 8176;

/// What the basic viewer answers to `viewport` over `source` (ASCII here): it reads
/// `min(max_bytes, 8176)` bytes and selects whole units up to the line budget
/// (`plugins/midnight-viewer-basic/src/window.rs`), so a page can end inside a row.
pub(super) fn basic_reply(source: &str, viewport: &messages::Viewport) -> viewer::TextPage {
    let count = (viewport.max_bytes as usize).min(BASIC_BYTES);
    let anchor = (viewport.source_anchor as usize).min(source.len());
    let max_lines = viewport.max_lines as usize;
    let (start, end) = if viewport.backwards {
        let limit = anchor.saturating_sub(count);
        let mut start = limit;
        let mut lines = 0;
        for at in (limit..anchor).rev() {
            if source.as_bytes()[at] == b'\n' && at + 1 < anchor {
                lines += 1;
                if lines >= max_lines {
                    start = at + 1;
                    break;
                }
            }
        }
        (start, anchor)
    } else {
        let mut end = (anchor + count).min(source.len());
        let mut lines = 0;
        for at in anchor..end {
            if source.as_bytes()[at] == b'\n' {
                lines += 1;
                if lines >= max_lines {
                    end = at + 1;
                    break;
                }
            }
        }
        (anchor, end)
    };
    let mut page = edge_page(source, start, end);
    page.partial_line = !(start == 0 || source[..start].ends_with('\n'))
        || (page.has_after && !page.text.ends_with('\n'));
    page
}

/// The real worker, fed by a simulated Source viewer that answers each request after
/// `delay` frames.
pub(super) struct Engine {
    pub worker: Worker,
    pub output: Mailbox,
    pub source: String,
    queue: Vec<(u64, Request, u32)>,
    pub delay: u32,
    pub tick: u32,
    /// Ranges the simulated plugin read and answered.
    pub reads: usize,
    line_height: f64,
    inbox: Vec<ViewerInput>,
    /// The worker takes its inputs and paints every `period` frames of the window.
    pub period: u32,
    /// The plugin prepares highlighting and the worker restyles the shown page.
    pub highlight: bool,
    pub refreshes: usize,
    /// Text bytes of the pages the simulated plugin answered.
    pub bytes: usize,
    /// The window presents every `ui_period` frames; meanwhile the worker's outputs wait
    /// in its mailbox, which keeps only the newest frame (`Mailbox::put`).
    pub ui_period: u32,
    outbox: Vec<(ViewerOutput, Option<f64>)>,
    /// Frames syntax preparation and its refresh take, when not `delay`.
    pub prepare_delay: Option<u32>,
}

const FIRST_REQUEST: u64 = 1 << 20;

impl Engine {
    pub fn open(source: String, mode: u16, geometry: ViewerGeometry, delay: u32) -> Self {
        let (mut worker, output) = worker();
        worker.mode = mode;
        worker.modes = vec![mode];
        worker.wrap = false;
        worker.length = source.len() as u64;
        worker.geometry = geometry;
        worker.image_size = [geometry.width, geometry.height];
        worker.input(ViewerInput::RetainedRaster(true), &output);
        unsent::take();
        unsent::take_admitted();
        unsent::admit_from(Some(FIRST_REQUEST));
        let mut engine = Self {
            worker,
            output,
            source,
            queue: Vec::new(),
            delay,
            tick: 0,
            reads: 0,
            line_height: 0.0,
            inbox: Vec::new(),
            period: 1,
            highlight: false,
            refreshes: 0,
            bytes: 0,
            ui_period: 1,
            outbox: Vec::new(),
            prepare_delay: None,
        };
        engine.worker.request_page(0, false, &engine.output);
        engine.collect();
        engine
    }

    /// Highlighting starts with the opened view, as the first request's demand does
    /// (`source_requested`, which needs the captured file's revision).
    pub fn highlighted(mut self) -> Self {
        self.highlight = true;
        let viewport = self.worker.page_viewport(0, false);
        self.worker
            .text
            .syntax
            .demand(self.worker.generation, 1, viewport);
        self
    }

    /// Queue the requests the worker sent, under the IDs it admitted them with.
    fn collect(&mut self) {
        unsent::take();
        for (id, viewport) in unsent::take_admitted() {
            self.queue
                .push((id, Request::Page(viewport), self.tick + self.delay));
        }
    }

    /// Admit syntax preparation and its refresh as the worker loop does after painting
    /// (`prepare_source`), with the IDs a session would give them.
    fn prepare_source(&mut self) {
        let worker = &mut self.worker;
        if !self.highlight || !worker.source_preparation_admissible() {
            return;
        }
        worker.source_stationary_target();
        let now = std::time::Instant::now();
        let due = self.tick + self.prepare_delay.unwrap_or(self.delay);
        if let Some(refresh) = &worker.text.syntax.refresh {
            let key = refresh.key.clone();
            if worker.text.syntax.latest.as_ref() != Some(&key) {
                worker.invalidate_source_target();
                return;
            }
            if !worker.text.syntax.refresh_due(now) {
                return;
            }
            let id = unsent::next_id().unwrap();
            worker.source_refresh_admission(&key, Ok(id), &self.output);
            self.queue.push((id, Request::Page(key.viewport), due));
            return;
        }
        if !worker.text.syntax.needs_preparation(now) {
            return;
        }
        let Some(key) = worker.text.syntax.latest.clone() else {
            return;
        };
        let id = unsent::next_id().unwrap();
        worker.source_prepare_admission(&key, Ok(id), &self.output);
        self.queue.push((id, Request::Prepare, due));
    }

    /// Queue an input for the worker's next turn.
    pub fn input(&mut self, input: ViewerInput) {
        self.inbox.push(input);
    }

    fn apply_inputs(&mut self) {
        for input in std::mem::take(&mut self.inbox) {
            self.worker.input(input, &self.output);
            self.collect();
        }
    }

    fn live(&self, id: u64, request: &Request) -> bool {
        if matches!(request, Request::Prepare) {
            return true;
        }
        [
            self.worker.viewport_request,
            self.worker.source_refresh_request(),
        ]
        .into_iter()
        .flatten()
        .any(|logical| logical == id || self.worker.chained_request(logical) == Some(id))
    }

    /// Answer the requests that are due and still awaited; superseded ones were
    /// cancelled before the plugin read them.
    pub fn answer(&mut self) {
        loop {
            let Some(index) = self.queue.iter().position(|(_, _, due)| *due <= self.tick) else {
                return;
            };
            let (id, request, _) = self.queue.remove(index);
            if !self.live(id, &request) {
                continue;
            }
            let events = match &request {
                Request::Page(viewport) => {
                    self.reads += 1;
                    if self.worker.source_refresh_request() == Some(id) {
                        self.refreshes += 1;
                    }
                    let page = if self.worker.mode == viewer::MODE_SOURCE {
                        source_reply(&self.source, viewport)
                    } else {
                        basic_reply(&self.source, viewport)
                    };
                    self.bytes += page.text.len();
                    vec![EventKind::TextPage(page)]
                }
                Request::Prepare => vec![EventKind::SourcePrepared(messages::SourcePrepared {
                    revision: 1,
                    frontier: self.source.len() as u64,
                    ready: true,
                })],
            };
            let generation = self.worker.generation;
            self.worker.session_events(
                events
                    .into_iter()
                    .chain([EventKind::Completed])
                    .map(|kind| HostEvent {
                        request_id: id,
                        generation,
                        kind,
                        fatal: false,
                    })
                    .collect(),
                &self.output,
            );
            self.collect();
        }
    }

    pub fn busy(&self) -> bool {
        !self.inbox.is_empty()
            || !self.outbox.is_empty()
            || self
                .queue
                .iter()
                .any(|(id, request, _)| self.live(*id, request))
    }

    /// Paint as the worker loop does, then hand the outputs to the window.
    pub fn paint(&mut self) -> Vec<(ViewerOutput, Option<f64>)> {
        if self.worker.dirty {
            self.worker.dirty = false;
            self.worker.paint(&self.output).unwrap();
        }
        // Every page the worker shows holds the file's text at its source range.
        if let Some(page) = &self.worker.page {
            let range = page.start as usize..page.end as usize;
            assert_eq!(&*page.text, &self.source[range.clone()], "page {range:?}");
            assert!(
                page.anchors.iter().all(
                    |anchor| anchor.source_offset - page.start == u64::from(anchor.text_offset)
                ),
                "anchors of page {range:?}"
            );
        }
        self.collect();
        let document = self.document_y();
        self.output
            .take()
            .into_iter()
            .map(|output| {
                let offset = match &output {
                    ViewerOutput::Frame(frame) => frame.raster.as_ref().map_or(document, |info| {
                        document.map(|document| document - info.origin[1])
                    }),
                    _ => None,
                };
                (output, offset)
            })
            .collect()
    }

    /// Document y of the worker camera, from the first whole row its page shows.
    pub fn document_y(&mut self) -> Option<f64> {
        let (page, shaped) = self.worker.page.as_ref().zip(self.worker.shaped.as_ref())?;
        let start = page.start as usize;
        let whole = if start == 0 || self.source.as_bytes()[start - 1] == b'\n' {
            0
        } else {
            page.text.find('\n')? + 1
        };
        let top = shaped.caret_geometry(whole).ok()?.y0;
        if self.line_height == 0.0 {
            let second = whole + page.text[whole..].find('\n')? + 1;
            self.line_height = shaped.caret_geometry(second).ok()?.y0 - top;
        }
        let source = page
            .anchors
            .iter()
            .find(|anchor| anchor.text_offset as usize == whole)?
            .source_offset as usize;
        let row = self.source.as_bytes()[..source]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        Some(row as f64 * self.line_height - top + f64::from(self.worker.origin[1]))
    }
}

/// The viewer window's retained camera (`raster_camera.rs`) and its routes for input
/// without retained pixels (`raster_motion.rs`), reduced to the vertical axis.
#[derive(Default)]
pub(super) struct Window {
    info: Option<ViewerRasterInfo>,
    origin: f64,
    /// Document y of world y 0 on the shown surface, or of the shown viewport.
    offset: f64,
    retained: bool,
    /// Logical pixels of the gesture the camera has not shown yet.
    pending: f64,
    direct: bool,
    sequence: u64,
    last_request: Option<(f64, bool)>,
    rebase: Option<u64>,
    rebase_sent: bool,
    accepted_rebase: Option<u64>,
    last_transition: u64,
    /// Pixels of a plain glide not sent yet, while no retained pixels were shown.
    unsent_raw: f64,
    /// A key moved plain content: relative input keeps that route (`admit_raw`).
    raw_owned: bool,
    /// Why travel was dropped, with the pixels it dropped.
    pub dropped: Vec<String>,
    pub log: Vec<String>,
    /// Frames this window showed without retained pixels, new surfaces and rebases.
    pub plain_frames: usize,
    pub surfaces: usize,
    pub rebases: usize,
    /// Frames on which the camera waited at a coverage edge with travel left.
    pub edge_waits: usize,
    position_sequence: u64,
    /// Pixels of the shown surface that did not reach the shown camera were refused; the
    /// camera then asks for itself without lookahead (`raster_camera::Camera::receive`).
    uncovered: bool,
    /// As before that request was made: a refused frame left the camera waiting.
    pub wait_after_refusal: bool,
    pub refused: usize,
}

impl Window {
    pub fn shown(&self) -> f64 {
        if self.retained {
            self.origin + self.offset
        } else {
            self.offset
        }
    }

    fn clamp(info: &ViewerRasterInfo, origin: f64) -> f64 {
        origin.clamp(
            info.limit_min[1].unwrap_or(f64::NEG_INFINITY),
            info.limit_max[1].unwrap_or(f64::INFINITY),
        )
    }

    fn covered(info: &ViewerRasterInfo, origin: f64) -> bool {
        origin >= info.coverage_min[1] && origin <= info.coverage_max[1]
    }

    /// A wheel notch or a key, in physical pixels: `direct` ones (keys) show their whole
    /// distance at once.
    pub fn push(&mut self, engine: &mut Engine, pixels: f64, direct: bool) {
        if self.raw_owned {
            engine.input(ViewerInput::Scroll {
                x: 0.0,
                y: pixels as f32,
            });
            return;
        }
        if self.rebase.is_some() {
            self.pending += pixels;
            self.direct = direct;
            return;
        }
        if !self.retained {
            // Keys take raw relative scrolling; the wheel glides it frame by frame.
            if direct {
                self.raw_owned = true;
                self.log.push("a key took raw scrolling".into());
                engine.input(ViewerInput::Scroll {
                    x: 0.0,
                    y: pixels as f32,
                });
            } else {
                self.unsent_raw += pixels;
            }
            return;
        }
        if pixels.signum() != self.pending.signum() {
            self.pending = 0.0;
        }
        self.pending += pixels;
        self.direct = direct;
        if direct {
            self.tick(engine);
        }
    }

    pub fn active(&self) -> bool {
        self.pending != 0.0 || self.unsent_raw != 0.0 || self.rebase.is_some()
    }

    /// A scrollbar drag released at `position` where no pixels cover it: the window asks
    /// the worker for that position (`TextPosition`).
    pub fn seek(&mut self, engine: &mut Engine, position: f64) {
        self.pending = 0.0;
        self.unsent_raw = 0.0;
        self.raw_owned = false;
        self.position_sequence += 1;
        let generation = engine.worker.positions.generation;
        engine.input(ViewerInput::Position {
            generation,
            sequence: self.position_sequence,
            horizontal: false,
            position,
        });
    }

    /// One presented frame.
    pub fn tick(&mut self, engine: &mut Engine) {
        if self.unsent_raw != 0.0 {
            let step = self.unsent_raw.clamp(-EASED_STEP, EASED_STEP);
            self.unsent_raw -= step;
            engine.input(ViewerInput::Scroll {
                x: 0.0,
                y: step as f32,
            });
        }
        if self.rebase.is_some() {
            if !self.rebase_sent
                && let (Some(transition), Some(_)) = (self.rebase, &self.info)
            {
                self.rebase_sent = true;
                engine.input(ViewerInput::RasterRebase {
                    transition,
                    origin: [0.0, self.origin],
                });
            }
            return;
        }
        let Some(info) = self.info.clone() else {
            return;
        };
        if self.pending != 0.0 {
            if Self::clamp(&info, self.origin + self.pending) == self.origin {
                self.log
                    .push(format!("limit at {} drops {}", self.origin, self.pending));
                self.pending = 0.0;
            } else {
                let low = info.coverage_min[1].max(info.limit_min[1].unwrap_or(f64::NEG_INFINITY));
                let high = info.coverage_max[1].min(info.limit_max[1].unwrap_or(f64::INFINITY));
                let step = if self.direct {
                    self.pending
                } else {
                    self.pending.clamp(-EASED_STEP, EASED_STEP)
                };
                let delivered =
                    step.clamp((low - self.origin).min(0.0), (high - self.origin).max(0.0));
                if delivered == 0.0 {
                    self.edge_waits += 1;
                }
                self.origin += delivered;
                self.pending -= delivered;
            }
        }
        let settled = self.pending == 0.0;
        let lookahead = if self.uncovered {
            0.0
        } else {
            PREFETCH.min((info.coverage_max[1] - info.coverage_min[1]) / 2.0)
        };
        let origin = Self::clamp(
            &info,
            self.origin + self.pending.clamp(-lookahead, lookahead),
        );
        if self.last_request != Some((origin, settled)) {
            self.sequence += 1;
            self.last_request = Some((origin, settled));
            engine.input(ViewerInput::Raster {
                surface: info.surface,
                input: ViewerRasterInput::PanTo {
                    sequence: self.sequence,
                    origin: [info.origin[0], origin],
                    settled,
                },
            });
        }
    }

    pub fn receive(&mut self, output: ViewerOutput, offset: Option<f64>) {
        match output {
            ViewerOutput::Frame(frame) => {
                let allowed = match self.rebase {
                    Some(transition) => self.rebase_sent && frame.raster_rebase == Some(transition),
                    None => {
                        frame.raster_rebase.is_none() || frame.raster_rebase == self.accepted_rebase
                    }
                };
                if !allowed {
                    self.log.push("frame refused during a rebase".into());
                    return;
                }
                let Some(offset) = offset else {
                    return;
                };
                let rebased = self.rebase.is_some();
                match frame.raster {
                    None => {
                        if self.retained && self.pending != 0.0 {
                            self.dropped.push(format!(
                                "a frame without retained pixels dropped {}",
                                self.pending
                            ));
                            self.pending = 0.0;
                        }
                        if rebased {
                            self.unsent_raw += self.pending;
                            self.pending = 0.0;
                        }
                        self.retained = false;
                        self.info = None;
                        self.offset = offset;
                        self.plain_frames += 1;
                    }
                    Some(mut info) => {
                        info.gutter = None;
                        let changed = self
                            .info
                            .as_ref()
                            .is_none_or(|old| old.surface != info.surface);
                        let exact = !changed
                            && info.settled
                            && self.pending == 0.0
                            && info.pan_sequence == self.sequence
                            && self.last_request == Some((self.origin, true))
                            && (info.origin[1] - self.origin).abs() < 0.25;
                        let origin = if changed || exact {
                            info.origin[1]
                        } else {
                            Self::clamp(&info, self.origin)
                        };
                        if !Self::covered(&info, origin) {
                            self.log.push(format!(
                                "frame refused: {origin} outside {}..{}",
                                info.coverage_min[1], info.coverage_max[1]
                            ));
                            self.refused += 1;
                            if !changed && !self.wait_after_refusal {
                                self.uncovered = true;
                                self.last_request = None;
                            }
                            return;
                        }
                        self.uncovered = false;
                        if changed {
                            self.surfaces += 1;
                            if self.pending != 0.0 && !rebased {
                                self.dropped.push(format!(
                                    "a new surface {} dropped {}",
                                    info.surface.0, self.pending
                                ));
                                self.pending = 0.0;
                            }
                            if self.unsent_raw != 0.0 {
                                self.dropped.push(format!(
                                    "retained pixels stranded {} of a plain glide",
                                    self.unsent_raw
                                ));
                                self.unsent_raw = 0.0;
                            }
                            self.last_request = Some((origin, info.settled));
                        }
                        if rebased {
                            self.rebase = None;
                            self.accepted_rebase = frame.raster_rebase;
                        }
                        self.origin = origin;
                        self.offset = offset;
                        self.retained = true;
                        self.info = Some(info);
                    }
                }
            }
            ViewerOutput::RasterRebase {
                surface,
                transition,
            } => {
                if transition <= self.last_transition
                    || self.rebase.is_some()
                    || self
                        .info
                        .as_ref()
                        .is_none_or(|info| info.surface != surface)
                {
                    return;
                }
                self.last_transition = transition;
                self.accepted_rebase = None;
                self.rebase = Some(transition);
                self.rebase_sent = false;
                self.rebases += 1;
            }
            ViewerOutput::RasterUnavailable { surface, message } => {
                let current = self
                    .info
                    .as_ref()
                    .is_some_and(|info| info.surface == surface);
                if current || self.rebase.is_some() {
                    if self.pending != 0.0 {
                        self.dropped
                            .push(format!("unavailable ({message}) dropped {}", self.pending));
                    }
                    self.pending = 0.0;
                    self.rebase = None;
                }
            }
            _ => {}
        }
    }
}

/// One frame: the window presents, the worker applies its input, the plugin answers
/// what is due, and the worker paints.
pub(super) fn frame(engine: &mut Engine, window: &mut Window) {
    let presents = engine.tick.is_multiple_of(engine.ui_period);
    if presents {
        window.tick(engine);
    }
    if engine.tick.is_multiple_of(engine.period) {
        engine.apply_inputs();
        engine.answer();
        let outputs = engine.paint();
        engine.prepare_source();
        for (output, offset) in outputs {
            // The mailbox keeps the newest frame and the newest raster control output.
            let raster = |output: &ViewerOutput| {
                matches!(
                    output,
                    ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
                )
            };
            if matches!(output, ViewerOutput::Frame(_)) {
                engine
                    .outbox
                    .retain(|(old, _)| !matches!(old, ViewerOutput::Frame(_)));
            } else if raster(&output) {
                engine.outbox.retain(|(old, _)| !raster(old));
            }
            engine.outbox.push((output, offset));
        }
    }
    if presents {
        for (output, offset) in std::mem::take(&mut engine.outbox) {
            window.receive(output, offset);
        }
    }
    engine.tick += 1;
}

/// Frames until the gesture is shown and the plugin has nothing left to answer.
pub(super) fn rest(engine: &mut Engine, window: &mut Window) {
    assert!(
        settle(engine, window),
        "the gesture did not come to rest: {:?}",
        window.log
    );
}

/// Whether the gesture came to rest within 2000 frames.
pub(super) fn settle(engine: &mut Engine, window: &mut Window) -> bool {
    for _ in 0..2000 {
        frame(engine, window);
        if !window.active() && !engine.busy() && !engine.worker.dirty {
            // A settled request may still be on its way: one more round.
            frame(engine, window);
            if !window.active() && !engine.busy() && !engine.worker.dirty {
                return true;
            }
        }
    }
    false
}

fn geometry(width: u32, height: u32, scale: f32) -> ViewerGeometry {
    ViewerGeometry {
        width,
        height,
        scale,
    }
}

/// Show `inputs` (physical pixels; keys are direct) and return how far the content moved.
/// A `burst` arrives within one frame.
pub(super) fn gesture(
    engine: &mut Engine,
    window: &mut Window,
    inputs: &[(f64, bool)],
    burst: bool,
) -> f64 {
    try_gesture(engine, window, inputs, burst)
        .unwrap_or_else(|| panic!("the gesture did not come to rest: {:?}", window.log))
}

/// [`gesture`], or `None` when the content stops short and never comes to rest.
pub(super) fn try_gesture(
    engine: &mut Engine,
    window: &mut Window,
    inputs: &[(f64, bool)],
    burst: bool,
) -> Option<f64> {
    settle(engine, window).then_some(())?;
    let before = window.shown();
    for (pixels, direct) in inputs {
        window.push(engine, *pixels, *direct);
        if !burst {
            frame(engine, window);
        }
    }
    settle(engine, window).then(|| window.shown() - before)
}

/// Exploration: the acceptance scenario's gestures from many scrollbar positions, window
/// sizes and plugin delays.
#[test]
#[ignore = "exploration; prints a travel table"]
fn explore_rust_frontier_travel() {
    let source = rust(6000);
    let mut report = String::new();
    for (size, delay, period, burst, ui_period, wait) in [
        (geometry(1312, 720, 1.0), 0, 1, false, 1, false),
        (geometry(1312, 720, 1.0), 3, 1, true, 1, false),
        (geometry(1312, 720, 1.0), 2, 3, true, 1, false),
        (geometry(1312, 720, 1.0), 12, 6, false, 1, false),
        (geometry(1000, 560, 1.0), 1, 2, true, 1, false),
        (geometry(1640, 900, 1.25), 3, 4, true, 1, false),
        (geometry(2624, 1440, 2.0), 2, 1, false, 1, false),
        (geometry(1312, 720, 1.0), 0, 1, false, 4, true),
        (geometry(1312, 720, 1.0), 0, 1, false, 4, false),
        (geometry(1312, 720, 1.0), 1, 1, true, 6, true),
        (geometry(1312, 720, 1.0), 1, 1, true, 6, false),
        (geometry(1312, 720, 1.0), 2, 1, false, 12, true),
        (geometry(1312, 720, 1.0), 2, 1, false, 12, false),
    ] {
        let mut engine =
            Engine::open(source.clone(), viewer::MODE_SOURCE, size, delay).highlighted();
        engine.period = period;
        engine.ui_period = ui_period;
        engine.prepare_delay = (ui_period > 1).then_some(0);
        let mut window = Window {
            wait_after_refusal: wait,
            ..Window::default()
        };
        rest(&mut engine, &mut window);
        let scale = f64::from(size.scale);
        let screen = f64::from(size.height);
        let notch = NOTCH * scale;
        let arrow = ARROW * scale;
        let back = (screen / 2.0 / arrow).floor() as usize;
        let mut failures = 0;
        for start in 0..40 {
            let position = 0.02 + start as f64 * 0.024;
            window.seek(&mut engine, position);
            rest(&mut engine, &mut window);
            let mut checks: Vec<(String, f64, f64)> = Vec::new();
            let mut stuck = None;
            let mut run = |engine: &mut Engine, window: &mut Window, inputs: &[(f64, bool)]| {
                if stuck.is_some() {
                    return 0.0;
                }
                try_gesture(engine, window, inputs, burst).unwrap_or_else(|| {
                    stuck = Some(format!(
                        "stuck at {:.1} with {:.1} left: {:?}",
                        window.shown(),
                        window.pending,
                        window.log.last()
                    ));
                    0.0
                })
            };
            for (name, inputs) in [
                ("notch down", vec![(notch, false)]),
                ("notch up", vec![(-notch, false)]),
                ("5 notches", vec![(notch, false); 5]),
                ("Down", vec![(arrow, true)]),
                ("Up", vec![(-arrow, true)]),
            ] {
                let expected: f64 = inputs.iter().map(|(pixels, _)| pixels).sum();
                let moved = run(&mut engine, &mut window, &inputs);
                checks.push((name.into(), moved, expected));
            }
            for (name, page, arrow) in [("PageDown", screen, -arrow), ("PageUp", -screen, arrow)] {
                let mut moved = run(&mut engine, &mut window, &[(page, true)]);
                for group in 0..back.div_ceil(4) {
                    let count = (back - group * 4).min(4);
                    moved += run(&mut engine, &mut window, &vec![(arrow, true); count]);
                }
                checks.push((name.into(), moved, page + back as f64 * arrow));
            }
            let moved = run(&mut engine, &mut window, &vec![(arrow, true); 20]);
            checks.push(("20 × Down".into(), moved, 20.0 * arrow));
            if let Some(stuck) = stuck {
                failures += 1;
                let _ = writeln!(
                    report,
                    "{}x{}@{} delay {delay} period {period} burst {burst} ui {ui_period} wait {wait} from {position:.3}: {stuck}",
                    size.width, size.height, size.scale
                );
                break;
            }
            for (name, moved, expected) in checks {
                if (moved - expected).abs() > 1.0 {
                    failures += 1;
                    let _ = writeln!(
                        report,
                        "{}x{}@{} delay {delay} period {period} burst {burst} ui {ui_period} wait {wait} from {position:.3}: {name} moved {moved:.1}, expected {expected:.1}",
                        size.width, size.height, size.scale
                    );
                }
            }
        }
        let _ = writeln!(
            report,
            "{}x{}@{} delay {delay} period {period} burst {burst} ui {ui_period} wait {wait}: {failures} short gestures, reads {} ({} refreshes), refused {}, dropped {:?}, plain frames {}, surfaces {}, rebases {}, edge waits {}",
            size.width,
            size.height,
            size.scale,
            engine.reads,
            engine.refreshes,
            window.refused,
            window.dropped,
            window.plain_frames,
            window.surfaces,
            window.rebases,
            window.edge_waits
        );
    }
    println!("{report}");
}

/// `count` rows of `width` bytes each, newline included.
pub(super) fn rows(count: usize, width: usize) -> String {
    (0..count)
        .map(|row| {
            let prefix = format!("row {row:07} ");
            format!("{prefix}{}\n", "x".repeat(width - prefix.len() - 1))
        })
        .collect()
}

/// A named gesture repeated a number of times.
type Group = (&'static str, Vec<(f64, bool)>, usize);

/// Measurement: plugin reads and bytes per gesture, by row width.
#[test]
#[ignore = "measurement; prints plugin reads per gesture"]
fn measure_page_reads_by_row_width() {
    let mut report = String::new();
    let size = geometry(1312, 720, 1.0);
    for (mode, whole) in [
        (viewer::MODE_TEXT, true),
        (viewer::MODE_TEXT, false),
        (viewer::MODE_SOURCE, true),
        (viewer::MODE_SOURCE, false),
    ] {
        unsent::read_whole(whole);
        let _ = writeln!(report, "-- whole reads {whole}");
        for width in [60, 120, 250, 500, 830, 1200, 2400] {
            let source = rows(3_000_000 / width, width);
            let started = std::time::Instant::now();
            let mut engine = Engine::open(source, mode, size, 1);
            let mut window = Window::default();
            rest(&mut engine, &mut window);
            let mut line = format!(
                "{} rows of {width:>4} bytes: open {} reads in {} frames",
                if mode == viewer::MODE_TEXT {
                    "Text  "
                } else {
                    "Source"
                },
                engine.reads,
                engine.tick
            );
            let mut short = 0;
            let screen = f64::from(size.height);
            let groups: [Group; 4] = [
                ("PageDown", vec![(screen, true)], 4),
                ("5 notches", vec![(NOTCH, false); 5], 4),
                ("20 Down", vec![(ARROW, true); 20], 1),
                ("PageUp", vec![(-screen, true)], 4),
            ];
            let (reads, bytes) = (engine.reads, engine.bytes);
            window.seek(&mut engine, 0.5);
            rest(&mut engine, &mut window);
            let _ = write!(
                line,
                ", seek {} ({} KiB)",
                engine.reads - reads,
                (engine.bytes - bytes) / 1024
            );
            for (name, inputs, repeat) in groups {
                let (reads, bytes, frames) = (engine.reads, engine.bytes, engine.tick);
                for _ in 0..repeat {
                    let expected: f64 = inputs.iter().map(|(pixels, _)| pixels).sum();
                    match try_gesture(&mut engine, &mut window, &inputs, false) {
                        Some(moved) if (moved - expected).abs() <= 1.0 => {}
                        _ => short += 1,
                    }
                }
                let _ = write!(
                    line,
                    ", {repeat}x {name} {} ({} KiB, {} frames)",
                    engine.reads - reads,
                    (engine.bytes - bytes) / 1024,
                    engine.tick - frames
                );
            }
            let _ = writeln!(
                report,
                "{line}; {short} short; plain frames {}; {:.1} s",
                window.plain_frames,
                started.elapsed().as_secs_f64()
            );
        }
    }
    unsent::read_whole(false);
    println!("{report}");
}

/// The acceptance scenario once reported five wheel notches moving 256 of 300 px in a long
/// Rust file. With highlighting prepared while a slow window presents every sixth turn of
/// the worker (its mailbox keeps only the newest frame), the worker restyles its page for
/// the camera the window asked for, which runs the request's lookahead ahead of the camera
/// the window shows. That page starts at the requested camera's row, so its pixels do not
/// reach the shown camera: the window refuses them and, waiting for pixels nothing would
/// paint, stops 44 px short. Asking for the shown camera itself after a refused frame
/// (`raster_camera::Camera::receive` in the viewer window) completes every gesture.
#[test]
fn a_burst_over_a_page_restyled_for_its_lookahead_keeps_its_whole_travel() {
    let source = rust(6000);
    for wait_after_refusal in [true, false] {
        let mut engine = Engine::open(
            source.clone(),
            viewer::MODE_SOURCE,
            geometry(1312, 720, 1.0),
            1,
        )
        .highlighted();
        engine.ui_period = 6;
        engine.prepare_delay = Some(0);
        let mut window = Window {
            wait_after_refusal,
            ..Window::default()
        };
        rest(&mut engine, &mut window);
        window.seek(&mut engine, 0.02);
        rest(&mut engine, &mut window);
        let near = |moved: Option<f64>, expected: f64| {
            moved.is_some_and(|moved| (moved - expected).abs() < 0.01)
        };
        let notch = [(NOTCH, false)];
        let up = [(-NOTCH, false)];
        assert!(near(
            try_gesture(&mut engine, &mut window, &notch, true),
            NOTCH
        ));
        assert!(near(
            try_gesture(&mut engine, &mut window, &up, true),
            -NOTCH
        ));
        let before = window.shown();
        let burst = try_gesture(&mut engine, &mut window, &[(NOTCH, false); 5], true);
        if wait_after_refusal {
            assert_eq!(burst, None, "the burst waits for pixels nothing paints");
            let moved = window.shown() - before;
            assert!(
                (moved - 256.0).abs() < 0.01,
                "the reported 256 of 300 px: {moved}"
            );
            assert!(window.refused > 0);
        } else {
            assert!(
                near(burst, 5.0 * NOTCH),
                "{burst:?}; refused frames: {}",
                window.refused
            );
            for (inputs, expected) in [
                (vec![(720.0, true)], 720.0),
                (vec![(-720.0, true)], -720.0),
                (vec![(ARROW, true); 20], 20.0 * ARROW),
            ] {
                let moved = try_gesture(&mut engine, &mut window, &inputs, true);
                assert!(near(moved, expected), "{moved:?} of {expected}");
            }
        }
    }
}

/// Exploration: short and stuck gestures over long rows, whole reads or reads of new rows,
/// and a window that waits after a refused frame or asks again.
#[test]
#[ignore = "exploration; prints a comparison"]
fn compare_long_row_variants() {
    let size = geometry(1312, 720, 1.0);
    let mut report = String::new();
    for (mode, width) in [
        (viewer::MODE_SOURCE, 1200),
        (viewer::MODE_TEXT, 1200),
        (viewer::MODE_TEXT, 830),
    ] {
        for whole in [true, false] {
            for wait in [true, false] {
                unsent::read_whole(whole);
                let mut engine = Engine::open(rows(3_000_000 / width, width), mode, size, 1);
                let mut window = Window {
                    wait_after_refusal: wait,
                    ..Window::default()
                };
                rest(&mut engine, &mut window);
                window.seek(&mut engine, 0.5);
                rest(&mut engine, &mut window);
                let screen = f64::from(size.height);
                let (mut short, mut stuck) = (0, 0);
                for (inputs, repeat) in [
                    (vec![(screen, true)], 4),
                    (vec![(NOTCH, false); 5], 4),
                    (vec![(ARROW, true); 20], 1),
                    (vec![(-screen, true)], 4),
                ] {
                    let expected: f64 = inputs.iter().map(|(pixels, _)| pixels).sum();
                    for _ in 0..repeat {
                        match try_gesture(&mut engine, &mut window, &inputs, false) {
                            Some(moved) if (moved - expected).abs() > 1.0 => short += 1,
                            Some(_) => {}
                            None => stuck += 1,
                        }
                        if stuck > 0 {
                            break;
                        }
                    }
                }
                let _ = writeln!(
                    report,
                    "mode {mode} rows {width} whole {whole} wait {wait}: {short} short, {stuck} stuck, {} reads, {} refused",
                    engine.reads, window.refused
                );
            }
        }
    }
    unsent::read_whole(false);
    println!("{report}");
}
