use i_slint_core::{
    input::{BackendMouseEvent, TouchPhase},
    lengths::LogicalPoint,
    platform::InternalEvent,
};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    import { PanelList } from "ui/panel-list.slint";
    export component ScrollTest inherits Window {
        width: 240px;
        height: 205px;
        in property <[int]> rows;
        in property <length> row-height: 24px;
        in property <bool> interaction-active;
        in property <length> list-height: 205px;
        in-out property <length> offset <=> list.content-y;
        out property <length> content-height <=> list.content-height;
        out property <length> visible-height <=> list.visible-height;
        public function reveal(offset: length) {
            list.cancel-scroll-settle();
            list.content-y = offset;
        }
        public function cancel() { list.cancel-scroll-settle(); }
        list := PanelList {
            width: parent.width;
            height: root.list-height;
            row-height: root.row-height;
            interaction-active: root.interaction-active;
            for row in root.rows : Rectangle {
                height: root.row-height;
                background: mod(row, 2) == 0 ? #ffffff : #eeeeee;
                TouchArea { }
            }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);

struct CountedRows {
    rows: Rc<slint::VecModel<i32>>,
    reads: Rc<Cell<usize>>,
}

impl Model for CountedRows {
    type Data = i32;

    fn row_count(&self) -> usize {
        self.rows.row_count()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.reads.set(self.reads.get() + 1);
        self.rows.row_data(row)
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        self.rows.model_tracker()
    }
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

struct Harness {
    native: Rc<MinimalSoftwareWindow>,
    time: Rc<Cell<Duration>>,
    window: ScrollTest,
    rows: Rc<slint::VecModel<i32>>,
}

impl Harness {
    fn draw(&self) {
        let mut pixels = vec![slint::Rgb8Pixel::default(); 240 * 205];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 240);
        });
        // Read the offset too, so virtual animation frames are observed even if
        // a future renderer decides that an intermediate frame is unchanged.
        let _ = self.window.get_offset();
    }

    fn advance(&self, millis: u64) {
        for _ in 0..millis / 10 {
            self.jump(10);
        }
    }

    fn jump(&self, millis: u64) {
        self.time
            .set(self.time.get() + Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
        self.draw();
    }

    fn wheel(&self, delta_y: f32) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(100.0, 100.0),
                delta_x: 0.0,
                delta_y,
            });
        self.draw();
    }

    fn native_wheel(&self, delta_y: f32, phase: TouchPhase) {
        self.window
            .window()
            .dispatch_event(WindowEvent::internal(BackendMouseEvent::Wheel {
                position: LogicalPoint::new(100.0, 100.0),
                delta_x: 0.0,
                delta_y,
                phase,
            }));
        self.draw();
    }

    fn touch(&self, y: f32, phase: TouchPhase) {
        // The production Winit observer supplies this state before forwarding
        // touch input. Synthetic WindowEvents bypass that native observer.
        self.window
            .set_interaction_active(matches!(phase, TouchPhase::Started | TouchPhase::Moved));
        self.window
            .window()
            .dispatch_event(WindowEvent::internal(InternalEvent::Touch {
                id: 7,
                position: LogicalPoint::new(100.0, y),
                phase,
            }));
        self.draw();
    }

    fn reset(&self) {
        self.window.invoke_reveal(0.0);
        self.advance(300);
    }
}

