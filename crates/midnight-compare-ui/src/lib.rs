//! Presentation and short input callbacks only. The comparator service owns file work.
#![deny(unsafe_code)]
#[cfg(feature = "acceptance-seams")]
pub mod acceptance_seams;
mod admission;
#[cfg(test)]
mod admission_tests;
mod appearance;
#[cfg(test)]
mod external_outcome_tests;
#[cfg(test)]
mod external_retry_tests;
mod file_choice;
mod find;
mod fonts;
#[cfg(test)]
mod identity_tests;
mod images;
mod presentation;
#[cfg(all(test, unix))]
#[path = "external_outcome_unix_tests.rs"]
mod raw_paths;
#[cfg(all(test, windows))]
#[path = "external_outcome_windows_tests.rs"]
mod raw_paths;
#[cfg(test)]
mod scroll_parity_tests;
#[cfg(test)]
mod scroll_tests;
mod text_view;
// The shared integration-test support names this crate by its public path.
#[cfg(test)]
extern crate self as midnight_compare_ui;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support;
mod tooltip;
#[cfg(test)]
mod wake_tests;
use midnight_compare_engine::{
    ExternalCompareChoiceOutcome, ImageMode, Input, Refused, Service, Session, Wake,
};
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
/// The window's native event observer: what `on_winit_window_event` calls, and the
/// acceptance seam too (`acceptance_seams`).
type NativeObserver =
    Rc<RefCell<dyn FnMut(&slint::winit_030::winit::event::WindowEvent) -> EventResult>>;
pub type FontListener = Rc<dyn Fn(String)>;

pub struct ComparatorWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
    picker_owner: file_choice::PickerOwner,
    font_listener: Rc<RefCell<Option<FontListener>>>,
    appearance: RefCell<Option<appearance::Presentation>>,
    scroll_preferences: ScrollPreferences,
    #[cfg(test)]
    poll_outputs: bool,
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
    tooltip_observer: RefCell<Option<midnight_ui_input::PostWindowEventObserver>>,
    geometry: Cell<(u32, u32, u32)>,
    geometry_revision: Cell<u64>,
    comparison_generation: Cell<u64>,
    find_revision: Cell<u64>,
    submitted_find: RefCell<Option<midnight_compare_engine::FindQuery>>,
    comparison_requested: Cell<bool>,
    external_outcome: Cell<Option<ExternalCompareChoiceOutcome>>,
    pending: RefCell<Vec<Input>>,
    choice_paths: RefCell<[std::path::PathBuf; 2]>,
    fonts: RefCell<Option<midnight_compare_engine::FontChoices>>,
    requested_font: RefCell<Option<String>>,
    /// A loaded comparison font that waits for the font catalog (`fonts::restore`).
    restored_font: RefCell<Option<String>>,
    surfaces: RefCell<images::Surfaces>,
    appearance: appearance::PaletteState,
    /// The compared text's view (`text_view`): scrolling moves it at once.
    view: RefCell<text_view::TextView>,
    /// Gives the worker the scrolling of a view that came to rest (`SETTLE`).
    settle: Timer,
    /// The host's sources of the system's wheel setting and of its wish for less motion.
    scroll_preferences: ScrollPreferences,
    #[cfg(feature = "acceptance-seams")]
    seam: acceptance_seams::Seam,
}
/// Inputs waiting for room in the bounded session queue.
const MAX_PENDING: usize = 64;
/// Inputs that arrive within this interval after a sent batch wait for the next batch, so
/// continuous input reaches the worker at most about once per display frame. An input
/// after a quiet interval is sent at once.
const BATCH_INTERVAL: Duration = Duration::from_millis(16);
/// A view that has not moved for this long is painted at its place: the worker is sent
/// the scrolling that moved it. Until then it moves over the painted margin.
const SETTLE: Duration = Duration::from_millis(50);

/// The host's readings of the system's scroll settings, as the viewer windows take them
/// (`ViewerWindows::set_motion_preference`): the wheel setting (Windows: lines per notch)
/// and whether the system asks for less motion. Read with a window's first native event
/// and whenever it gains the focus.
#[derive(Clone)]
struct ScrollPreferences {
    wheel: Rc<RefCell<Rc<dyn Fn() -> midnight_ui_input::WheelSetting>>>,
    reduce_motion: Rc<RefCell<Rc<dyn Fn() -> bool>>>,
}

