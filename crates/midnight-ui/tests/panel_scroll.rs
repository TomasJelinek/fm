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
const BACKGROUND: [u8; 3] = [16, 32, 48];
const EVEN: [u8; 3] = [255, 255, 255];
const ODD: [u8; 3] = [238, 238, 238];

slint::slint! {
    import { PanelList } from "ui/panel-list.slint";
    export component ScrollTest inherits Window {
        width: 240px;
        height: 205px;
        in property <[int]> rows;
        in property <length> row-height: 24px;
        in property <bool> interaction-active;
        in property <bool> settle-rows: true;
        in property <bool> reduce-motion;
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
        pure public function band-visible(band: int) -> bool { return list.band-visible(band); }
        list := PanelList {
            width: parent.width;
            height: root.list-height;
            row-height: root.row-height;
            interaction-active: root.interaction-active;
            settle-rows: root.settle-rows;
            reduce-motion: root.reduce-motion;
            x: 0px; y: 0px;
            for row[index] in root.rows : Rectangle {
                height: root.row-height;
                Rectangle {
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
    reads: Rc<Cell<usize>>,
    pixels: RefCell<Vec<slint::Rgb8Pixel>>,
}

impl Harness {
    fn new() -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let time = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone())))
            .unwrap();
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
            reads,
            pixels: RefCell::new(vec![
                slint::Rgb8Pixel::default();
                PHYSICAL_WIDTH * PHYSICAL_HEIGHT
            ]),
        };
        h.draw();
        h
    }

    fn draw(&self) {
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels.borrow_mut(), PHYSICAL_WIDTH);
        });
        // Read the offset too, so virtual animation frames are observed even if
        // a future renderer decides that an intermediate frame is unchanged.
        let _ = self.window.get_offset();
    }

    fn pixel(&self, y: f32) -> [u8; 3] {
        let row = (y * SCALE) as usize;
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

    /// A frame-sized step of an application glide, as `scroll_input` delivers it.
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

    fn offset(&self) -> f32 {
        self.window.get_offset()
    }

    fn reset(&self) {
        self.window.invoke_reveal(0.0);
        self.advance(500);
    }

    /// Offsets of every frame until the list has been idle for `millis`.
    fn trace(&self, millis: u64) -> Vec<f32> {
        (0..millis / 10)
            .map(|_| {
                self.jump(10);
                self.offset()
            })
            .collect()
    }
}

fn on_row(offset: f32, row: f32) -> bool {
    ((offset / row) - (offset / row).round()).abs() < 1e-4
}

#[test]
fn partly_visible_rows_are_drawn_and_clickable_at_a_fractional_offset() {
    let h = Harness::new();
    assert_eq!(h.window.get_content_height(), 24000.0);
    assert!(
        h.reads.get() < 100,
        "only visible rows should be materialized: {}",
        h.reads.get()
    );
    // An application glide: one frame-sized pixel step, then input holds the list.
    h.window.set_interaction_active(true);
    h.wheel(-37.0);
    assert_eq!(
        h.offset(),
        -37.0,
        "public pixel input must retain its original delta"
    );
    h.advance(500);
    assert_eq!(h.offset(), -37.0, "held input keeps the pixel position");
    // Row 1 is cut by the top edge, row 10 by the bottom edge: both are drawn, so
    // no band of the list background shows anywhere.
    assert_eq!(h.pixel(5.0), ODD);
    assert_eq!(h.pixel(15.0), EVEN);
    assert_eq!(h.pixel(190.0), ODD);
    assert_eq!(h.pixel(204.0), EVEN);
    for y in 0..205 {
        assert_ne!(
            h.pixel(y as f32),
            BACKGROUND,
            "blank band at {y} px at offset -37"
        );
    }
    let clicked = Rc::new(Cell::new(None));
    let recorded = clicked.clone();
    h.window.on_row_clicked(move |row| recorded.set(Some(row)));
    h.click(5.0);
    assert_eq!(
        clicked.get(),
        Some(1),
        "the row cut at the top takes clicks"
    );
    h.click(204.0);
    assert_eq!(
        clicked.get(),
        Some(10),
        "the row cut at the bottom takes clicks"
    );
}