#[test]
fn pixel_scroll_settles_on_rows_after_native_motion_stops() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = ScrollTest::new().unwrap();
    let rows = Rc::new(slint::VecModel::from((0..1000).collect::<Vec<_>>()));
    let reads = Rc::new(Cell::new(0));
    window.set_rows(slint::ModelRc::new(CountedRows {
        rows: rows.clone(),
        reads: reads.clone(),
    }));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(240, 205));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(240.0, 205.0),
    });
    let h = Harness {
        native,
        time,
        window,
        rows,
    };
    h.draw();
    assert_eq!(h.window.get_content_height(), 24000.0);
    assert!(
        reads.get() < 100,
        "only visible rows should be materialized: {}",
        reads.get()
    );

    h.wheel(-37.0);
    let moving = h.window.get_offset();
    assert_eq!(
        moving, -37.0,
        "public pixel input must retain its original delta"
    );
    assert!(
        (moving / 24.0).fract().abs() > 0.01,
        "motion must retain fractional rows"
    );
    h.advance(500);
    assert!(
        (h.window.get_offset() + 48.0).abs() < 0.1,
        "idle must align nearest row"
    );

    // Native unphased wheel events retain the SDK's 180ms deceleration.
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(90);
    let moving = h.window.get_offset();
    assert!(
        moving < -1.0 && moving > -37.0,
        "native wheel must animate: {moving}"
    );
    assert!((moving / 24.0).fract().abs() > 0.01);
    h.advance(100);
    assert!(
        (h.window.get_offset() + 37.0).abs() < 0.1,
        "native motion must finish first"
    );
    h.advance(500);
    assert!((h.window.get_offset() + 48.0).abs() < 0.1);

    // A delayed frame must not let an expired idle timer interrupt physics.
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(10);
    h.jump(100);
    h.jump(70);
    assert!(
        (h.window.get_offset() + 37.0).abs() < 0.1,
        "slow rendering must retain the whole native wheel delta: {}",
        h.window.get_offset()
    );
    h.advance(500);
    assert!((h.window.get_offset() + 48.0).abs() < 0.1);

    // A held finger can pause at a fractional row without causing a snap.
    h.reset();
    h.touch(160.0, TouchPhase::Started);
    h.advance(20);
    h.touch(123.0, TouchPhase::Moved);
    let held = h.window.get_offset();
    assert!(
        (held + 37.0).abs() < 0.1,
        "touch drag must track pixels: {held}"
    );
    h.advance(500);
    assert!(
        (h.window.get_offset() - held).abs() < 0.1,
        "held touch must not snap"
    );
    h.touch(123.0, TouchPhase::Ended);
    h.advance(500);
    assert!((h.window.get_offset() + 48.0).abs() < 0.1);

    // A quick touch release keeps native inertia; settling follows its end.
    h.reset();
    h.touch(160.0, TouchPhase::Started);
    h.advance(20);
    h.touch(145.0, TouchPhase::Moved);
    h.advance(20);
    h.touch(120.0, TouchPhase::Moved);
    h.advance(10);
    h.touch(120.0, TouchPhase::Ended);
    h.advance(60);
    let inertia = h.window.get_offset();
    assert!(
        inertia < -50.0,
        "release must retain native inertia: {inertia}"
    );
    h.advance(60);
    assert!(
        h.window.get_offset() < inertia - 10.0,
        "inertia must keep moving"
    );
    h.advance(1800);
    assert!((h.window.get_offset() / 24.0).fract().abs() < 0.01);

    // Phased touchpad input retains subpixel deltas throughout a held gesture.
    h.reset();
    h.window.set_interaction_active(true);
    h.native_wheel(0.0, TouchPhase::Started);
    h.advance(20);
    h.native_wheel(-10.5, TouchPhase::Moved);
    assert!((h.window.get_offset() + 10.5).abs() < 0.1);
    h.advance(20);
    h.native_wheel(-27.0, TouchPhase::Moved);
    h.advance(500);
    assert!((h.window.get_offset() + 37.5).abs() < 0.1);
    h.window.set_interaction_active(false);
    h.native_wheel(0.0, TouchPhase::Ended);
    h.advance(500);
    assert!((h.window.get_offset() + 48.0).abs() < 0.1);

    // Restarting scroll during settlement cancels that animation immediately.
    h.reset();
    h.wheel(-37.0);
    h.advance(120);
    let settling = h.window.get_offset();
    assert!(
        settling < -37.0 && settling > -48.0,
        "settle must animate: {settling}"
    );
    h.wheel(-24.0);
    let restarted = h.window.get_offset();
    assert!((restarted - settling + 24.0).abs() < 0.1);
    h.advance(40);
    assert!(
        (h.window.get_offset() - restarted).abs() < 0.1,
        "old snap must stay cancelled"
    );
    h.advance(500);
    assert!((h.window.get_offset() + 72.0).abs() < 0.1);

    // Compact Touch rows win over an old pending density target.
    h.reset();
    h.wheel(-37.0);
    h.advance(120);
    h.window.set_row_height(40.0);
    h.draw();
    h.advance(500);
    assert!((h.window.get_offset() / 40.0).fract().abs() < 0.01);

    // A cursor that is already visible still cancels outstanding native motion.
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(50);
    let frozen = h.window.get_offset();
    h.window.invoke_cancel();
    h.advance(500);
    assert!(
        (h.window.get_offset() - frozen).abs() < 0.1,
        "cursor reveal must cancel native movement even without changing offset"
    );

    // Programmatic cursor reveals stay exact even if a wheel settle was pending.
    h.reset();
    h.wheel(-37.0);
    h.advance(120);
    h.window.invoke_reveal(-123.0);
    h.advance(500);
    assert!(
        (h.window.get_offset() + 123.0).abs() < 0.1,
        "programmatic reveal must remain exact: {}",
        h.window.get_offset()
    );
    // A refreshed model alone must never arm snapping or jump the viewport.
    h.rows.set_vec((0..1001).collect::<Vec<_>>());
    h.draw();
    h.advance(500);
    assert!(
        (h.window.get_offset() + 123.0).abs() < 0.1,
        "refresh must preserve the viewport: {}",
        h.window.get_offset()
    );

    // A resize during settlement cancels its stale target and recomputes bounds.
    h.window.set_row_height(24.0);
    h.rows.set_vec((0..10).collect::<Vec<_>>());
    h.window.set_list_height(190.0);
    h.reset();
    h.wheel(-49.0);
    h.advance(120);
    h.window.set_list_height(205.0);
    h.draw();
    h.advance(500);
    assert!((h.window.get_offset() + 35.0).abs() < 0.1);

    // The exact bottom wins when the viewport does not contain whole rows.
    h.window.set_row_height(24.0);
    h.rows.set_vec((0..1000).collect::<Vec<_>>());
    h.window.set_list_height(190.0);
    h.reset();
    h.wheel(-100000.0);
    // A long ListView seek materializes the target row at its origin. The next
    // small pixel input reaches the exact lower bound without another seek.
    h.wheel(-13.0);
    h.advance(500);
    assert_eq!(h.window.get_visible_height(), 190.0);
    assert!(
        (h.window.get_offset() + 23810.0).abs() < 0.1,
        "bottom must remain clamped: offset {}, content {}",
        h.window.get_offset(),
        h.window.get_content_height()
    );

    // Empty and shorter-than-viewport models remain in bounds.
    for row_count in [0, 2] {
        h.rows.set_vec((0..row_count).collect::<Vec<_>>());
        h.reset();
        h.wheel(-37.0);
        h.advance(500);
        assert_eq!(
            h.window.get_offset(),
            0.0,
            "short model must not expose blank space"
        );
    }
    assert!(
        slint::platform::duration_until_next_timer_update().is_none(),
        "settling must leave no idle timer running"
    );
}
