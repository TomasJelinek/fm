//! Smooth the pinned Fluent scrollbar wheel path and the horizontally remapped
//! command prompt, and synthesise touchpad momentum for the whole window. Other
//! content keeps Slint's own wheel easing unless the host opts into content glide.
use crate::kinetic_native::{LINE_PIXELS, NativeKinetic, WheelSource};
use crate::scroll_press::PressState;
use crate::{WHEEL_FRAME, WheelMotion};
use i_slint_core::{
    item_tree::{ItemRc, ItemWeak, ParentItemTraversalMode},
    items::{Flickable, TouchArea},
    lengths::{LogicalPoint, LogicalRect},
    window::{WindowInner, WindowKind},
};
use slint::{
    ComponentHandle,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            dpi::PhysicalPosition,
            event::{DeviceId, ElementState, MouseScrollDelta, TouchPhase, WindowEvent},
        },
    },
};
use std::time::Duration;

// Slint Fluent 1.18.1 scrollview.slint: the bar's TouchArea fills its 14px
// overlay. Verify that actual visible item too; an overflow alone does not imply
// a scrollbar when its policy is always-off.
const OVERLAY: f32 = 14.0;

/// Bounds and identity of the app's vertically remapped horizontal command strip.
#[derive(Clone, PartialEq)]
pub struct CommandStrip {
    pub bounds: LogicalRect,
    pub path: slint::SharedString,
}

#[derive(PartialEq)]
enum Target {
    // The native adapter owns this observer; strong item handles would retain
    // its component and form a component -> adapter -> hook -> component cycle.
    Scrollbar {
        scope: ItemWeak,
        bar: ItemWeak,
        viewport: ItemWeak,
        horizontal: bool,
    },
    Command(CommandStrip),
    /// Any content of a window that opted into [`NativeScroll::glide_content`].
    Content {
        scope: ItemWeak,
    },
}

/// What momentum scrolls, resolved once per fling instead of on every frame.
enum KineticView {
    /// The host's [`KineticSurface`].
    Surface,
    /// A Flickable that scrolls along the fling; `bar` is its Fluent scrollbar
    /// when the gesture happened on it.
    Flickable {
        flick: ItemWeak,
        bar: Option<Target>,
    },
}

// Pinned Winit has no create_child_window_adapter override, so SDK ComboBox
// and custom popups are embedded and deliver input to this owning window.
fn scope(window: &slint::Window) -> (ItemRc, bool) {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    if let Some(popup) = popups
        .iter()
        .rev()
        .find(|p| !matches!(p.window_kind, WindowKind::ToolTip))
    {
        (ItemRc::new_root(popup.component.clone()), true)
    } else {
        (ItemRc::new_root(inner.component()), false)
    }
}

/// An open popup other than a tooltip (an SDK combo box list, a menu) owns pointer
/// input, so content behind it must not scroll.
pub fn popup_open(window: &slint::Window) -> bool {
    WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
}

fn bounds(item: &ItemRc) -> LogicalRect {
    let rect = item.geometry();
    // Pinned ItemRc mapping composes parents only; add this item's own origin.
    LogicalRect::new(item.map_to_native_window(rect.origin), rect.size)
}