/// Exhaustive over fractional offsets, row heights and viewport heights: the
/// bands the list draws cover its viewport, without a gap at the top or the
/// bottom, and with at most one extra band at either edge.
#[test]
fn drawn_bands_cover_the_viewport_at_every_offset_row_height_and_viewport() {
    let h = Harness::new();
    let rows = 1000.0;
    for row in [
        1.0f32, 7.5, 13.0, 22.0, 24.0, 27.0, 40.0, 135.0, 162.0, 199.5,
    ] {
        h.window.set_row_height(row);
        for list in [10.0f32, 23.0, 36.0, 66.0, 190.0, 205.0] {
            h.window.set_list_height(list);
            let lowest = (list - row * rows).min(0.0);
            for band in [0.0f32, 1.0, 37.0] {
                for step in 0..64 {
                    let offset = (-(band + step as f32 / 64.0) * row).max(lowest);
                    h.window.set_offset(offset);
                    let first = (-offset / row).floor() as i32;
                    let last = ((list - offset) / row).ceil() as i32 - 1;
                    for band in first..=last.min(rows as i32 - 1) {
                        assert!(
                            h.window.invoke_band_visible(band),
                            "row {row}, list {list}, offset {offset}: band {band} \
                             is in view ({first}..={last}) but not drawn"
                        );
                    }
                    assert!(
                        !h.window.invoke_band_visible(first - 2)
                            && !h.window.invoke_band_visible(last + 2),
                        "row {row}, list {list}, offset {offset}: bands far out of view drawn"
                    );
                }
            }
        }
    }
}

#[test]
fn an_idle_list_glides_to_the_nearest_row_only_after_input_ends() {
    let h = Harness::new();
    // Native unphased wheel events keep the SDK's 180 ms easing (other platforms'
    // precise pixels); the list settles only after that motion ends.
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(90);
    let moving = h.offset();
    assert!(
        moving < -1.0 && moving > -37.0,
        "native wheel must animate: {moving}"
    );
    let trace = h.trace(700);
    let at_rest = trace.iter().position(|&y| (y + 37.0).abs() < 0.01).unwrap();
    // 37 px is 1.54 rows of 24 px: the nearest row is the second.
    assert_eq!(*trace.last().unwrap(), -48.0, "settled on the nearest row");
    let aligning = trace
        .iter()
        .position(|&y| y < -37.01)
        .expect("the alignment moves the list");
    assert!(
        aligning > at_rest + 8,
        "the list rests for the idle interval before aligning: rest at frame \
         {at_rest}, alignment at {aligning}"
    );
    // The alignment glides: small monotonic steps over about 200 ms, no jump.
    let glide = &trace[aligning - 1..];
    let steps: Vec<f32> = glide.windows(2).map(|w| w[0] - w[1]).collect();
    assert!(
        steps.iter().all(|&step| (0.0..3.0).contains(&step)),
        "the alignment glides in small steps: {steps:?}"
    );
    let moving_frames = steps.iter().filter(|&&step| step > 0.0).count();
    assert!(
        (15..=21).contains(&moving_frames),
        "the alignment takes about 200 ms: {moving_frames} frames of 10 ms"
    );
    assert!(
        slint::platform::duration_until_next_timer_update().is_none(),
        "settling leaves no idle timer running"
    );

    // Upward motion settles on its own nearest row.
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(1000);
    h.native_wheel(30.0, TouchPhase::Moved);
    h.advance(1000);
    assert_eq!(h.offset(), -24.0, "-18 px is nearest to the first row");
}

#[test]
fn input_that_still_holds_the_list_postpones_the_alignment() {
    let h = Harness::new();
    // A Windows touchpad's lines become an application glide whose frames arrive
    // as pixel steps; the window reports the glide, fingers and momentum as
    // interaction until they end.
    h.window.set_interaction_active(true);
    for _ in 0..5 {
        h.wheel(-7.3);
        h.jump(16);
    }
    let held = h.offset();
    assert!((held + 36.5).abs() < 0.01);
    h.advance(2000);
    assert_eq!(h.offset(), held, "no alignment while input is active");
    h.window.set_interaction_active(false);
    h.advance(80);
    assert_eq!(h.offset(), held, "the idle interval follows the input");
    h.advance(600);
    assert_eq!(h.offset(), -48.0);

    // A held finger can pause at a fractional row without causing a snap.
    h.reset();
    h.touch(160.0, TouchPhase::Started);
    h.window.set_interaction_active(true);
    h.advance(20);
    h.touch(123.0, TouchPhase::Moved);
    let held = h.offset();
    assert!(
        (held + 37.0).abs() < 0.1,
        "touch drag must track pixels: {held}"
    );
    h.advance(500);
    assert_eq!(h.offset(), held, "held touch must not snap");
    h.touch(123.0, TouchPhase::Ended);
    h.window.set_interaction_active(false);
    h.advance(800);
    assert_eq!(h.offset(), -48.0, "the released touch settles");

    // A quick touch release keeps native inertia, then settles.
    h.reset();
    h.touch(160.0, TouchPhase::Started);
    h.advance(20);
    h.touch(145.0, TouchPhase::Moved);
    h.advance(20);
    h.touch(120.0, TouchPhase::Moved);
    h.advance(10);
    h.touch(120.0, TouchPhase::Ended);
    h.advance(60);
    let inertia = h.offset();
    assert!(
        inertia < -50.0,
        "release must retain native inertia: {inertia}"
    );
    h.advance(60);
    assert!(h.offset() < inertia - 10.0, "inertia must keep moving");
    h.advance(3000);
    assert!(
        on_row(h.offset(), 24.0),
        "inertia ends on a row: {}",
        h.offset()
    );
}

