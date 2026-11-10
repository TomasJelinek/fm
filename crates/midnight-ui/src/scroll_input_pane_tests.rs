//! Regression through the real MainWindow, FilePane, presenter model and timer.
use super::*;
use crate::{Presenter, Theme};
use i_slint_core::{item_tree::ItemRc, items::Flickable, lengths::LogicalPoint};
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, Event, NavigationId, PaneId};
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::Cell, path::PathBuf, sync::Arc};

#[path = "scroll_input_toolbar_tests.rs"]
mod toolbar_tests;

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

struct Harness {
    window: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    time: Rc<Cell<Duration>>,
    presenter: Rc<RefCell<Presenter>>,
    state: Rc<RefCell<InputState>>,
    timer: Rc<slint::Timer>,
    pixels: RefCell<Vec<slint::Rgb8Pixel>>,
}

impl Harness {
    fn draw(&self) {
        let mut pixels = self.pixels.borrow_mut();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1340);
        });
    }

    fn advance(&self, frames: usize) {
        for _ in 0..frames {
            self.time.set(self.time.get() + Duration::from_millis(16));
            slint::platform::update_timers_and_animations();
            // Folder-size/metadata acknowledgements redraw visible rows. They
            // must not reveal an unchanged cursor or reset the scroll offset.
            crate::refresh(&self.window, &self.presenter.borrow());
            self.input(WindowEvent::RedrawRequested);
            self.draw();
        }
    }

    fn input(&self, event: WindowEvent) -> bool {
        observe(&self.window, &self.state, &self.timer, &event)
    }

    fn point(&self, x: f32, y: f32) {
        self.input(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x.into(), y.into()),
        });
    }

    fn wheel(&self, lines: f32) -> bool {
        self.input(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, lines),
            phase: TouchPhase::Moved,
        })
    }

    fn offset(&self, pane: usize) -> f32 {
        let (x, y) = if pane == 0 {
            (
                self.window.get_left_list_left(),
                self.window.get_left_list_top(),
            )
        } else {
            (
                self.window.get_right_list_left(),
                self.window.get_right_list_top(),
            )
        };
        fn find(item: ItemRc, x: f32, y: f32) -> Option<f32> {
            if let Some(flickable) = item.downcast::<Flickable>() {
                let position = item.map_to_window(LogicalPoint::default());
                if (position.x - x).abs() < 1.0 && (position.y - y).abs() < 1.0 {
                    return Some(flickable.as_pin_ref().content_y().get());
                }
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(offset) = find(item, x, y) {
                    return Some(offset);
                }
            }
            None
        }
        find(
            ItemRc::new_root(WindowInner::from_pub(self.window.window()).component()),
            x,
            y,
        )
        .expect("real file-list Flickable")
    }
}

