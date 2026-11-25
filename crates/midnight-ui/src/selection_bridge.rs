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

impl Layout {
    fn row_at(
        self,
        window: &MainWindow,
        pane: PaneId,
        point: (f32, f32),
        content_y: f32,
    ) -> Option<Option<usize>> {
        let (x, y) = point;
        let content_point = (x, y - content_y);
        self.grid
            .row_at(content_point)
            .map(|row| {
                window
                    .invoke_file_row_band_visible(pane.index() as i32, y)
                    .then_some(row)
            })
            .or_else(|| self.grid.empty_at(content_point).then_some(None))
    }
}

struct Gesture {
    pane: PaneId,
    capture: Capture,
    layout: Layout,
    anchor: (f32, f32),
    pointer: (f32, f32),
    mode: SelectionMode,
    moved: bool,
    /// The ranges of the last applied preview. The 30 ms autoscroll tick and
    /// pointer moves inside one cell keep them; an unchanged preview does not
    /// re-notify every visible row.
    previewed: Option<SelectionRanges>,
}

fn same_ranges(left: &SelectionRanges, right: &SelectionRanges) -> bool {
    left.source_len() == right.source_len() && left.spans() == right.spans()
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
                window.get_left_list_row_offset_y(),
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
                window.get_right_list_row_offset_y(),
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
        && !window.get_dialog_input_blocked()
        && !window.get_embedded_modal_open()
        && !window.get_touch_contact()
        && !window.get_drag_active()
        && window.get_copy_draft_token().is_empty()
        && window.get_operation_token().is_empty()
        && !WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
}