#[test]
fn new_input_and_explicit_positions_supersede_the_alignment() {
    let h = Harness::new();
    // Restarting scroll during settlement cancels that animation immediately.
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(400);
    let settling = h.offset();
    assert!(
        settling < -37.0 && settling > -48.0,
        "settle must animate: {settling}"
    );
    h.window.set_interaction_active(true);
    h.wheel(10.0);
    assert!(
        (h.offset() - settling - 10.0).abs() < 0.5,
        "new input continues from the shown position: {settling} -> {}",
        h.offset()
    );
    let moved = h.offset();
    h.advance(500);
    assert_eq!(h.offset(), moved, "the old alignment stays cancelled");
    h.window.set_interaction_active(false);
    h.advance(800);
    assert!(on_row(h.offset(), 24.0));

    // Programmatic cursor reveals stay exact even if an alignment was pending
    // or running.
    for wait in [50, 400] {
        h.reset();
        h.native_wheel(-37.0, TouchPhase::Moved);
        h.advance(wait);
        h.window.invoke_reveal(-123.0);
        h.advance(1000);
        assert_eq!(
            h.offset(),
            -123.0,
            "reveal after {wait} ms must remain exact"
        );
    }
    // A refreshed model alone never arms the alignment nor moves the viewport.
    h.rows.set_vec((0..1001).collect::<Vec<_>>());
    h.draw();
    h.advance(1000);
    assert_eq!(h.offset(), -123.0, "refresh must preserve the viewport");

    // A resize during the alignment aims at the new nearest row in the new bounds.
    h.rows.set_vec((0..10).collect::<Vec<_>>());
    h.window.set_list_height(190.0);
    h.reset();
    h.native_wheel(-45.0, TouchPhase::Moved);
    h.advance(380);
    h.window.set_list_height(205.0);
    h.draw();
    h.advance(1000);
    assert_eq!(h.offset(), -35.0, "the new bottom bound wins");
}

#[test]
fn the_bottom_bound_the_grid_and_reduced_motion_keep_their_positions() {
    let h = Harness::new();
    // The exact bottom wins when the viewport does not contain whole rows; the
    // row cut at the top is drawn.
    h.window.set_list_height(190.0);
    h.wheel(-100000.0);
    // A long ListView seek materializes the target row at its origin. The next
    // small pixel input reaches the exact lower bound without another seek.
    h.wheel(-13.0);
    h.advance(1000);
    assert_eq!(h.window.get_visible_height(), 190.0);
    assert!(
        (h.offset() + 23810.0).abs() < 0.1,
        "bottom must remain clamped: offset {}, content {}",
        h.offset(),
        h.window.get_content_height()
    );
    // 23,810 px is 992 rows and 2 px: row 992 shows its last 22 px.
    assert_eq!(h.pixel(2.0), EVEN, "the row cut at the top is drawn");

    // The thumbnail grid keeps its pixel position.
    h.window.set_settle_rows(false);
    h.window.set_list_height(205.0);
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    h.advance(1000);
    assert!((h.offset() + 37.0).abs() < 0.1, "{}", h.offset());
    h.window.set_settle_rows(true);

    // Reduced motion aligns at once, without a glide.
    h.window.set_reduce_motion(true);
    h.reset();
    h.native_wheel(-37.0, TouchPhase::Moved);
    let trace = h.trace(800);
    let moved: Vec<&f32> = trace
        .iter()
        .skip_while(|&&y| y > -37.01)
        .take_while(|&&y| y > -47.99)
        .collect();
    assert!(
        moved.is_empty(),
        "no intermediate alignment frames: {moved:?}"
    );
    assert_eq!(h.offset(), -48.0);
    h.window.set_reduce_motion(false);

    // Short and empty models remain in bounds.
    for row_count in [0, 2] {
        h.rows.set_vec((0..row_count).collect::<Vec<_>>());
        h.reset();
        h.wheel(-37.0);
        h.advance(1000);
        assert_eq!(h.offset(), 0.0, "short model must not expose blank space");
    }
    assert!(
        slint::platform::duration_until_next_timer_update().is_none(),
        "scrolling leaves no idle timer running"
    );
}