impl Default for ScrollPreferences {
    fn default() -> Self {
        Self {
            wheel: Rc::new(RefCell::new(Rc::new(
                midnight_ui_input::WheelSetting::default,
            ))),
            reduce_motion: Rc::new(RefCell::new(Rc::new(|| false))),
        }
    }
}

impl ScrollPreferences {
    fn apply(&self, scroll: &mut midnight_ui_input::NativeScroll) {
        let wheel = self.wheel.borrow().clone();
        let reduce = self.reduce_motion.borrow().clone();
        scroll.set_wheel_setting(wheel());
        scroll.set_reduce_motion(reduce());
    }
}

impl OpenWindow {
    fn new(
        window: ComparatorWindow,
        session: Session,
        appearance: &ViewerAppearance,
        scroll_preferences: ScrollPreferences,
    ) -> Rc<Self> {
        let appearance = appearance::PaletteState::new(appearance, session.palette_revision());
        Rc::new_cyclic(|me| OpenWindow {
            window,
            session,
            me: me.clone(),
            batch: Timer::default(),
            poll: Timer::default(),
            closed: Cell::new(false),
            tooltip_observer: RefCell::new(None),
            geometry: Cell::new((0, 0, 0)),
            geometry_revision: Cell::new(0),
            comparison_generation: Cell::new(0),
            find_revision: Cell::new(0),
            submitted_find: RefCell::new(None),
            comparison_requested: Cell::new(false),
            external_outcome: Cell::new(None),
            pending: RefCell::new(Vec::new()),
            choice_paths: RefCell::new(Default::default()),
            fonts: RefCell::new(None),
            requested_font: RefCell::new(None),
            restored_font: RefCell::new(None),
            surfaces: RefCell::new(images::Surfaces::default()),
            appearance,
            view: RefCell::new(text_view::TextView::default()),
            settle: Timer::default(),
            scroll_preferences,
            #[cfg(feature = "acceptance-seams")]
            seam: acceptance_seams::Seam::default(),
        })
    }
    fn send(&self, input: Input) -> bool {
        if self.closed.get() {
            return false;
        }
        let compare = matches!(&input, Input::Compare(_));
        let choosing = compare || matches!(&input, Input::Browse { .. });
        if choosing {
            // Explicit choices use Session admission and are never retained for replay.
            if self.session.send(input).is_err() {
                return false;
            }
            self.external_outcome.set(None);
            if compare {
                self.reset_find();
                self.comparison_requested.set(true);
                self.window.set_identity_state(-1);
                self.window.set_status("Preparing comparison…".into());
            }
            return true;
        }
        // The view's scrolling goes first: the worker applies inputs in order.
        if !matches!(input, Input::Scroll { .. }) {
            self.queue_unsent();
        }
        queue_input(&mut self.pending.borrow_mut(), input);
        if !self.batch.running() {
            self.flush();
        }
        true
    }
    /// Queue the scrolling that moved the view here, for the worker to paint around it.
    fn queue_unsent(&self) {
        self.settle.stop();
        if let Some((x, y)) = self.view.borrow_mut().take() {
            queue_input(&mut self.pending.borrow_mut(), Input::Scroll { x, y });
        }
    }
    /// Give the worker the view's scrolling now.
    fn send_unsent(&self) {
        if self.closed.get() {
            return;
        }
        self.queue_unsent();
        if !self.batch.running() && !self.pending.borrow().is_empty() {
            self.flush();
        }
    }
    /// Present the view where it stands: both panes' clips in the painted surface, the
    /// position bar and where touchpad momentum stands.
    fn show_view(&self) {
        let shown = self
            .view
            .borrow()
            .shown(queued_scroll(&self.pending.borrow()));
        let window = &self.window;
        window.set_text_view(shown.is_some());
        let Some(shown) = shown else {
            return;
        };
        window.set_view_left_x(shown.x[0]);
        window.set_view_right_x(shown.x[1]);
        window.set_view_left_width(shown.width[0]);
        window.set_view_right_width(shown.width[1]);
        window.set_view_y(shown.y);
        window.set_view_height(shown.height);
        window.set_view_split(shown.split);
        window.set_position((shown.position[1] / shown.extent[1].max(1.0)) as f32);
        window.set_shown_horizontal(shown.position[0] as f32);
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
            .take_while(|input| match self.session.send((*input).clone()) {
                Ok(()) => {
                    if let Input::Scroll { x, y } = input {
                        self.view.borrow_mut().sent(*x, *y);
                    }
                    true
                }
                Err(Refused::Rejected) => true,
                Err(error) => {
                    refused = Some(error);
                    false
                }
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
        let logical = window.window().size().to_logical(scale);
        tooltip::synchronize(self);
        window.set_compact(logical.width < 500.0);
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
        self.present_output(&output);
        service.retire(output);
    }
    fn close(&self) {
        if !self.closed.replace(true) {
            self.tooltip_observer.borrow_mut().take();
            self.reset_find();
            self.external_outcome.set(None);
            self.comparison_requested.set(false);
            self.window.set_identity_state(-1);
            self.window.set_status("".into());
            self.session.close();
            self.batch.stop();
            self.poll.stop();
            self.settle.stop();
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
            appearance: RefCell::new(None),
            scroll_preferences: ScrollPreferences::default(),
            #[cfg(test)]
            poll_outputs: false,
        }
    }
    /// A headless fixture may run alongside the process-wide event-loop proxy test.
    #[cfg(test)]
    fn new_polled(service: Service) -> Self {
        Self {
            poll_outputs: true,
            ..Self::new(service)
        }
    }
    /// The system's wheel setting (Windows: lines per notch), as the host reads it. Each
    /// window asks with its first native event and whenever it gains the focus; a wheel
    /// notch then moves the distance the text viewer moves for it. Without a source, the
    /// platform default.
    pub fn set_wheel_setting(&self, source: Rc<dyn Fn() -> midnight_ui_input::WheelSetting>) {
        *self.scroll_preferences.wheel.borrow_mut() = source;
    }
    /// Whether the system asks for less motion (Windows: animation effects off), as the
    /// host reads it, asked like the wheel setting: wheel notches then move at once and no
    /// touchpad momentum is synthesised, as in the viewer windows. Without a source,
    /// motion is full.
    pub fn set_motion_preference(&self, source: Rc<dyn Fn() -> bool>) {
        *self.scroll_preferences.reduce_motion.borrow_mut() = source;
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
        self.open_with_observer(launch, appearance, placement, keys, |_| {})
    }
    fn open_with_observer(
        &self,
        launch: FileCompareLaunch,
        mut appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
        before_palette_read: impl FnOnce(&Session),
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
        let presentation = self
            .appearance
            .borrow()
            .clone()
            .unwrap_or_else(|| appearance::Presentation::legacy(&appearance));
        presentation.apply_window(&window);
        presentation.opening_palette(&mut appearance);
        window.set_picker_available(self.service.has_path_picker());
        // The worker wakes the event loop once per output burst using a weak window handle.
        let ready = window.as_weak();
        #[cfg(test)]
        let force_poll = self.poll_outputs;
        #[cfg(not(test))]
        let force_poll = false;
        let wake: Wake = Arc::new(move || {
            !force_poll
                && ready
                    .upgrade_in_event_loop(|window| window.invoke_output_ready())
                    .is_ok()
        });
        let session = admission::open_with_wake(
            &self.service,
            launch,
            appearance.clone(),
            ViewerGeometry {
                width: size.width.max(2.0) as u32,
                height: (size.height - 100.0).max(1.0) as u32,
                scale: window.window().scale_factor(),
            },
            wake,
        )
        .map_err(|error| error.to_string())?;
        before_palette_read(&session);
        let open = OpenWindow::new(
            window,
            session,
            &appearance,
            self.scroll_preferences.clone(),
        );
        open.appearance.presented(presentation);
        bind(&open, keys, self.service.clone());
        fonts::bind(&open, self.font_listener.clone());
        file_choice::bind(&open, self.picker_owner);
        *open.tooltip_observer.borrow_mut() =
            Some(tooltip::install(&open).map_err(|error| error.to_string())?);
        open.window.show().map_err(|error| error.to_string())?;
        // The size after `show` may equal the size the layout started with, which raises
        // no change; the worker started from an estimate.
        open.sync_geometry();
        let weak = open.window.as_weak();
        // The first presentation pass also shows whether the worker can wake this event
        // loop. A platform without a cross-thread event-loop proxy (a headless test
        // platform) cannot be woken, so there the window polls its mailbox instead.
        if force_poll
            || matches!(
                weak.upgrade_in_event_loop(|window| window.invoke_output_ready()),
                Err(EventLoopError::NoEventLoopProvider)
            )
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
    /// Apply the comparison font that the settings read loaded to the open windows, as
    /// viewer windows restore their preferences: nothing is saved and the font listener is
    /// not called. A window still waiting for its font catalog applies it with the catalog.
    pub fn restore_font(&self, family: &str) {
        for open in self
            .windows
            .borrow()
            .iter()
            .filter(|open| !open.closed.get())
        {
            fonts::restore(open, family);
        }
    }
}

/// The compared content under `point`, unless a popup owns the input.
fn on_content(window: &ComparatorWindow, point: slint::LogicalPosition) -> bool {
    let (left, top) = (window.get_content_left(), window.get_content_top());
    point.x >= left
        && point.y >= top
        && point.x < left + window.get_content_width()
        && point.y < top + window.get_content_height()
        && !midnight_ui_input::popup_open(window.window())
}

/// The window's shared wheel/touchpad input: compared content takes pixel deltas
/// without easing of its own, so wheel notches glide there, and touchpad momentum
/// continues there (only), stopping when the text position stops moving.
fn native_scroll(window: &ComparatorWindow) -> midnight_ui_input::NativeScroll {
    let weak = window.as_weak();
    let mut scroll = midnight_ui_input::NativeScroll::new(window)
        .lists_at_once()
        .glide_content(Box::new(move |point| {
            weak.upgrade()
                .is_some_and(|window| on_content(&window, point))
        }));
    let contains = window.as_weak();
    let position = window.as_weak();
    scroll.set_kinetic_surface(midnight_ui_input::KineticSurface {
        contains: Box::new(move |point| {
            contains
                .upgrade()
                .is_some_and(|window| on_content(&window, point))
        }),
        position: Box::new(move || {
            position
                .upgrade()
                .filter(|window| !window.get_image_mode())
                .map_or([None; 2], |window| {
                    momentum_position(
                        f64::from(window.get_shown_horizontal()),
                        f64::from(window.get_position()),
                    )
                })
        }),
    });
    scroll
}

/// Where the text stands for touchpad momentum, which stops an axis whose position stops
/// changing: given only at an edge (the left edge, the start or the end), as the text
/// viewer gives it. Between the edges the view may wait at the end of the painted margin
/// for the worker's next surface, and momentum must not take that for the text's end.
fn momentum_position(horizontal: f64, vertical: f64) -> [Option<f64>; 2] {
    const EDGE: f64 = 1e-6;
    [
        (horizontal <= EDGE).then_some(horizontal),
        (vertical <= EDGE || vertical >= 1.0 - EDGE).then_some(vertical),
    ]
}

fn bind(open: &Rc<OpenWindow>, keys: KeyResolver, service: Service) {
    let weak = Rc::downgrade(open);
    open.window.on_search(move |backwards| {
        if let Some(open) = weak.upgrade() {
            open.find(backwards);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_search_edited(move || {
        if let Some(open) = weak.upgrade() {
            open.clear_find();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_clear_search(move || {
        if let Some(open) = weak.upgrade() {
            open.clear_find();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_search_options_changed(move || {
        if let Some(open) = weak.upgrade() {
            open.clear_find();
            open.find(false);
        }
    });
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
            scroll_content(&open, x, y);
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
    open.window.on_show_bytes(move |show| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Bytes(show));
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
    let mut scroll = native_scroll(&open.window);
    let weak = Rc::downgrade(open);
    let settled = Rc::downgrade(open);
    open.settle.start(TimerMode::SingleShot, SETTLE, move || {
        if let Some(open) = settled.upgrade() {
            open.send_unsent();
        }
    });
    open.settle.stop();
    let mut preferences_read = false;
    let observe = move |event: &slint::winit_030::winit::event::WindowEvent| {
        let Some(open) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        if let slint::winit_030::winit::event::WindowEvent::Focused(active) = event {
            appearance::activation_changed(&open, *active);
        }
        // The system's settings: read with the first native event and on each focus gain
        // (the person changes them in another window), never per notch.
        if !preferences_read
            || matches!(
                event,
                slint::winit_030::winit::event::WindowEvent::Focused(true)
            )
        {
            preferences_read = true;
            open.scroll_preferences.apply(&mut scroll);
        }
        scroll.set_content_page([
            open.window.get_content_width(),
            open.window.get_content_height(),
        ]);
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
    };
    let observer: NativeObserver = Rc::new(RefCell::new(observe));
    #[cfg(feature = "acceptance-seams")]
    open.seam.observer.replace(Some(observer.clone()));
    open.window.window().on_winit_window_event(move |_, event| {
        // Winit delivers one event at a time: a nested one would find the observer busy.
        observer
            .try_borrow_mut()
            .map_or(EventResult::Propagate, |mut observe| (*observe)(event))
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

/// A wheel glide frame, touchpad, pointer-drag or scroll-key delta over the compared
/// content (negative moves down or right). Text and bytes move at once (`text_view`); the
/// worker is sent the scrolling when the view nears the painted margin's end or comes to
/// rest. An image comparison's worker pans it.
fn scroll_content(open: &OpenWindow, x: f32, y: f32) {
    let queued = queued_scroll(&open.pending.borrow());
    if !open.view.borrow_mut().scroll(x, y, queued) {
        open.send(Input::Scroll { x, y });
        return;
    }
    open.show_view();
    if open.view.borrow().due(queued) {
        open.send_unsent();
    } else {
        open.settle.restart();
    }
}

/// The scrolling waiting in the window's input queue, summed per axis as it will be sent.
fn queued_scroll(pending: &[Input]) -> [f64; 2] {
    pending.iter().fold([0.0; 2], |sum, input| match input {
        Input::Scroll { x, y } => [sum[0] + f64::from(*x), sum[1] + f64::from(*y)],
        _ => sum,
    })
}

/// Navigation only moves the view, so a full queue may shed it. Every other input sets
/// state the window already shows (geometry, image mode, Hex view, font, fit or actual
/// pixels).
/// Explicit file choices use session admission directly.
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
            | (Input::Bytes(_), Input::Bytes(_))
            | (Input::FontFamily(_), Input::FontFamily(_))
            | (Input::Find { .. }, Input::Find { .. })
            | (Input::FindStep { .. }, Input::FindStep { .. })
            | (
                Input::Fit | Input::ActualPixels,
                Input::Fit | Input::ActualPixels
            )
            | (Input::Close, Input::Close)
    )
}

/// Coalesce repeated inputs and bound the queue without dropping state. A state input
/// replaces a queued one of its kind and is queued last, after the navigation it
/// follows, so at most one of each kind waits and a full queue always holds navigation.
/// The oldest navigation input is shed first.
fn queue_input(pending: &mut Vec<Input>, input: Input) {
    let input = match input {
        Input::FindStep {
            mut delta,
            revision,
        } => {
            if let Some(index) = pending
                .iter()
                .position(|old| matches!(old, Input::FindStep { .. }))
                && let Input::FindStep { delta: old, .. } = pending.remove(index)
            {
                delta = delta.saturating_add(old);
            }
            Input::FindStep { delta, revision }
        }
        input @ Input::Find { .. } => {
            // Replacing/clearing a query invalidates every relative step for the old query.
            pending.retain(|old| !matches!(old, Input::FindStep { .. }));
            input
        }
        input => input,
    };
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
        "viewer.find" if !open.window.get_image_mode() && !open.window.get_bytes_mode() => {
            open.window.invoke_focus_search();
            return true;
        }
        "viewer.find-next" | "viewer.find-previous"
            if !open.window.get_image_mode() && !open.window.get_bytes_mode() =>
        {
            open.find(action == "viewer.find-previous");
            return true;
        }
        "compare.next-difference" => Input::Difference { backwards: false },
        "compare.previous-difference" => Input::Difference { backwards: true },
        "compare.first-difference" => Input::DifferenceEdge { last: false },
        "compare.last-difference" => Input::DifferenceEdge { last: true },
        "viewer.first" => Input::Seek(0.0),
        "viewer.last" => Input::Seek(1.0),
        // Text and bytes: the text viewer's distances, at once (`text_view`).
        action
            if open.view.borrow().active()
                && let Some((x, y)) =
                    midnight_ui_input::key_scroll(action, open.window.get_content_height()) =>
        {
            scroll_content(open, -x, -y);
            return true;
        }
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

/// The compared files' paths as the window shows them: explicit bidirectional controls
/// become visible escapes, so a name cannot reorder its displayed extension. The worker
/// keeps comparing the captured files.
fn show_names(window: &ComparatorWindow, names: &[String; 2]) {
    window.set_left_name(midnight_core::visible_name(&names[0]).as_ref().into());
    window.set_right_name(midnight_core::visible_name(&names[1]).as_ref().into());
}

/// Window presentation on a platform without an event-loop proxy: the wake test owns the
/// process-wide one, so no comparator here may wake an event loop.
#[cfg(test)]
mod window_tests {
    use super::*;
    use slint::platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    };

    struct TestPlatform;
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }

    #[test]
    fn compared_paths_show_bidi_controls_as_escapes() {
        slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
        let window = ComparatorWindow::new().unwrap();
        // Shown raw, the override control makes the name read "photoexe.jpg".
        show_names(
            &window,
            &[
                "C:\\in\\photo\u{202e}gpj.exe".into(),
                "C:\\in\\plain.txt".into(),
            ],
        );
        assert_eq!(window.get_left_name(), "C:\\in\\photo\\u202Egpj.exe");
        assert_eq!(window.get_right_name(), "C:\\in\\plain.txt");
    }

    /// The font families of the inputs the window sent or holds for its worker.
    fn fonts_sent(open: &OpenWindow) -> Vec<String> {
        open.pending
            .borrow()
            .iter()
            .filter_map(|input| match input {
                Input::FontFamily(family) => Some(family.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_restored_font_reaches_the_worker_with_its_catalog_and_is_not_saved() {
        slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
        let runtime = super::identity_tests::runtime();
        let service = Service::new(runtime.handle().clone(), runtime.spawner());
        let appearance = ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [240, 240, 240, 255],
            background: [20, 20, 20, 255],
            selection: [60, 80, 120, 255],
            selection_text: [0; 4],
            accent: [100, 150, 220, 255],
        };
        let policy = service.comparator_snapshot();
        // A worker for the file form. Its outputs are never presented here, and it has no
        // wake, so it cannot reach the process-wide event-loop proxy.
        let session = service
            .open_with_wake(
                FileCompareLaunch {
                    sources: Vec::new(),
                    disabled_plugins: Arc::from([]),
                    comparators: policy.preferences,
                    comparator_policy: policy.id,
                },
                appearance.clone(),
                ViewerGeometry {
                    width: 200,
                    height: 100,
                    scale: 1.0,
                },
                Arc::new(|| false),
            )
            .unwrap();
        let open = OpenWindow::new(
            ComparatorWindow::new().unwrap(),
            session,
            &appearance,
            ScrollPreferences::default(),
        );
        let saved = Rc::new(Cell::new(0));
        let counted = saved.clone();
        let listener: FontListener = Rc::new(move |_| counted.set(counted.get() + 1));
        fonts::bind(&open, Rc::new(RefCell::new(Some(listener))));
        let manager = ComparatorWindows::new(service);
        manager.windows.borrow_mut().push(open.clone());
        // Inputs right after a sent batch wait for the next one, so what the window sends
        // stays in its queue; the batch timer never fires without a timer update.
        open.flush();
        assert!(open.batch.running());

        // The worker ignores a font change before its catalog: the window keeps the family
        // and sends it once the catalog arrives.
        manager.restore_font("Mono B");
        assert!(fonts_sent(&open).is_empty());
        let families: Arc<[String]> = Arc::from(["Mono A".to_owned(), "Mono B".to_owned()]);
        let catalog = |selected: &str| midnight_compare_engine::FontChoices {
            families: families.clone(),
            selected: selected.into(),
        };
        fonts::present(&open, Some(&catalog("Mono A")));
        assert_eq!(fonts_sent(&open), ["Mono B"]);
        assert_eq!(
            open.window.get_font_index(),
            1,
            "the choice follows the worker, which still paints Mono A"
        );
        fonts::present(&open, Some(&catalog("Mono B")));
        assert_eq!(open.window.get_font_index(), 2);

        // With the catalog known a restore is sent at once, unless the worker already
        // paints the family or the catalog does not offer it.
        open.pending.borrow_mut().clear();
        manager.restore_font("Mono B");
        manager.restore_font("Missing Mono");
        assert!(fonts_sent(&open).is_empty());
        manager.restore_font("monospace");
        assert_eq!(fonts_sent(&open), ["monospace"]);
        fonts::present(&open, Some(&catalog("monospace")));
        assert_eq!(open.window.get_font_index(), 0);
        assert_eq!(saved.get(), 0, "restoring saves nothing");
        manager.close_all();
    }
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
        queue_input(&mut pending, resize(100, 1));
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
        assert!(matches!(pending[0], Input::Resize(_, 1)));
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