fn contains(item: &ItemRc, position: LogicalPoint) -> bool {
    if !item.is_visible() || !bounds(item).contains(position) {
        return false;
    }
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.borrow().as_ref().clips_children() && !bounds(&item).contains(position) {
            return false;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    true
}

fn collect(item: ItemRc, position: LogicalPoint, views: &mut Vec<ItemRc>, bars: &mut Vec<ItemRc>) {
    // The pinned compiler lowers visible:false to an active zero-sized Clip.
    // This also prunes hidden branches; zero-sized nonclipping wrappers may
    // still contain visible children, so is_visible alone cannot prune them.
    if item.borrow().as_ref().clips_children() && !bounds(&item).contains(position) {
        return;
    }
    if item.downcast::<Flickable>().is_some() && contains(&item, position) {
        views.push(item.clone());
    } else if let Some(area) = item.downcast::<TouchArea>() {
        let rect = item.geometry();
        if area.as_pin_ref().enabled()
            && ((rect.width() - OVERLAY).abs() < 0.1 || (rect.height() - OVERLAY).abs() < 0.1)
            && contains(&item, position)
        {
            bars.push(item.clone());
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        collect(item, position, views, bars);
    }
}

impl Target {
    fn content(window: &slint::Window) -> Self {
        Self::Content {
            scope: scope(window).0.downgrade(),
        }
    }

    fn at(
        window: &slint::Window,
        position: LogicalPoint,
        command: Option<&CommandStrip>,
    ) -> Option<Self> {
        let (scope, popup) = scope(window);
        if !popup
            && let Some(command) = command
            && command.bounds.contains(position)
        {
            return Some(Self::Command(command.clone()));
        }
        let mut views = Vec::new();
        let mut bars = Vec::new();
        collect(scope.clone(), position, &mut views, &mut bars);
        for viewport in views.into_iter().rev() {
            let area = bounds(&viewport);
            let flick = viewport.downcast::<Flickable>().unwrap();
            for bar in bars.iter().rev() {
                let rect = bounds(bar);
                let vertical = flick.as_pin_ref().content_height().get() > area.height()
                    && (rect.width() - OVERLAY).abs() < 0.1
                    && (rect.max_x() - area.max_x()).abs() < 0.1
                    && (rect.min_y() - area.min_y()).abs() < 0.1
                    && (rect.height() - area.height()).abs() <= OVERLAY + 0.1;
                let horizontal = flick.as_pin_ref().content_width().get() > area.width()
                    && (rect.height() - OVERLAY).abs() < 0.1
                    && (rect.max_y() - area.max_y()).abs() < 0.1
                    && (rect.min_x() - area.min_x()).abs() < 0.1
                    && (rect.width() - area.width()).abs() <= OVERLAY + 0.1;
                if vertical || horizontal {
                    return Some(Self::Scrollbar {
                        scope: scope.downgrade(),
                        bar: bar.downgrade(),
                        viewport: viewport.downgrade(),
                        horizontal,
                    });
                }
            }
        }
        None
    }

    fn valid(
        &self,
        window: &slint::Window,
        position: LogicalPoint,
        command: Option<&CommandStrip>,
    ) -> bool {
        let (current, popup) = scope(window);
        match self {
            Self::Command(saved) => {
                !popup && command == Some(saved) && saved.bounds.contains(position)
            }
            Self::Content { scope } => current.downgrade() == *scope,
            Self::Scrollbar {
                scope,
                bar,
                viewport,
                ..
            } => {
                current.downgrade() == *scope
                    && bar.upgrade().is_some_and(|bar| contains(&bar, position))
                    && viewport
                        .upgrade()
                        .is_some_and(|viewport| contains(&viewport, position))
            }
        }
    }

    fn accepts(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Scrollbar {
                horizontal: true, ..
            } => x != 0.0,
            Self::Scrollbar {
                horizontal: false, ..
            } => y != 0.0,
            Self::Command(_) | Self::Content { .. } => x != 0.0 || y != 0.0,
        }
    }

    fn scrolled(&self) {
        if let Self::Scrollbar { viewport, .. } = self
            && let Some(viewport) = viewport.upgrade()
        {
            // Fluent's wheel handler writes the bound value but omits scrolled.
            // Notify the owning list, just as content motion and thumb dragging do.
            (Flickable::FIELD_OFFSETS.flicked())
                .apply_pin(viewport.downcast::<Flickable>().unwrap().as_pin_ref())
                .call(&());
        }
    }
}

/// Release list cursor-following before forwarding native input to a Fluent bar.
/// This emits the SDK's omitted notification without changing the wheel delta.
pub fn notify_scrollbar_wheel(window: &slint::Window, position: LogicalPoint, x: f32, y: f32) {
    if x.is_finite()
        && y.is_finite()
        && (x != 0.0 || y != 0.0)
        && let Some(target) = Target::at(window, position, None)
        && target.accepts(x, y)
    {
        target.scrolled();
    }
}
/// Host delivery of one synthesised momentum frame as a native wheel event in the
/// units of the finger stream (pixels, or lines for X11 streams, so each host applies
/// its own line distance). Returns true when the host consumed it; otherwise Slint
/// receives it as a wheel.
pub type KineticSink = Box<dyn FnMut(&WindowEvent) -> bool>;
/// Window region whose wheel notches glide; see [`NativeScroll::glide_content`].
pub type GlideRegion = Box<dyn Fn(slint::LogicalPosition) -> bool>;

/// Host content that scrolls by wheel events without a Slint Flickable (backend
/// viewers, comparison text). Momentum starts only over such content or over a
/// Flickable that can scroll along the fling, never over per-event controls.
pub struct KineticSurface {
    /// Whether this window point lies on the content.
    pub contains: Box<dyn Fn(slint::LogicalPosition) -> bool>,
    /// The content position per axis (any unit), `None` where unknown. Momentum on
    /// an axis stops once its position stops changing (a content edge).
    pub position: Box<dyn Fn() -> [Option<f64>; 2]>,
}

/// Composable native wheel observer for direct scrollbar/command-strip handlers,
/// and the owner of the window's shared kinetic (touchpad momentum) scroller.
pub struct NativeScroll {
    pressed: PressState,
    cursor: Option<PhysicalPosition<f64>>,
    position: Option<LogicalPoint>,
    target: Option<Target>,
    motion: WheelMotion,
    timer: slint::Timer,
    kinetic: NativeKinetic,
    source: WheelSource,
    kinetic_view: Option<KineticView>,
    kinetic_sink: Option<KineticSink>,
    kinetic_surface: Option<KineticSurface>,
    glide_region: Option<GlideRegion>,
    /// The system's wheel setting, which the host reads and keeps current.
    wheel_setting: crate::WheelSetting,
}

impl NativeScroll {
    pub fn new<C: ComponentHandle + 'static>(window: &C) -> Self {
        let weak = window.as_weak();
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, WHEEL_FRAME, move || {
            if let Some(window) = weak.upgrade() {
                window.window().request_redraw();
            }
        });
        timer.stop();
        Self {
            pressed: PressState::default(),
            cursor: None,
            position: None,
            target: None,
            motion: WheelMotion::default(),
            timer,
            kinetic: NativeKinetic::default(),
            source: WheelSource::Other,
            kinetic_view: None,
            kinetic_sink: None,
            kinetic_surface: None,
            glide_region: None,
            wheel_setting: crate::WheelSetting::default(),
        }
    }

    /// The system's wheel setting for the notches this observer glides
    /// (`notch_pixels_for`); the default until the host supplies its own.
    pub fn set_wheel_setting(&mut self, setting: crate::WheelSetting) {
        self.wheel_setting = setting;
    }

    /// Glide wheel notches over this region of the window too, not only over
    /// Fluent scrollbars. For content that takes pixel deltas without easing of its
    /// own; never include controls that act once per wheel event (combo boxes).
    pub fn glide_content(mut self, region: GlideRegion) -> Self {
        self.glide_region = Some(region);
        self
    }

    /// Override the platform default for synthesised touchpad momentum (on except
    /// on Windows and macOS, whose touchpads deliver their own momentum events).
    pub fn synthesize_momentum(mut self, enabled: bool) -> Self {
        self.kinetic.cancel();
        self.kinetic.enabled = enabled;
        self
    }

    /// Route momentum frames over the host surface through a native host path
    /// first (e.g. a browser surface).
    pub fn set_kinetic_sink(&mut self, sink: KineticSink) {
        self.kinetic_sink = Some(sink);
    }

    /// Host content that takes momentum without a Slint Flickable.
    pub fn set_kinetic_surface(&mut self, surface: KineticSurface) {
        self.kinetic_surface = Some(surface);
    }

    /// Stop wheel easing and touchpad momentum.
    pub fn cancel(&mut self) {
        self.cancel_wheel();
        self.kinetic.cancel();
        self.kinetic_view = None;
        self.timer.stop();
    }

    /// Stop only the eased scrollbar/content wheel; touchpad momentum continues.
    pub fn cancel_wheel(&mut self) {
        self.motion.cancel();
        self.target = None;
        if !self.kinetic.kinetic.active() {
            self.timer.stop();
        }
    }

    /// Discard input ownership when a modal owner becomes disabled.
    pub fn reset(&mut self) {
        self.cancel();
        self.kinetic.reset_modifiers();
        self.pressed = PressState::default();
        self.cursor = None;
        self.position = None;
    }

    /// Native drag loops can consume the release before Winit observes it.
    pub fn release_mouse_drag(&mut self) {
        self.pressed.release_left_button();
        self.cancel();
    }

    /// Touchpad momentum is running or waits for an unphased stream to end.
    pub fn kinetic_active(&self) -> bool {
        self.kinetic.kinetic.active()
    }

    /// Classification of the last tracked event. A touchpad sample should reach
    /// its content without wheel easing.
    pub fn source(&self) -> WheelSource {
        self.source
    }

    fn logical_cursor(&self, window: &slint::Window) -> Option<LogicalPoint> {
        let scale = window.scale_factor();
        self.cursor
            .filter(|_| scale.is_finite() && scale > 0.0)
            .map(|p| {
                let p = p.to_logical::<f32>(f64::from(scale));
                LogicalPoint::new(p.x, p.y)
            })
    }

    /// Feed one native event to the kinetic tracker. Call exactly once per event,
    /// before routing it; [`Self::observe`] does both.
    pub fn track(&mut self, window: &slint::Window, event: &WindowEvent) -> WheelSource {
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = Some(*position);
        }
        let cursor = self.logical_cursor(window);
        let was_flinging = self.kinetic.kinetic.flinging();
        self.source = self
            .kinetic
            .observe(event, cursor, window.scale_factor(), now(window));
        if !self.kinetic.kinetic.flinging() || !was_flinging {
            self.kinetic_view = None;
        }
        if self.kinetic.kinetic.active() && !self.timer.running() {
            self.timer.restart();
        }
        self.source
    }

    pub fn observe(
        &mut self,
        window: &slint::Window,
        event: &WindowEvent,
        command: Option<CommandStrip>,
    ) -> bool {
        self.track(window, event);
        self.route(window, event, command)
    }

    /// Resolve what this fling scrolls. Momentum needs content that scrolls along
    /// it: the host surface, or a Flickable (whose other axis is stopped). Anything
    /// else, such as a spin box acting once per wheel event, gets none.
    fn kinetic_view(&mut self, window: &slint::Window, origin: LogicalPoint) -> Option<()> {
        if self.kinetic_view.is_some() {
            return Some(());
        }
        let point = slint::LogicalPosition::new(origin.x, origin.y);
        let view = if self
            .kinetic_surface
            .as_ref()
            .is_some_and(|surface| (surface.contains)(point))
        {
            KineticView::Surface
        } else {
            let velocity = self.kinetic.kinetic.fling_velocity()?;
            let (flick, axes) = flickable_at(window, origin, velocity)?;
            for (axis, scrolls) in axes.into_iter().enumerate() {
                if !scrolls {
                    self.kinetic.kinetic.stop_axis(axis);
                }
            }
            KineticView::Flickable {
                flick,
                bar: Target::at(window, origin, None),
            }
        };
        self.kinetic_view = Some(view);
        Some(())
    }

    fn kinetic_frame(&mut self, window: &slint::Window, now: Duration) {
        let Some((mut x, mut y)) = self.kinetic.kinetic.tick(now) else {
            return;
        };
        let Some(origin) = self.kinetic.origin else {
            self.kinetic.cancel();
            return;
        };
        if self.kinetic_view(window, origin).is_none() {
            self.kinetic.cancel();
            return;
        }
        // An axis the Flickable cannot scroll was stopped above, after this frame's tick.
        let velocity = self.kinetic.kinetic.fling_velocity().unwrap_or_default();
        if velocity[0] == 0.0 {
            x = 0.0;
        }
        if velocity[1] == 0.0 {
            y = 0.0;
        }
        let scale = window.scale_factor();
        let consumed = matches!(self.kinetic_view, Some(KineticView::Surface))
            && self.kinetic_sink.as_mut().is_some_and(|sink| {
                sink(&WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: if self.kinetic.lines {
                        MouseScrollDelta::LineDelta(x / LINE_PIXELS, y / LINE_PIXELS)
                    } else {
                        MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                            f64::from(x * scale),
                            f64::from(y * scale),
                        ))
                    },
                    phase: TouchPhase::Moved,
                })
            });
        if !consumed && (x != 0.0 || y != 0.0) {
            if let Some(KineticView::Flickable { bar: Some(bar), .. }) = &self.kinetic_view {
                bar.scrolled();
            }
            window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(origin.x, origin.y),
                delta_x: x,
                delta_y: y,
            });
        }
        let position = match &self.kinetic_view {
            Some(KineticView::Surface) => self
                .kinetic_surface
                .as_ref()
                .map_or([None; 2], |surface| (surface.position)()),
            Some(KineticView::Flickable { flick, .. }) => flick
                .upgrade()
                .and_then(|item| {
                    let flick = item.downcast::<Flickable>()?;
                    let flick = flick.as_pin_ref();
                    Some([
                        Some(f64::from(flick.content_x().get())),
                        Some(f64::from(flick.content_y().get())),
                    ])
                })
                .unwrap_or([None; 2]),
            None => [None; 2],
        };
        self.kinetic.kinetic.observe_position(position, now);
    }

    /// Route an event already passed to [`Self::track`].
    pub fn route(
        &mut self,
        window: &slint::Window,
        event: &WindowEvent,
        command: Option<CommandStrip>,
    ) -> bool {
        self.pressed.observe(event);
        let now = now(window);
        let cursor = self.logical_cursor(window);
        if self.target.as_ref().is_some_and(|target| {
            cursor.is_none_or(|point| !target.valid(window, point, command.as_ref()))
                || self
                    .position
                    .is_none_or(|point| !target.valid(window, point, command.as_ref()))
        }) {
            self.cancel_wheel();
        }
        match event {
            WindowEvent::RedrawRequested => {
                if let Some((x, y)) = self.motion.tick(now)
                    && let Some(point) = self.position
                {
                    // Release cursor-following before content-y changes; its
                    // changed handler otherwise restores the first wheel step.
                    if let Some(target) = &self.target {
                        target.scrolled();
                    }
                    window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(point.x, point.y),
                        delta_x: x,
                        delta_y: y,
                    });
                }
                if !self.motion.active() {
                    self.cancel_wheel();
                }
                self.kinetic_frame(window, now);
                if !self.motion.active() && !self.kinetic.kinetic.active() {
                    self.kinetic_view = None;
                    self.timer.stop();
                }
            }
            // Finger motion follows the finger: it reaches the content directly,
            // as a wheel without a phase. Slint then neither eases it like a notch
            // nor starts its own Flickable fling on the lift, so the shared
            // momentum is the only one.
            WindowEvent::MouseWheel { delta, .. } if self.source == WheelSource::Touchpad => {
                self.cancel_wheel();
                let Some(point) = cursor else {
                    return false;
                };
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * LINE_PIXELS, y * LINE_PIXELS),
                    MouseScrollDelta::PixelDelta(delta) => {
                        let delta = delta.to_logical::<f32>(f64::from(window.scale_factor()));
                        (delta.x, delta.y)
                    }
                };
                if x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0) {
                    notify_scrollbar_wheel(window, point, x, y);
                    window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(point.x, point.y),
                        delta_x: x,
                        delta_y: y,
                    });
                }
                return true;
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
                ..
            } if !self.pressed.active() => {
                let Some(point) = cursor else {
                    return false;
                };
                // A notch with Ctrl, Alt or Super acts (zooms) once per wheel event;
                // gliding it would repeat the action for every frame.
                let target = Target::at(window, point, command.as_ref()).or_else(|| {
                    let glide = !self.kinetic.wheel_modified()
                        && self.glide_region.as_ref().is_some_and(|region| {
                            region(slint::LogicalPosition::new(point.x, point.y))
                        });
                    glide.then(|| Target::content(window))
                });
                let Some(target) = target else {
                    self.cancel_wheel();
                    return false;
                };
                if !target.accepts(*x, *y) {
                    self.cancel_wheel();
                    return false;
                }
                if self.target.as_ref() != Some(&target) {
                    self.cancel_wheel();
                }
                self.target = Some(target);
                self.position = Some(point);
                let notch = crate::notch_pixels_for(self.wheel_setting, 0.0);
                if self.motion.push(x * notch, y * notch, now) {
                    if !self.timer.running() {
                        self.timer.restart();
                    }
                    window.request_redraw();
                    return true;
                }
            }
            WindowEvent::CursorLeft { .. }
            | WindowEvent::Focused(false)
            | WindowEvent::Destroyed => {
                self.cancel();
                self.cursor = None;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.cancel_wheel();
                if let Some(point) = cursor {
                    let (x, y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                        MouseScrollDelta::PixelDelta(point) => (point.x as f32, point.y as f32),
                    };
                    notify_scrollbar_wheel(window, point, x, y);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
            | WindowEvent::Touch(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Resized(_) => self.cancel(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.cancel()
            }
            _ => {}
        }
        false
    }
}

