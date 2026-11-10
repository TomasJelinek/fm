//! Background pointer ownership and bounded visible-row repainting only.
use crate::presenter::selection_rectangle::Capture;
use crate::{MainWindow, Presenter, Rows, SelectionRectangle, pane, selection_rectangle::Grid};
use i_slint_core::window::{WindowInner, WindowKind};
use midnight_core::*;
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    grid: Grid,
    left: f32,
    top: f32,
    height: f32,
    viewport_width: f32,
    content_x: f32,
}
struct Gesture {
    pane: PaneId,
    capture: Capture,
    layout: Layout,
    anchor: (f32, f32),
    pointer: (f32, f32),
    mode: SelectionMode,
    moved: bool,
}
struct Bridge {
    window: slint::Weak<MainWindow>,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    gesture: RefCell<Option<Gesture>>,
    cancel_pending: Cell<bool>,
    timer: slint::Timer,
}

fn layout(window: &MainWindow, presenter: &Presenter, pane: PaneId) -> (Layout, f32) {
    let p = &presenter.panes[pane.index()];
    let (left, top, width, height, offset, mode, columns, page_rows, row_height) =
        if pane == PaneId::Left {
            (
                window.get_left_list_left(),
                window.get_left_list_top(),
                window.get_left_list_width(),
                window.get_left_list_height(),
                window.get_left_list_content_y(),
                window.get_left_view_mode(),
                window.get_left_view_columns(),
                window.get_left_view_rows(),
                window.get_left_view_row_height(),
            )
        } else {
            (
                window.get_right_list_left(),
                window.get_right_list_top(),
                window.get_right_list_width(),
                window.get_right_list_height(),
                window.get_right_list_content_y(),
                window.get_right_view_mode(),
                window.get_right_view_columns(),
                window.get_right_view_rows(),
                window.get_right_view_row_height(),
            )
        };
    (
        Layout {
            grid: Grid {
                mode,
                columns: columns.max(0) as usize,
                page_rows: page_rows.max(0) as usize,
                count: p.display_len() + p.parent_offset(),
                parent: p.parent_offset(),
                width: if pane == PaneId::Left {
                    window.get_left_list_grid_width()
                } else {
                    window.get_right_list_grid_width()
                },
                row_height,
            },
            left,
            top,
            height,
            viewport_width: width,
            content_x: if pane == PaneId::Left {
                window.get_left_list_content_x()
            } else {
                window.get_right_list_content_x()
            },
        },
        offset,
    )
}
fn available(window: &MainWindow) -> bool {
    window.window().is_visible()
        && !window.get_touch_contact()
        && !window.get_drag_active()
        && window.get_copy_draft_token().is_empty()
        && window.get_operation_token().is_empty()
        && !WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
}

