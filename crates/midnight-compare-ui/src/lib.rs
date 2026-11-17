//! Presentation and short input callbacks only. The comparator service owns file work.
#![deny(unsafe_code)]
mod file_choice;
mod fonts;
mod images;
#[cfg(test)]
mod wake_tests;
use midnight_compare_engine::{ImageMode, Input, Output, Refused, Service, Session};
use midnight_core::{FileCompareLaunch, ShortcutChord, ViewerAppearance, ViewerGeometry};
use slint::{
    ComponentHandle, EventLoopError, Timer, TimerMode,
    winit_030::{EventResult, WinitWindowAccessor},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::ComparatorWindow;
pub use midnight_ui_input::WindowPlacement;

pub type KeyResolver = Rc<dyn Fn(&ShortcutChord, bool) -> Option<String>>;
pub type FontListener = Rc<dyn Fn(String)>;

pub struct ComparatorWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
    picker_owner: file_choice::PickerOwner,
    font_listener: Rc<RefCell<Option<FontListener>>>,
}
/// One comparison window. It arms no timer while idle (Slint's own text cursor still
/// blinks in a focused file field): the worker's outputs arrive through `output-ready`,
/// geometry through `geometry-changed`, and the batch timer runs only while inputs arrive
/// or wait for room in the queue of a running worker.
struct OpenWindow {
    window: ComparatorWindow,
    session: Session,
    me: std::rc::Weak<OpenWindow>,
    /// Paces input batches and retries inputs the full session queue refused.
    batch: Timer,
    /// Polls the mailbox, only on a platform whose event loop the worker cannot wake.
    poll: Timer,
    closed: Cell<bool>,
    geometry: Cell<(u32, u32, u32)>,
    geometry_revision: Cell<u64>,
    pending: RefCell<Vec<Input>>,
    choice_paths: RefCell<[std::path::PathBuf; 2]>,
    fonts: RefCell<Option<midnight_compare_engine::FontChoices>>,
    requested_font: RefCell<Option<String>>,
    surfaces: RefCell<images::Surfaces>,
}
/// Inputs waiting for room in the bounded session queue.
const MAX_PENDING: usize = 64;
/// Inputs that arrive within this interval after a sent batch wait for the next batch, so
/// continuous input reaches the worker at most about once per display frame. An input
/// after a quiet interval is sent at once.
const BATCH_INTERVAL: Duration = Duration::from_millis(16);

