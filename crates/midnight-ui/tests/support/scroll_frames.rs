//! Controlled native wheel frames for real app components, including embedded popups.
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    input::{BackendMouseEvent, TouchPhase},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    lengths::LogicalPoint,
    window::WindowInner,
};
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct ClockPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

pub struct Frames<C: ComponentHandle + 'static> {
    pub window: C,
    native: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    pixels: Vec<slint::Rgb8Pixel>,
    width: usize,
    driver: Option<crate::native_scroll::NativeScroll>,
    pub command: Option<crate::native_scroll::CommandStrip>,
}

impl<C: ComponentHandle + 'static> Frames<C> {
    pub fn new(create: impl FnOnce() -> Result<C, PlatformError>, width: u32, height: u32) -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(ClockPlatform(native.clone(), clock.clone())))
            .unwrap();
        let window = create().unwrap();
        window.show().unwrap();
        native.set_size(slint::PhysicalSize::new(width, height));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        Self {
            window,
            native,
            clock,
            pixels: vec![slint::Rgb8Pixel::default(); width as usize * height as usize],
            width: width as usize,
            driver: None,
            command: None,
        }
    }

    pub fn install_native_scroll(&mut self) {
        self.driver = Some(crate::native_scroll::NativeScroll::new(&self.window));
    }

    pub fn take_native_scroll(&mut self) -> crate::native_scroll::NativeScroll {
        self.driver.take().unwrap()
    }
    pub fn driver_event(&mut self, event: slint::winit_030::winit::event::WindowEvent) -> bool {
        self.driver
            .as_mut()
            .unwrap()
            .observe(self.window.window(), &event, self.command.clone())
    }

    pub fn driver_wheel(&mut self, point: LogicalPoint, x: f32, y: f32) -> bool {
        use slint::winit_030::winit::{
            dpi::PhysicalPosition,
            event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
        };
        let scale = f64::from(self.window.window().scale_factor());
        self.driver_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(f64::from(point.x) * scale, f64::from(point.y) * scale),
        });
        self.driver_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(x, y),
            phase: TouchPhase::Moved,
        })
    }
    pub fn draw(&mut self) -> bool {
        slint::platform::update_timers_and_animations();
        if let Some(driver) = &mut self.driver {
            driver.observe(
                self.window.window(),
                &slint::winit_030::winit::event::WindowEvent::RedrawRequested,
                self.command.clone(),
            );
        }
        let mut rendered = false;
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, self.width);
            rendered = true;
        });
        rendered
    }

    pub fn advance(&mut self, millis: u64) -> bool {
        self.clock
            .set(self.clock.get() + Duration::from_millis(millis));
        self.draw()
    }

    pub fn settle(&mut self) {
        for _ in 0..4 {
            self.advance(16);
        }
    }

    pub fn item(&self, role: AccessibleRole, label: &str) -> ItemRc {
        let inner = WindowInner::from_pub(self.window.window());
        for popup in inner.active_popups().iter().rev() {
            if let Some(item) = find(ItemRc::new_root(popup.component.clone()), role, label) {
                return item;
            }
        }
        find(ItemRc::new_root(inner.component()), role, label)
            .unwrap_or_else(|| panic!("missing {role:?} {label}"))
    }

    pub fn native_wheel(&mut self, point: LogicalPoint, delta_y: f32) {
        self.window
            .window()
            .dispatch_event(WindowEvent::internal(BackendMouseEvent::Wheel {
                position: point,
                delta_x: 0.0,
                delta_y,
                phase: TouchPhase::Moved,
            }));
        self.draw();
    }

    pub fn pixels(&mut self, point: LogicalPoint, delta_y: f32) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(point.x, point.y),
                delta_x: 0.0,
                delta_y,
            });
        self.draw();
    }

    /// Catch immediate wheel assignments, row snapping and animations that only
    /// change a property while rendered row positions remain stationary.
    pub fn check_motion(&mut self, role: AccessibleRole, label: &str) -> ItemRc {
        let row = self.item(role, label);
        let viewport = scroll_parent(&row);
        let point = inside(&viewport);
        let initial = offset(&viewport);
        let initial_y = bounds(&row).1;
        let row_height = row.geometry().height();
        self.native_wheel(point, -60.0);
        near(
            offset(&viewport),
            initial,
            "native wheel must begin an animation",
        );
        let mut previous = initial;
        let mut moving_frames = 0;
        let mut fractional_frames = 0;
        for _ in 0..15 {
            let rendered = self.advance(16);
            let current = offset(&viewport);
            let delta = current - previous;
            assert!(
                delta <= 0.01,
                "wheel reversed unexpectedly: {previous} -> {current}"
            );
            assert!(
                delta.abs() < row_height,
                "whole-row jump: {previous} -> {current}"
            );
            if delta.abs() > 0.01 {
                assert!(rendered, "moving content must request a rendered frame");
                moving_frames += 1;
                fractional_frames += usize::from((current - current.round()).abs() > 0.01);
                let row = self.item(role, label);
                near(
                    bounds(&row).1 - initial_y,
                    current - initial,
                    "rendered row follows offset",
                );
            }
            previous = current;
        }
        assert!(
            moving_frames >= 10,
            "only {moving_frames} distinct moving frames"
        );
        assert!(
            fractional_frames >= 6,
            "only {fractional_frames} fractional moving frames"
        );
        near(
            offset(&viewport),
            initial - 60.0,
            "wheel must retain its exact distance",
        );
        let settled = offset(&viewport);
        self.advance(400);
        near(
            offset(&viewport),
            settled,
            "ordinary lists must not snap onto a row",
        );

        // A direction change while the first impulse is still moving must be
        // visible on the next frame, without first finishing the old direction.
        self.native_wheel(point, -60.0);
        self.advance(48);
        let turning = offset(&viewport);
        self.native_wheel(point, 60.0);
        self.advance(16);
        assert!(
            offset(&viewport) > turning,
            "reverse wheel must reverse rendered movement"
        );
        for _ in 0..14 {
            self.advance(16);
        }
        near(
            offset(&viewport),
            settled,
            "opposite wheel impulses retain their distance",
        );

        self.pixels(point, -7.25);
        near(
            offset(&viewport),
            settled - 7.25,
            "direct pixels retain fractional distance",
        );
        let direct = offset(&viewport);
        self.advance(400);
        near(
            offset(&viewport),
            direct,
            "direct pixel input must not acquire another coast",
        );
        viewport
    }
}

fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .as_deref()
            == Some(label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, role, label) {
            return Some(found);
        }
    }
    None
}

pub fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    loop {
        let item = parent.expect("control must have a scrolling ancestor");
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

pub fn offset(item: &ItemRc) -> f32 {
    item.downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_y()
        .get()
}

pub fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

pub fn inside(item: &ItemRc) -> LogicalPoint {
    let mut clip = bounds(item);
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let area = bounds(&item);
            clip = (
                clip.0.max(area.0),
                clip.1.max(area.1),
                clip.2.min(area.2),
                clip.3.min(area.3),
            );
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    assert!(
        clip.2 - clip.0 > 20.0 && clip.3 - clip.1 > 4.0,
        "empty scroll clip {clip:?}"
    );
    LogicalPoint::new(clip.0 + 20.0, (clip.1 + clip.3) / 2.0)
}

pub fn near(actual: f32, expected: f32, reason: &str) {
    assert!(
        (actual - expected).abs() < 0.03,
        "{reason}: {actual} != {expected}"
    );
}