#[test]
fn real_file_panes_interpolate_wheel_during_model_refresh_then_snap_after_idle() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(
        PathBuf::from("left"),
        PathBuf::from("right"),
    )));
    crate::install(&window, presenter.clone(), Rc::new(|_| true));
    for pane in [PaneId::Left, PaneId::Right] {
        let entries: Arc<[Entry]> = (0..1000)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("file-{id:04}.txt")),
                    EntryKind::File,
                )
            })
            .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation: NavigationId(1),
            entries,
            index,
        });
    }
    crate::refresh(&window, &presenter.borrow());
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(1340.0, 820.0),
        });
    let state = Rc::new(RefCell::new(InputState::default()));
    let timer = wheel_timer(&window, &state);
    let h = Harness {
        window,
        native,
        time,
        presenter,
        state,
        timer,
        pixels: RefCell::new(vec![slint::Rgb8Pixel::default(); 1340 * 820]),
    };
    h.draw();
    let left_x = h.window.get_left_list_left() + 100.0;
    let list_y = h.window.get_left_list_top() + 100.0;
    h.point(left_x, list_y);
    assert!(h.wheel(-1.0), "the file list replaces discrete line input");
    h.point(left_x + 1.0, list_y + 1.0);
    assert!(
        h.timer.running(),
        "same-pane pointer jitter preserves a queued notch"
    );
    assert_eq!(
        h.offset(0),
        0.0,
        "a notch queues movement rather than jumping"
    );
    h.time.set(Duration::from_millis(16));
    slint::platform::update_timers_and_animations();
    assert_eq!(
        h.offset(0),
        0.0,
        "timer wakeups cannot spend unpresented wheel frames"
    );
    let mut previous = 0.0;
    for _ in 0..7 {
        h.advance(1);
        let offset = h.offset(0);
        assert!(
            offset < previous && offset > previous - 20.0,
            "fractional frame: {previous} -> {offset}"
        );
        assert!(
            h.window.get_scroll_interaction_active(),
            "no snapping during interpolation"
        );
        assert_eq!(h.offset(1), 0.0, "the other pane remains stationary");
        previous = offset;
        if offset > -20.0 {
            let x = (h.window.get_left_list_left() + 8.0) as usize;
            let top = h.window.get_left_list_top() as usize;
            let row = h.window.global::<Theme>().get_row_height() as usize;
            let pixels = h.pixels.borrow();
            let first = pixels[(top + 1) * 1340 + x];
            assert!(
                (2..row).any(|y| pixels[(top + y) * 1340 + x] != first),
                "rendered row boundary must move through pixel positions inside a row"
            );
        }
    }
    h.advance(6);
    assert!(
        (h.offset(0) + 60.0).abs() < 0.01,
        "all wheel pixels delivered once"
    );
    assert!(!h.window.get_scroll_interaction_active());
    let settling: Vec<_> = (0..24)
        .map(|_| {
            h.advance(1);
            h.offset(0)
        })
        .collect();
    let row = h.window.global::<Theme>().get_row_height();
    let snapped = (-60.0 / row).round() * row;
    assert!(
        (h.offset(0) - snapped).abs() < 0.01,
        "resting position aligns to a row: expected {snapped}, actual {}, row {row}, frames {settling:?}",
        h.offset(0)
    );
    assert!(!h.timer.running(), "no perpetual animation timer");

    assert!(h.wheel(-1.0));
    h.point(h.window.get_right_list_left() + 100.0, list_y);
    assert!(
        !h.timer.running(),
        "cross-pane movement cancels before the next frame"
    );
    h.advance(1);
    assert!((h.offset(0) - snapped).abs() < 0.01);
    assert_eq!(
        h.offset(1),
        0.0,
        "an old notch cannot scroll a different pane"
    );
    h.point(left_x, list_y);

    // A reverse notch changes direction on the next frame, without draining
    // pending downward motion first, even while presenter refreshes continue.
    assert!(h.wheel(-3.0));
    h.advance(1);
    let down = h.offset(0);
    assert!(h.wheel(1.0));
    h.advance(1);
    assert!(h.offset(0) > down);
    h.input(WindowEvent::Focused(false));
    assert!(!h.timer.running());
    h.window.set_left_cursor(0);
    h.window.invoke_reveal_left();
    h.advance(30);
    assert_eq!(
        h.offset(0),
        0.0,
        "explicit cursor reveal cancels residual motion"
    );

    h.window.invoke_view_mode(0, 2);
    h.advance(1);
    h.point(left_x, h.window.get_left_list_top() + 100.0);
    assert!(h.wheel(-1.0));
    h.advance(40);
    assert!(
        h.offset(0) < -60.0,
        "a discrete thumbnail notch must reach another band instead of snapping back"
    );
    h.window.invoke_view_mode(0, 0);
    h.window.invoke_reveal_left();
    h.advance(30);

    // Other scrollable controls keep native behavior. A popup covering the
    // list must never have its input transformed into delayed panel frames.
    h.point(5.0, 5.0);
    assert!(!h.wheel(-1.0));
    h.point(left_x, list_y);
    assert!(h.wheel(-1.0));
    h.window.invoke_show_options();
    h.advance(1);
    assert!(
        !h.timer.running(),
        "opening a popup cancels queued panel motion"
    );
    assert_eq!(h.offset(0), 0.0);
    assert!(
        !h.wheel(-1.0),
        "popup input remains native even above the list"
    );
}