impl OpenWindow {
    fn send(&self, input: Input) {
        if self.closed.get() {
            return;
        }
        queue_input(&mut self.pending.borrow_mut(), input);
        if !self.batch.running() {
            self.flush();
        }
    }
    /// Send queued inputs while the session accepts them, then hold later inputs for one
    /// interval. The timer sends what arrived meanwhile or retries what the full queue
    /// refused; once nothing waits it is not rearmed. A worker that has stopped takes no
    /// more input, so what waits for it is dropped and nothing is armed.
    fn flush(&self) {
        let mut pending = self.pending.borrow_mut();
        let mut refused = None;
        let sent = pending
            .iter()
            .take_while(|input| {
                let result = self.session.send((*input).clone());
                refused = result.err();
                result.is_ok()
            })
            .count();
        if refused == Some(Refused::Stopped) {
            pending.clear();
            return;
        }
        pending.drain(..sent);
        drop(pending);
        let open = self.me.clone();
        self.batch
            .start(TimerMode::SingleShot, BATCH_INTERVAL, move || {
                if let Some(open) = open.upgrade()
                    && !open.closed.get()
                    && !open.pending.borrow().is_empty()
                {
                    open.flush();
                }
            });
    }
    /// Send the content geometry if it changed: on `geometry-changed` and once after `show`.
    fn sync_geometry(&self) {
        if self.closed.get() {
            return;
        }
        let window = &self.window;
        let scale = window.window().scale_factor();
        window.set_compact(window.window().size().to_logical(scale).width < 500.0);
        let geometry = (
            (window.get_content_width() * scale).max(2.0) as u32,
            (window.get_content_height() * scale).max(1.0) as u32,
            scale.to_bits(),
        );
        if self.geometry.replace(geometry) != geometry {
            let revision = self.geometry_revision.get().saturating_add(1);
            self.geometry_revision.set(revision);
            window.set_geometry_revision(revision as i32);
            self.send(Input::Resize(
                ViewerGeometry {
                    width: geometry.0,
                    height: geometry.1,
                    scale,
                },
                revision,
            ));
        }
    }
    /// Present the worker's latest output, if any.
    fn present(&self, service: &Service) {
        if self.closed.get() {
            return;
        }
        let Some(output) = self.session.mailbox.take() else {
            return;
        };
        let window = &self.window;
        match &output {
            Output::Frame { frame, names } => {
                fonts::present(self, frame.fonts.as_ref());
                if window.get_choosing_files() {
                    window.set_choosing_files(false);
                    window.set_choice_busy(false);
                    window.invoke_focus_content();
                }
                window.set_frame_revision(frame.geometry_revision as i32);
                window.set_rendered_mode(match frame.mode {
                    Some(ImageMode::SideBySide) => 0,
                    Some(ImageMode::Swipe) => 1,
                    Some(ImageMode::OnionSkin) => 2,
                    None => -1,
                });
                window.set_left_name(names[0].as_str().into());
                window.set_right_name(names[1].as_str().into());
                window.set_image_mode(frame.image);
                window.set_position(frame.position as f32);
                window.set_status(frame.status.as_str().into());
                window.set_accessible_text(frame.accessible_text.as_str().into());
                if let Some(layers) = &frame.layers {
                    self.surfaces.borrow_mut().present(
                        window,
                        layers,
                        frame.mode == Some(ImageMode::SideBySide),
                    );
                } else {
                    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                        &frame.pixels,
                        frame.width,
                        frame.height,
                    );
                    window.set_surface(slint::Image::from_rgba8_premultiplied(pixels));
                }
            }
            Output::Status(message) => window.set_status(message.as_str().into()),
            Output::ChooseFiles(choice) => self.show_file_choice(choice),
        }
        service.retire(output);
    }
    fn close(&self) {
        if !self.closed.replace(true) {
            self.session.close();
            self.batch.stop();
            self.poll.stop();
            self.window.set_surface(slint::Image::default());
            self.window.set_left_surface(slint::Image::default());
            self.window.set_right_surface(slint::Image::default());
            *self.surfaces.borrow_mut() = images::Surfaces::default();
            self.window.set_accessible_text("".into());
            let _ = self.window.hide();
        }
    }
}

