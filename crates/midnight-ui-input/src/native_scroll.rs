//! Smooth the pinned Fluent scrollbar wheel path and the horizontally remapped
//! command prompt. Ordinary content keeps Slint's own wheel/touch physics.
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
            event::{ElementState, MouseScrollDelta, TouchPhase, WindowEvent},
        },
    },
};

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
            Self::Command(_) => x != 0.0 || y != 0.0,
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
/// Composable native wheel observer for direct scrollbar/command-strip handlers.
pub struct NativeScroll {
    pressed: PressState,
    cursor: Option<PhysicalPosition<f64>>,
    position: Option<LogicalPoint>,
    target: Option<Target>,
    motion: WheelMotion,
    timer: slint::Timer,
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
        }
    }

    pub fn cancel(&mut self) {
        self.motion.cancel();
        self.target = None;
        self.timer.stop();
    }

    /// Discard input ownership when a modal owner becomes disabled.
    pub fn reset(&mut self) {
        self.cancel();
        self.pressed = PressState::default();
        self.cursor = None;
        self.position = None;
    }

    /// Native drag loops can consume the release before Winit observes it.
    pub fn release_mouse_drag(&mut self) {
        self.pressed.release_left_button();
        self.cancel();
    }

    pub fn observe(
        &mut self,
        window: &slint::Window,
        event: &WindowEvent,
        command: Option<CommandStrip>,
    ) -> bool {
        self.pressed.observe(event);
        let now = WindowInner::from_pub(window)
            .context()
            .platform()
            .duration_since_start();
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = Some(*position);
        }
        let scale = window.scale_factor();
        let cursor = self
            .cursor
            .filter(|_| scale.is_finite() && scale > 0.0)
            .map(|p| {
                let p = p.to_logical::<f32>(f64::from(scale));
                LogicalPoint::new(p.x, p.y)
            });
        if self.target.as_ref().is_some_and(|target| {
            cursor.is_none_or(|point| !target.valid(window, point, command.as_ref()))
                || self
                    .position
                    .is_none_or(|point| !target.valid(window, point, command.as_ref()))
        }) {
            self.cancel();
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
                    self.cancel();
                }
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
                ..
            } if !self.pressed.active() => {
                let Some(point) = cursor else {
                    return false;
                };
                let Some(target) = Target::at(window, point, command.as_ref()) else {
                    self.cancel();
                    return false;
                };
                if !target.accepts(*x, *y) {
                    self.cancel();
                    return false;
                }
                if self.target.as_ref() != Some(&target) {
                    self.cancel();
                }
                self.target = Some(target);
                self.position = Some(point);
                if self.motion.push(x * 60.0, y * 60.0, now) {
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
                self.cancel();
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