/// Reuse selection geometry for drag targets. None rejects gutters/outside points;
/// Some(None) represents empty content inside the receiving panel.
pub(crate) fn drop_row_at(
    window: &MainWindow,
    presenter: &Presenter,
    pane: PaneId,
    point: (f32, f32),
) -> Option<Option<usize>> {
    let (layout, offset) = layout(window, presenter, pane);
    let x = point.0 - layout.left - layout.content_x;
    let y = point.1 - layout.top;
    if ![x, y, offset].into_iter().all(f32::is_finite)
        || x < 0.0
        || x >= layout.grid.width
        || y < 0.0
        || y >= layout.height
    {
        return None;
    }
    let point = (x, y - offset);
    layout
        .grid
        .row_at(point)
        .map(Some)
        .or_else(|| layout.grid.empty_at(point).then_some(None))
}
fn repaint(window: &MainWindow, presenter: &Rc<RefCell<Presenter>>, pane: PaneId) {
    let (model, first, end) = if pane == PaneId::Left {
        (
            window.get_left_rows(),
            window.get_left_visible_row_start(),
            window.get_left_visible_row_end(),
        )
    } else {
        (
            window.get_right_rows(),
            window.get_right_visible_row_start(),
            window.get_right_visible_row_end(),
        )
    };
    if let Some(rows) = model.as_any().downcast_ref::<Rows>() {
        rows.refresh(
            &presenter.borrow().panes[pane.index()],
            first.max(0) as usize,
            end.max(0) as usize,
            true,
        );
    }
}
impl Bridge {
    fn clear(&self, window: &MainWindow, pane: PaneId) {
        self.cancel_pending.set(false);
        self.timer.stop();
        self.presenter.borrow_mut().cancel_rectangle(pane);
        window.global::<SelectionRectangle>().set_active(false);
        window.global::<SelectionRectangle>().set_pane(-1);
        repaint(window, &self.presenter, pane);
    }
    fn cancel(&self, window: &MainWindow) -> bool {
        let gesture = self.gesture.borrow_mut().take();
        if let Some(gesture) = gesture {
            gesture.capture.cancel();
            if self.presenter.try_borrow_mut().is_err() {
                *self.gesture.borrow_mut() = Some(gesture);
                self.cancel_pending.set(true);
                window.global::<SelectionRectangle>().set_active(false);
                return true;
            }
            self.clear(window, gesture.pane);
            true
        } else {
            false
        }
    }
    fn update(&self, window: &MainWindow, scroll: bool, release: bool) -> bool {
        // No RefCell guard survives Slint setters, callbacks or model notifications.
        let Some(mut gesture) = self.gesture.borrow_mut().take() else {
            return false;
        };
        let Ok(presenter) = self.presenter.try_borrow() else {
            *self.gesture.borrow_mut() = Some(gesture);
            return false;
        };
        let (current, mut offset) = layout(window, &presenter, gesture.pane);
        let valid = presenter.rectangle_current(gesture.pane, &gesture.capture);
        drop(presenter);
        if self.cancel_pending.replace(false)
            || !available(window)
            || current != gesture.layout
            || !valid
            || window.get_active_pane() != gesture.pane.index() as i32
            || window.get_focused_pane() != gesture.pane.index() as i32
        {
            self.clear(window, gesture.pane);
            return false;
        }
        if scroll && gesture.moved {
            let y = gesture.pointer.1 - current.top;
            let edge = 24.0_f32.min(current.height / 3.0);
            let step = (current.grid.row_height * 0.4).clamp(8.0, 64.0);
            let delta = if y < edge {
                step
            } else if y > current.height - edge {
                -step
            } else {
                0.0
            };
            if delta != 0.0 {
                window.invoke_scroll_selection(gesture.pane.index() as i32, delta);
                offset = layout(window, &self.presenter.borrow(), gesture.pane).1;
            }
        }
        let pointer = (
            gesture.pointer.0 - current.left,
            gesture.pointer.1 - current.top - offset,
        );
        let distance = (
            (pointer.0 - gesture.anchor.0).abs(),
            (pointer.1 - gesture.anchor.1).abs(),
        );
        gesture.moved |= distance.0 >= 6.0 || distance.1 >= 6.0;
        if !gesture.moved {
            if release {
                self.clear(window, gesture.pane);
            } else {
                *self.gesture.borrow_mut() = Some(gesture);
            }
            return true;
        }
        let ranges = match current.grid.ranges(gesture.anchor, pointer) {
            Ok(ranges) => ranges,
            Err(error) => {
                self.presenter.borrow_mut().panes[gesture.pane.index()].error = error.into();
                self.clear(window, gesture.pane);
                crate::refresh(window, &self.presenter.borrow());
                return false;
            }
        };
        if release {
            self.presenter.borrow_mut().commit_rectangle(
                gesture.pane,
                &gesture.capture,
                ranges,
                gesture.mode,
                self.send.as_ref(),
            );
            self.clear(window, gesture.pane);
            crate::refresh(window, &self.presenter.borrow());
            return true;
        }
        if !self.presenter.borrow_mut().preview_rectangle(
            gesture.pane,
            &gesture.capture,
            ranges,
            gesture.mode,
        ) {
            self.clear(window, gesture.pane);
            return false;
        }
        let x1 = gesture.anchor.0.clamp(0.0, current.grid.width);
        let x2 = pointer.0.clamp(0.0, current.grid.width);
        let y1 = (gesture.anchor.1 + offset).clamp(0.0, current.height);
        let y2 = (gesture.pointer.1 - current.top).clamp(0.0, current.height);
        let pane = gesture.pane;
        *self.gesture.borrow_mut() = Some(gesture);
        let global = window.global::<SelectionRectangle>();
        global.set_left(current.left + x1.min(x2));
        global.set_top(current.top + y1.min(y2));
        global.set_width((x1 - x2).abs());
        global.set_height((y1 - y2).abs());
        global.set_active(true);
        repaint(window, &self.presenter, pane);
        true
    }
    fn input(
        self: &Rc<Self>,
        phase: i32,
        id: i32,
        x: f32,
        y: f32,
        control: bool,
        shift: bool,
    ) -> bool {
        let Some(window) = self.window.upgrade() else {
            return false;
        };
        if phase == 3 {
            return self.cancel(&window);
        }
        if phase == 0 {
            self.cancel(&window);
            if self.cancel_pending.get() {
                return false;
            }
            if !(0..2).contains(&id) || !x.is_finite() || !y.is_finite() || !available(&window) {
                return false;
            }
            let pane = pane(id);
            let p = self.presenter.borrow();
            let (layout, offset) = layout(&window, &p, pane);
            let point = (x - layout.left, y - layout.top - offset);
            if y < layout.top
                || y >= layout.top + layout.height
                || point.0 < 0.0
                || point.0 >= layout.viewport_width
                || layout.content_x.abs() > 0.01
                || !(point.0 >= layout.grid.width || layout.grid.empty_at(point))
            {
                return false;
            }
            let Some(capture) = p.capture_rectangle(pane) else {
                return false;
            };
            drop(p);
            *self.gesture.borrow_mut() = Some(Gesture {
                pane,
                capture,
                layout,
                anchor: point,
                pointer: (x, y),
                moved: false,
                mode: if shift {
                    SelectionMode::Add
                } else if control {
                    SelectionMode::Toggle
                } else {
                    SelectionMode::Replace
                },
            });
            window.global::<SelectionRectangle>().set_pane(id);
            let weak = Rc::downgrade(self);
            self.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(30),
                move || {
                    if let Some(state) = weak.upgrade()
                        && let Some(window) = state.window.upgrade()
                    {
                        state.update(&window, true, false);
                    }
                },
            );
            return true;
        }
        if let Some(gesture) = self.gesture.borrow_mut().as_mut() {
            gesture.pointer = (x, y);
        }
        self.update(&window, false, phase == 2)
    }
}
pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let bridge = Rc::new(Bridge {
        window: window.as_weak(),
        presenter,
        send,
        gesture: RefCell::new(None),
        cancel_pending: Cell::new(false),
        timer: slint::Timer::default(),
    });
    let state = bridge.clone();
    window
        .global::<SelectionRectangle>()
        .on_input(move |phase, pane, x, y, control, shift| {
            state.input(phase, pane, x, y, control, shift)
        });
    let weak = window.as_weak();
    window
        .global::<SelectionRectangle>()
        .on_cancel(move || weak.upgrade().is_some_and(|window| bridge.cancel(&window)));
}