impl ComparatorWindows {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            windows: RefCell::new(Vec::new()),
            picker_owner: |_| midnight_core::PathPickerOwner::None,
            font_listener: Rc::new(RefCell::new(None)),
        }
    }
    /// Persist an explicitly chosen family through the application's preferences backend.
    pub fn set_font_listener(&self, listener: FontListener) {
        *self.font_listener.borrow_mut() = Some(listener);
    }
    /// Reuse the host's platform-specific window identity without performing native I/O here.
    pub fn with_picker_owner(
        mut self,
        owner: fn(&slint::Window) -> midnight_core::PathPickerOwner,
    ) -> Self {
        self.picker_owner = owner;
        self
    }
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
    ) -> Result<slint::Weak<ComparatorWindow>, String> {
        self.windows.borrow_mut().retain(|open| !open.closed.get());
        if self.windows.borrow().len() >= 2 {
            return Err("Close a comparator before opening another (maximum two).".into());
        }
        let window = ComparatorWindow::new().map_err(|error| error.to_string())?;
        let placement = placement.into();
        let size = placement.logical_size();
        window.set_compact(size.width < 500.0);
        placement.apply(window.window());
        window.set_foreground(color(appearance.foreground));
        window.set_background_color(color(appearance.background));
        window.set_accent(color(appearance.accent));
        window.set_ui_font_size(appearance.font_size);
        window.set_ui_font_family(appearance.family.as_str().into());
        window.set_picker_available(self.service.has_path_picker());
        window.invoke_apply_palette(
            appearance.background[..3]
                .iter()
                .map(|channel| u16::from(*channel))
                .sum::<u16>()
                > 384,
        );
        // Runs on the worker: schedules one `output-ready` pass per burst of outputs. It
        // holds only a weak handle, so the worker never keeps the window alive, and a pass
        // that runs after the window is gone does nothing.
        let ready = window.as_weak();
        let session = self.service.open_with_wake(
            launch,
            appearance,
            ViewerGeometry {
                width: size.width.max(2.0) as u32,
                height: (size.height - 100.0).max(1.0) as u32,
                scale: window.window().scale_factor(),
            },
            Arc::new(move || {
                ready
                    .upgrade_in_event_loop(|window| window.invoke_output_ready())
                    .is_ok()
            }),
        )?;
        let open = Rc::new_cyclic(|me| OpenWindow {
            window,
            session,
            me: me.clone(),
            batch: Timer::default(),
            poll: Timer::default(),
            closed: Cell::new(false),
            geometry: Cell::new((0, 0, 0)),
            geometry_revision: Cell::new(0),
            pending: RefCell::new(Vec::new()),
            choice_paths: RefCell::new(Default::default()),
            fonts: RefCell::new(None),
            requested_font: RefCell::new(None),
            surfaces: RefCell::new(images::Surfaces::default()),
        });
        bind(&open, keys, self.service.clone());
        fonts::bind(&open, self.font_listener.clone());
        file_choice::bind(&open, self.picker_owner);
        open.window.show().map_err(|error| error.to_string())?;
        // The size after `show` may equal the size the layout started with, which raises
        // no change; the worker started from an estimate.
        open.sync_geometry();
        let weak = open.window.as_weak();
        // The first presentation pass also shows whether the worker can wake this event
        // loop. A platform without a cross-thread event-loop proxy (a headless test
        // platform) cannot be woken, so there the window polls its mailbox instead.
        if let Err(EventLoopError::NoEventLoopProvider) =
            weak.upgrade_in_event_loop(|window| window.invoke_output_ready())
        {
            let poll = Rc::downgrade(&open);
            let service = self.service.clone();
            open.poll
                .start(TimerMode::Repeated, BATCH_INTERVAL, move || {
                    if let Some(open) = poll.upgrade() {
                        open.present(&service);
                    }
                });
        }
        open.window.invoke_focus_content();
        self.windows.borrow_mut().push(open);
        Ok(weak)
    }
    pub fn close_all(&self) {
        for open in self.windows.borrow_mut().drain(..) {
            open.close();
            self.service.retire(open.session.mailbox.take());
        }
    }
}