fn now(window: &slint::Window) -> Duration {
    WindowInner::from_pub(window)
        .context()
        .platform()
        .duration_since_start()
}

/// The innermost Flickable under `position` that scrolls along `velocity` (as
/// Slint routes a wheel), with the axes it can scroll.
fn flickable_at(
    window: &slint::Window,
    position: LogicalPoint,
    velocity: [f64; 2],
) -> Option<(ItemWeak, [bool; 2])> {
    let (scope, _) = scope(window);
    let mut views = Vec::new();
    let mut bars = Vec::new();
    collect(scope, position, &mut views, &mut bars);
    views.into_iter().rev().find_map(|view| {
        let area = bounds(&view);
        let flick = view.downcast::<Flickable>()?;
        let flick = flick.as_pin_ref();
        let axes = [
            flick.content_width().get() > area.width(),
            flick.content_height().get() > area.height(),
        ];
        let along = (velocity[0] != 0.0 && axes[0]) || (velocity[1] != 0.0 && axes[1]);
        along.then(|| (view.downgrade(), axes))
    })
}

pub fn install<C: ComponentHandle + 'static>(
    component: &C,
    blocked: impl Fn(&slint::Window) -> bool + 'static,
) {
    let weak = component.as_weak();
    let mut scroll = NativeScroll::new(component);
    component.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        if blocked(window.window()) {
            scroll.reset();
            return EventResult::Propagate;
        }
        if scroll.observe(window.window(), event, None) {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
}
