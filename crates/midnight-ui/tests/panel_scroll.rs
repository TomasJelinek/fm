use i_slint_core::{
    input::{BackendMouseEvent, TouchPhase},
    lengths::LogicalPoint,
    platform::InternalEvent,
};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const SCALE: f32 = 1.25;
const PHYSICAL_WIDTH: usize = 300;
const PHYSICAL_HEIGHT: usize = 257;

slint::slint! {
    import { PanelList } from "ui/panel-list.slint";
    export component ScrollTest inherits Window {
        width: 240px;
        height: 205px;
        in property <[int]> rows;
        in property <length> row-height: 24px;
        background: #102030;
        callback row-clicked(int);
        in property <length> list-height: 205px;
        in-out property <length> offset <=> list.content-y;
        out property <length> content-height <=> list.content-height;
        out property <length> visible-height <=> list.visible-height;
        public function reveal(offset: length) {
            list.stop-scroll-motion();
            list.content-y = offset;
        }
        public function cancel() { list.stop-scroll-motion(); }
        list := PanelList {
            width: parent.width;
            height: root.list-height;
            row-height: root.row-height;
            x: 0px; y: 0px;
            for row[index] in root.rows : Rectangle {
                height: root.row-height;
                Rectangle {
                    y: list.row-y-shift;
                    width: parent.width; height: parent.height;
                    visible: list.band-visible(index);
                    background: mod(row, 2) == 0 ? #ffffff : #eeeeee;
                    TouchArea { clicked => { root.row-clicked(row); } }
                }
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
    pixels: RefCell<Vec<slint::Rgb8Pixel>>,
}

impl Harness {
    fn draw(&self) {
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels.borrow_mut(), PHYSICAL_WIDTH);
        });
        // Read the offset too, so virtual animation frames are observed even if
        // a future renderer decides that an intermediate frame is unchanged.
        let _ = self.window.get_offset();
    }

    fn pixel(&self, y: usize) -> [u8; 3] {
        let row = (y as f32 * SCALE) as usize;
        let pixel = self.pixels.borrow()[row * PHYSICAL_WIDTH + 125];
        [pixel.r, pixel.g, pixel.b]
    }

    fn click(&self, y: f32) {
        let position = slint::LogicalPosition::new(100.0, y);
        let button = PointerEventButton::Left;
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
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
                position: slint::LogicalPosition::new(
                    100.0,
                    self.window.get_visible_height() / 2.0,
                ),
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
fn pixel_scroll_keeps_fractional_positions_and_shows_whole_rows() {
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
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
    native.set_size(slint::PhysicalSize::new(
        PHYSICAL_WIDTH as u32,
        PHYSICAL_HEIGHT as u32,
    ));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(240.0, 205.0),
    });
    let h = Harness {
        native,
        time,
        window,
        rows,
        pixels: RefCell::new(vec![
            slint::Rgb8Pixel::default();
            PHYSICAL_WIDTH * PHYSICAL_HEIGHT
        ]),
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
    assert_eq!(
        h.window.get_offset(),
        moving,
        "idle retains the fractional position"
    );

    // Only complete rows draw and receive clicks.
    assert_eq!(h.pixel(5), [16, 32, 48]);
    assert_eq!(h.pixel(15), [255, 255, 255]);
    assert_eq!(h.pixel(190), [238, 238, 238]);
    assert_eq!(h.pixel(204), [16, 32, 48]);
    let clicked = Rc::new(Cell::new(None));
    let recorded = clicked.clone();
    h.window.on_row_clicked(move |row| recorded.set(Some(row)));
    h.click(5.0);
    h.click(204.0);
    assert_eq!(
        clicked.get(),
        None,
        "clipped fragments cannot receive clicks"
    );
    h.click(15.0);
    assert_eq!(
        clicked.get(),
        Some(2),
        "the first complete row remains clickable"
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
    assert!((h.window.get_offset() + 37.0).abs() < 0.1);

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
    assert!((h.window.get_offset() + 37.0).abs() < 0.1);

    // A held finger can pause and release at a fractional row.
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
        "held touch keeps its position"
    );
    h.touch(123.0, TouchPhase::Ended);
    h.advance(500);
    assert!((h.window.get_offset() + 37.0).abs() < 0.1);

    // A quick touch release keeps native inertia.
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
    let resting = h.window.get_offset();
    h.advance(500);
    assert!((h.window.get_offset() - resting).abs() < 0.1);

    // Phased touchpad input retains subpixel deltas throughout a held gesture.
    h.reset();
    h.native_wheel(0.0, TouchPhase::Started);
    h.advance(20);
    h.native_wheel(-10.5, TouchPhase::Moved);
    assert!((h.window.get_offset() + 10.5).abs() < 0.1);
    h.advance(20);
    h.native_wheel(-27.0, TouchPhase::Moved);
    h.advance(500);
    assert!((h.window.get_offset() + 37.5).abs() < 0.1);
    h.native_wheel(0.0, TouchPhase::Ended);
    h.advance(500);
    assert!((h.window.get_offset() + 37.5).abs() < 0.1);

    // Successive gestures accumulate their original pixel deltas.
    h.reset();
    h.wheel(-37.0);
    h.advance(120);
    h.wheel(-24.0);
    h.advance(500);
    assert!((h.window.get_offset() + 61.0).abs() < 0.1);

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

    // A programmatic reveal supersedes in-flight native wheel motion.
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(50);
    h.window.invoke_reveal(-123.0);
    h.advance(500);
    assert!(
        (h.window.get_offset() + 123.0).abs() < 0.1,
        "programmatic reveal must remain exact: {}",
        h.window.get_offset()
    );
    // A refreshed model preserves the viewport.
    h.rows.set_vec((0..1001).collect::<Vec<_>>());
    h.draw();
    h.advance(500);
    assert!(
        (h.window.get_offset() + 123.0).abs() < 0.1,
        "refresh must preserve the viewport: {}",
        h.window.get_offset()
    );

    // Growing the viewport clamps to the new lower bound.
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

    // A viewport with room for one row paints and hits the same aligned band
    // while the underlying pixel offset accumulates small inputs.
    h.rows.set_vec((0..10).collect::<Vec<_>>());
    h.window.set_list_height(36.0);
    h.reset();
    h.wheel(-3.0);
    assert_eq!(h.window.get_offset(), -3.0);
    h.wheel(-5.0);
    h.advance(500);
    assert_eq!(h.window.get_offset(), -8.0);
    assert_eq!(h.pixel(2), [238, 238, 238]);
    assert_eq!(h.pixel(22), [238, 238, 238]);
    assert_eq!(h.pixel(28), [16, 32, 48]);
    h.click(5.0);
    assert_eq!(clicked.get(), Some(1));

    h.wheel(-500.0);
    h.wheel(-13.0);
    h.advance(500);
    assert!((h.window.get_offset() + 204.0).abs() < 0.1);
    h.click(5.0);
    assert_eq!(
        clicked.get(),
        Some(9),
        "the compact viewport can reach its final row"
    );

    // Empty and shorter-than-viewport models remain in bounds.
    h.window.set_list_height(190.0);
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
        "scrolling leaves no idle timer running"
    );
}