fn bind(open: &Rc<OpenWindow>, keys: KeyResolver, service: Service) {
    let weak = Rc::downgrade(open);
    open.window.on_output_ready(move || {
        if let Some(open) = weak.upgrade() {
            open.present(&service);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_geometry_changed(move || {
        if let Some(open) = weak.upgrade() {
            open.sync_geometry();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_close_comparator(move || {
        if let Some(open) = weak.upgrade() {
            open.close();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.window().on_close_requested(move || {
        if let Some(open) = weak.upgrade() {
            open.close();
        }
        slint::CloseRequestResponse::HideWindow
    });
    let weak = Rc::downgrade(open);
    open.window.on_scroll(move |x, y| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Scroll { x, y });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_seek(move |value| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Seek(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_difference(move |backwards| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Difference { backwards });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_mode(move |mode| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Mode(match mode {
                1 => ImageMode::Swipe,
                2 => ImageMode::OnionSkin,
                _ => ImageMode::SideBySide,
            }));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_amount(move |value| {
        if let Some(open) = weak.upgrade()
            && value.is_finite()
        {
            open.window.set_amount(value.clamp(0.0, 1.0));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_zoom(move |value| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Zoom(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_fit(move || {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Fit);
        }
    });
    let native = Rc::new(RefCell::new(midnight_ui_input::ShortcutState::default()));
    let state = native.clone();
    let native_keys = keys.clone();
    let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
    let weak = Rc::downgrade(open);
    open.window.window().on_winit_window_event(move |_, event| {
        let Some(open) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        let scrolled = scroll.observe(open.window.window(), event, None);
        if midnight_ui_input::native_escape_pressed(event) {
            open.close();
            return EventResult::PreventDefault;
        }
        if state.borrow_mut().handle(event, |chord| {
            native_keys(&chord, !open.window.get_content_focused())
                .is_some_and(|action| key(&open, &action))
        }) || scrolled
        {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_input_key(move |event| {
        let Some(open) = weak.upgrade() else {
            return false;
        };
        if midnight_ui_input::escape_pressed(&event) {
            open.close();
            return true;
        }
        native
            .borrow()
            .logical_chord(&event)
            .and_then(|chord| keys(&chord, !open.window.get_content_focused()))
            .is_some_and(|action| key(&open, &action))
    });
}

/// Navigation only moves the view, so a full queue may shed it. Every other input sets
/// state the window already shows (geometry, image mode, font, fit or actual pixels) or
/// answers a file choice that keeps the chooser busy until it is handled.
fn navigation(input: &Input) -> bool {
    matches!(
        input,
        Input::Scroll { .. }
            | Input::Lines(_)
            | Input::Pages(_)
            | Input::Seek(_)
            | Input::Difference { .. }
            | Input::DifferenceEdge { .. }
            | Input::Zoom(_)
    )
}

/// State inputs of one kind: a newer one makes a queued one obsolete.
fn same_state(old: &Input, new: &Input) -> bool {
    matches!(
        (old, new),
        (Input::Resize(..), Input::Resize(..))
            | (Input::Mode(_), Input::Mode(_))
            | (Input::FontFamily(_), Input::FontFamily(_))
            | (
                Input::Fit | Input::ActualPixels,
                Input::Fit | Input::ActualPixels
            )
            | (
                Input::Compare(_) | Input::Browse { .. },
                Input::Compare(_) | Input::Browse { .. }
            )
            | (Input::Close, Input::Close)
    )
}

/// Coalesce repeated inputs and bound the queue without dropping state. A state input
/// replaces a queued one of its kind and is queued last, after the navigation it
/// follows, so at most one of each kind waits and a full queue always holds navigation.
/// The oldest navigation input is shed first.
fn queue_input(pending: &mut Vec<Input>, input: Input) {
    if !navigation(&input) {
        pending.retain(|old| !same_state(old, &input));
        pending.push(input);
    } else if let Some(old) = pending.last_mut().filter(|old| {
        !matches!(
            input,
            Input::Difference { .. } | Input::DifferenceEdge { .. } | Input::Zoom(_)
        ) && std::mem::discriminant(*old) == std::mem::discriminant(&input)
    }) {
        match (&mut *old, input) {
            (Input::Scroll { x, y }, Input::Scroll { x: new_x, y: new_y }) => {
                *x = (*x + new_x).clamp(-262144.0, 262144.0);
                *y = (*y + new_y).clamp(-262144.0, 262144.0);
            }
            // Key repeats accumulate; replacing them would lose steps.
            (Input::Lines(lines), Input::Lines(more))
            | (Input::Pages(lines), Input::Pages(more)) => {
                *lines = lines.saturating_add(more);
            }
            // A newer absolute position replaces the queued one.
            (old, input) => *old = input,
        }
    } else {
        pending.push(input);
    }
    while pending.len() > MAX_PENDING {
        let Some(oldest) = pending.iter().position(navigation) else {
            break;
        };
        pending.remove(oldest);
    }
}

fn key(open: &OpenWindow, action: &str) -> bool {
    // File fields own editing/navigation keys, even if mapped globally for the viewer.
    if open.window.get_choosing_files() && action != "viewer.close" {
        return false;
    }
    let input = match action {
        "viewer.close" => {
            open.close();
            return true;
        }
        "compare.next-difference" => Input::Difference { backwards: false },
        "compare.previous-difference" => Input::Difference { backwards: true },
        "compare.first-difference" => Input::DifferenceEdge { last: false },
        "compare.last-difference" => Input::DifferenceEdge { last: true },
        "viewer.first" => Input::Seek(0.0),
        "viewer.last" => Input::Seek(1.0),
        "viewer.up" => Input::Lines(-1),
        "viewer.down" => Input::Lines(1),
        "viewer.left" => Input::Scroll { x: 48.0, y: 0.0 },
        "viewer.right" => Input::Scroll { x: -48.0, y: 0.0 },
        "viewer.page-up" => Input::Pages(-1),
        "viewer.page-down" => Input::Pages(1),
        "viewer.zoom-in" => Input::Zoom(1.25),
        "viewer.zoom-out" => Input::Zoom(0.8),
        "viewer.fit" => Input::Fit,
        "viewer.actual-pixels" => Input::ActualPixels,
        _ => return false,
    };
    open.send(input);
    true
}
fn color(value: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(value[3], value[0], value[1], value[2])
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn resize(width: u32, revision: u64) -> Input {
        Input::Resize(
            ViewerGeometry {
                width,
                height: 10,
                scale: 1.0,
            },
            revision,
        )
    }

    fn count(pending: &[Input], kind: impl Fn(&Input) -> bool) -> usize {
        pending.iter().filter(|input| kind(input)).count()
    }

    #[test]
    fn a_full_queue_keeps_the_newest_geometry_and_accumulates_key_steps() {
        let mut pending = Vec::new();
        queue_input(&mut pending, resize(100, 1));
        for _ in 0..3 {
            queue_input(&mut pending, Input::Lines(1));
        }
        queue_input(&mut pending, Input::Pages(-1));
        queue_input(&mut pending, Input::Pages(-1));
        assert!(matches!(
            pending[..],
            [Input::Resize(_, 1), Input::Lines(3), Input::Pages(-2)]
        ));
        // A mode click and a font choice made during a long decode, then a held key.
        queue_input(&mut pending, Input::Mode(ImageMode::OnionSkin));
        queue_input(&mut pending, Input::FontFamily("Cascadia Mono".into()));
        for _ in 0..2 * MAX_PENDING {
            queue_input(&mut pending, Input::Zoom(1.25));
        }
        assert_eq!(pending.len(), MAX_PENDING);
        // State inputs survive a navigation flood.
        assert!(
            matches!(pending[0], Input::Resize(_, 1)),
            "{:?}",
            pending[0]
        );
        assert!(matches!(pending[1], Input::Mode(ImageMode::OnionSkin)));
        assert!(matches!(&pending[2], Input::FontFamily(family) if family == "Cascadia Mono"));
        assert_eq!(
            count(&pending, |input| matches!(input, Input::Zoom(_))),
            MAX_PENDING - 3,
            "the oldest navigation is shed first"
        );
        queue_input(&mut pending, resize(200, 2));
        assert_eq!(pending.len(), MAX_PENDING);
        assert!(
            matches!(pending.last(), Some(Input::Resize(geometry, 2)) if geometry.width == 200),
            "the newest geometry is never dropped"
        );
        assert_eq!(
            count(&pending, |input| matches!(input, Input::Resize(..))),
            1
        );
        assert_eq!(count(&pending, |input| matches!(input, Input::Mode(_))), 1);
        assert_eq!(
            count(&pending, |input| matches!(input, Input::FontFamily(_))),
            1
        );
    }

    #[test]
    fn newer_state_replaces_queued_state_of_its_kind_and_stays_after_earlier_navigation() {
        let mut pending = Vec::new();
        let paths = [std::path::PathBuf::from("a.txt"), "b.txt".into()];
        queue_input(&mut pending, Input::Compare(paths.clone()));
        // Alternating state kinds cannot fill the queue: one of each kind waits.
        for round in 0..MAX_PENDING {
            queue_input(&mut pending, Input::Mode(ImageMode::Swipe));
            queue_input(&mut pending, Input::Zoom(2.0));
            queue_input(
                &mut pending,
                if round % 2 == 0 {
                    Input::Fit
                } else {
                    Input::ActualPixels
                },
            );
            queue_input(&mut pending, Input::FontFamily(format!("Mono {round}")));
        }
        assert_eq!(pending.len(), MAX_PENDING);
        assert!(
            matches!(&pending[0], Input::Compare(queued) if *queued == paths),
            "a queued file choice is never dropped"
        );
        assert_eq!(count(&pending, |input| !navigation(input)), 4);
        // The last fit choice follows the zoom steps queued before it.
        let fit = pending
            .iter()
            .position(|input| matches!(input, Input::Fit | Input::ActualPixels))
            .unwrap();
        assert!(matches!(pending[fit], Input::ActualPixels));
        assert!(matches!(pending[fit - 1], Input::Zoom(_)));
        let last = format!("Mono {}", MAX_PENDING - 1);
        assert!(matches!(pending.last(), Some(Input::FontFamily(family)) if *family == last));
        // Consecutive absolute positions keep only the newest.
        let mut pending = vec![resize(100, 1)];
        queue_input(&mut pending, Input::Mode(ImageMode::Swipe));
        queue_input(&mut pending, Input::Seek(0.25));
        queue_input(&mut pending, Input::Seek(0.75));
        assert_eq!(pending.len(), 3);
        assert!(matches!(pending[2], Input::Seek(position) if position == 0.75));
    }
}