/// Reuse selection geometry for drag targets. None rejects points outside the list;
/// Some(None) is the receiving panel's own folder: empty content, and the gutters
/// beside the rows (the scrollbar's strip at the right edge), so that a folder full
/// of subfolders still takes a drop into itself.
pub(crate) fn drop_row_at(
    window: &MainWindow,
    presenter: &Presenter,
    pane: PaneId,
    point: (f32, f32),
) -> Option<Option<usize>> {
    let (layout, offset) = layout(window, presenter, pane);
    let shown = point.0 - layout.left;
    let x = shown - layout.content_x;
    let y = point.1 - layout.top;
    if ![x, y, offset].into_iter().all(f32::is_finite)
        || shown < 0.0
        || shown >= layout.viewport_width
        || y < 0.0
        || y >= layout.height
    {
        return None;
    }
    if x < 0.0 || x >= layout.grid.width {
        return Some(None);
    }
    layout.row_at(window, pane, (x, y), offset)
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
        // Release the presenter before notifying: delegates read their rows.
        let (structure, edits) = {
            let presenter = presenter.borrow();
            let pane = &presenter.panes[pane.index()];
            (
                crate::RowStructure::from_pane(pane),
                pane.row_edits().cloned(),
            )
        };
        rows.refresh_structure(
            structure,
            edits.as_ref(),
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
        let details = crate::panel_models::details_rows(&self.presenter.borrow(), pane);
        crate::panel_models::refresh_details(window, details, pane);
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
        // This path will mutate preview/selection or finish deferred cancellation.
        // A shared refresh borrow permits reads but must defer all of that work.
        let Ok(presenter) = self.presenter.try_borrow_mut() else {
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
        let changed = !gesture
            .previewed
            .as_ref()
            .is_some_and(|previewed| same_ranges(previewed, &ranges));
        // Both calls return early for an unchanged preview. The flush also
        // retries a summary request that a full command queue refused.
        if !self.presenter.borrow_mut().preview_rectangle(
            gesture.pane,
            &gesture.capture,
            ranges.clone(),
            gesture.mode,
        ) {
            self.clear(window, gesture.pane);
            return false;
        }
        self.presenter
            .borrow_mut()
            .flush_rectangle_summary(gesture.pane, self.send.as_ref());
        if changed {
            gesture.previewed = Some(ranges);
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
        if changed {
            repaint(window, &self.presenter, pane);
        }
        // The few status rows are compared before they are replaced; keep them
        // current even when the previewed ranges did not change.
        let details = crate::panel_models::details_rows(&self.presenter.borrow(), pane);
        crate::panel_models::refresh_details(window, details, pane);
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
            let point = (x - layout.left, y - layout.top);
            if y < layout.top
                || y >= layout.top + layout.height
                || point.0 < 0.0
                || point.0 >= layout.viewport_width
                || layout.content_x.abs() > 0.01
                || !(point.0 >= layout.grid.width
                    || layout.row_at(&window, pane, point, offset) == Some(None))
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
                anchor: (point.0, point.1 - offset),
                pointer: (x, y),
                moved: false,
                previewed: None,
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

#[cfg(test)]
mod tests {
    use super::drop_row_at;
    use crate::{SelectionRectangle, test_harness::Harness};
    use i_slint_core::model::{ModelChangeListener, ModelChangeListenerBox};
    use midnight_core::*;
    use slint::{ComponentHandle, Model};
    use std::{cell::Cell, pin::Pin, sync::Arc};

    #[derive(Default)]
    struct Notifications(Cell<usize>);
    impl Notifications {
        fn count(&self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl ModelChangeListener for Notifications {
        fn row_changed(self: Pin<&Self>, _: usize) {
            self.count();
        }
        fn row_added(self: Pin<&Self>, _: usize, _: usize) {
            self.count();
        }
        fn row_removed(self: Pin<&Self>, _: usize, _: usize) {
            self.count();
        }
        fn reset(self: Pin<&Self>) {
            self.count();
        }
    }

    /// The left pane lists `count` files and its empty selection is acknowledged.
    fn listed(count: u64) -> Harness {
        let mut h = Harness::new(UserPreferences::default());
        let navigation = h
            .sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .expect("startup navigation");
        let root = h.presenter.borrow().panes[0].display_path().to_path_buf();
        let entries: Arc<[Entry]> = (0..count)
            .map(|row| {
                Entry::new(
                    EntryId(100 + row),
                    root.join(format!("{row}.txt")),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into();
        let sent = h.sent.clone();
        h.presenter.borrow_mut().apply_with_sender(
            Event::ListingFinished {
                pane: PaneId::Left,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            },
            &|command| {
                sent.borrow_mut().push(command);
                true
            },
        );
        let request = h
            .sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Selection(request) if request.pane == PaneId::Left => {
                    Some(request.clone())
                }
                _ => None,
            })
            .expect("listing requests a backend selection");
        let index = request.view.clone().expect("complete listing");
        let bits = vec![0; index.by_id.len().div_ceil(64)];
        h.presenter.borrow_mut().apply(Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: request.navigation,
            revision: request.revision,
            selection: Arc::new(SelectionSnapshot::from_bits(index, bits).unwrap()),
            deferred: Default::default(),
        });
        h.sent.borrow_mut().clear();
        h.refresh();
        h
    }

    fn summaries(h: &Harness) -> usize {
        h.sent
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::SummarizeRectangle(_)))
            .count()
    }

    #[test]
    fn a_row_cut_by_the_list_top_is_its_own_drop_target() {
        let h = listed(100);
        let w = &h.window;
        let x = w.get_left_list_left() + 20.0;
        let top = w.get_left_list_top();
        let row_height = w.get_left_view_row_height();
        w.invoke_scroll_selection(0, -row_height / 2.0);
        assert_eq!(w.get_left_list_content_y(), -row_height / 2.0);
        let presenter = h.presenter.borrow();

        // The list top cuts the first row in half; its visible half is drawn and
        // takes drops like the rest of the row.
        assert_eq!(
            drop_row_at(w, &presenter, PaneId::Left, (x, top + 1.0)),
            Some(Some(0))
        );
        assert_eq!(
            drop_row_at(w, &presenter, PaneId::Left, (x, top + row_height)),
            Some(Some(1))
        );
    }

    #[test]
    fn a_band_held_inside_one_cell_notifies_no_rows_but_retries_a_refused_summary() {
        let h = listed(5);
        let w = &h.window;
        let left = w.get_left_list_left();
        let top = w.get_left_list_top();
        let row = w.get_left_view_row_height();
        // Empty space below the five rows starts the gesture.
        let start = (left + 20.0, top + w.get_left_list_height() - 35.0);
        h.pointer(1, start.0, start.1);
        h.pointer(0, start.0, start.1);
        assert_eq!(w.global::<SelectionRectangle>().get_pane(), 0);

        let rows = w.get_left_rows();
        let probe = ModelChangeListenerBox::new(Notifications::default());
        rows.model_tracker()
            .attach_peer(probe.as_ref().model_peer());
        // The band needs a width to touch any cell.
        h.refuse.set(true);
        h.pointer(1, left + 60.0, top + 2.5 * row);
        assert!(w.global::<SelectionRectangle>().get_active());
        assert!(probe.0.get() > 0, "a new preview repaints the visible rows");
        assert_eq!(summaries(&h), 0, "the full queue refused the summary");

        probe.0.set(0);
        h.refuse.set(false);
        h.pointer(1, left + 61.0, top + 2.5 * row + 1.0);
        assert_eq!(probe.0.get(), 0, "the unchanged preview repaints no row");
        assert_eq!(summaries(&h), 1, "the refused summary is retried");
        h.pointer(1, left + 62.0, top + 2.5 * row + 2.0);
        assert_eq!(summaries(&h), 1, "a submitted summary is not repeated");

        h.pointer(1, left + 60.0, top + 1.5 * row);
        assert!(probe.0.get() > 0, "crossing into another row repaints");
        assert_eq!(summaries(&h), 2, "the new preview has its own summary");
        drop(probe);
        h.pointer(2, left + 60.0, top + 1.5 * row);
        assert_eq!(w.global::<SelectionRectangle>().get_pane(), -1);
    }
}
