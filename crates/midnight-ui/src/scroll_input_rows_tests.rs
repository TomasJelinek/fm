//! Row coverage of the real panels: no offset, view or gesture leaves a blank band.
use super::*;
use i_slint_core::items::AccessibleRole;

/// About the size of `C:\Windows\System32`: folders (with tree expanders) then files.
fn system32_like() -> Arc<[Entry]> {
    folder_of(5104)
}

fn folder_of(count: u64) -> Arc<[Entry]> {
    (0..count)
        .map(|id| {
            if id < 120 {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("Folder {id:04}")),
                    EntryKind::Directory,
                )
            } else {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("library-{id:05}.dll")),
                    EntryKind::File,
                )
            }
        })
        .collect()
}

impl Harness {
    /// The list viewport of `pane` in window coordinates: left, top, right, bottom.
    fn list_bounds(&self, pane: usize) -> (f32, f32, f32, f32) {
        let w = &self.window;
        let (left, top, width, height) = if pane == 0 {
            (
                w.get_left_list_left(),
                w.get_left_list_top(),
                w.get_left_list_width(),
                w.get_left_list_height(),
            )
        } else {
            (
                w.get_right_list_left(),
                w.get_right_list_top(),
                w.get_right_list_width(),
                w.get_right_list_height(),
            )
        };
        (left, top, left + width, top + height)
    }

    /// Window-space vertical spans of the row cells shown in `pane`'s list,
    /// clipped to its viewport: what the virtualized repeater instantiated and
    /// left visible.
    fn row_spans(&self, pane: usize) -> Vec<(f32, f32)> {
        let bounds = self.list_bounds(pane);
        let mut spans = Vec::new();
        fn visit(item: ItemRc, bounds: (f32, f32, f32, f32), spans: &mut Vec<(f32, f32)>) {
            if item.accessible_role() == AccessibleRole::ListItem && item.is_visible() {
                let geometry = item.geometry();
                let origin = item.map_to_window(geometry.origin);
                let (left, top, right, bottom) = bounds;
                let (y0, y1) = (origin.y, origin.y + geometry.height());
                if origin.x >= left - 0.5
                    && origin.x < right
                    && y1 > top
                    && y0 < bottom
                    && geometry.width() > 0.0
                {
                    spans.push((y0.max(top), y1.min(bottom)));
                }
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                visit(item, bounds, spans);
            }
        }
        visit(
            ItemRc::new_root(WindowInner::from_pub(self.window.window()).component()),
            bounds,
            &mut spans,
        );
        spans
    }

    /// The first shown row reaches `pane`'s list top: no blank band below the
    /// column header.
    #[track_caller]
    fn assert_top_covered(&self, pane: usize, context: &str) {
        let (_, top, ..) = self.list_bounds(pane);
        let first = self
            .row_spans(pane)
            .into_iter()
            .map(|(start, _)| start)
            .fold(f32::INFINITY, f32::min);
        assert!(
            first <= top + 0.01,
            "{context}: blank band of {:.2} px below the column header (offset {})",
            first - top,
            self.offset(pane)
        );
    }

    /// The shown rows cover `pane`'s whole list viewport: no blank band at its
    /// top, between rows or at its bottom.
    #[track_caller]
    fn assert_rows_cover(&self, pane: usize, context: &str) {
        let (_, top, _, bottom) = self.list_bounds(pane);
        let mut spans = self.row_spans(pane);
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reached = top;
        for &(start, end) in &spans {
            assert!(
                start <= reached + 0.01,
                "{context}: blank band from {:.2} to {:.2} px below the list top \
                 (offset {}, rows {spans:?})",
                reached - top,
                start - top,
                self.offset(pane)
            );
            reached = reached.max(end);
        }
        assert!(
            reached >= bottom - 0.01,
            "{context}: blank band from {:.2} px below the list top to its bottom {:.2} \
             (offset {}, rows {spans:?})",
            reached - top,
            bottom - top,
            self.offset(pane)
        );
    }

    /// Move `pane`'s list to `offset` (a content-y) as a program would.
    fn scroll_to(&self, pane: usize, offset: f32) {
        self.window
            .invoke_scroll_selection(pane as i32, offset - self.offset(pane));
        self.draw();
    }

    fn row_height(&self, pane: usize) -> f32 {
        if pane == 0 {
            self.window.get_left_view_row_height()
        } else {
            self.window.get_right_view_row_height()
        }
    }
}

fn wait_frame(h: &Harness) {
    h.time.set(h.time.get() + Duration::from_millis(8));
    slint::platform::update_timers_and_animations();
}

fn on_row(offset: f32, row: f32) -> bool {
    (offset / row - (offset / row).round()).abs() < 1e-3
}

/// The reported state: the Details view of a System32-sized folder, scrolled with
/// a Windows precision touchpad (fractional wheel lines; the system adds momentum)
/// to the middle, where the list came to rest a little past a row boundary. The
/// row cut by the column header was not drawn: a blank band about one row tall
/// separated the header from the first row. Both panels, every frame.
#[test]
fn a_touchpad_scroll_resting_mid_folder_leaves_no_blank_band_below_the_header() {
    let h = harness_entries(false, false, system32_like);
    for pane in 0..2 {
        let (left, top, ..) = h.list_bounds(pane);
        h.point(left + 100.0, top + 100.0);
        let row = h.row_height(pane);
        // The middle of the folder, at a row boundary.
        h.scroll_to(pane, -(2552.0 * row));
        h.advance(1);
        h.assert_top_covered(pane, "a row boundary");
        // Two reports of a fortieth of a line: the list stops a few pixels past the
        // boundary, short of a row.
        for _ in 0..2 {
            wait_frame(&h);
            assert!(h.wheel(-0.025), "the panel takes the touchpad's lines");
        }
        for frame in 0..80 {
            h.advance(1);
            let context = format!("panel {pane}, frame {frame}");
            h.assert_top_covered(pane, &context);
            h.assert_rows_cover(pane, &context);
        }
        h.settle(60);
        h.assert_rows_cover(pane, "at rest");
        assert!(
            on_row(h.offset(pane), row),
            "the list settles on the nearest row: {}",
            h.offset(pane)
        );
    }
}

/// Every fractional offset, row height and scale: the rows shown cover the list
/// viewport in the Details, Brief and Thumbnails views.
#[test]
fn rows_cover_the_list_at_every_fractional_offset_in_each_view() {
    let h = harness_entries(false, false, system32_like);
    for scale in [1.0f32, 1.5, 2.0] {
        h.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        h.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::Resized {
                size: slint::LogicalSize::new(1340.0 / scale, 820.0 / scale),
            });
        for font in [12, 15, 19] {
            h.window.global::<Theme>().set_desktop_font_size(font);
            for mode in 0..3 {
                h.window.invoke_view_mode(0, mode);
                h.advance(1);
                let row = h.row_height(0);
                // A row boundary in the middle of the folder, then fractions of a row.
                let base = -(row * (700.0 / (mode + 1) as f32).floor());
                h.scroll_to(0, base);
                for step in 0..=32 {
                    let fraction = row * step as f32 / 32.0;
                    let offset = base - fraction - if step == 0 { 0.0 } else { 0.01 };
                    h.scroll_to(0, offset);
                    h.assert_rows_cover(
                        0,
                        &format!("scale {scale}, font {font}, view {mode}, offset {offset}"),
                    );
                }
            }
        }
    }
}

/// Keyboard navigation reveals the cursor row at the bottom edge, which can cut
/// the top row: it is drawn. Refreshes that add or remove rows and a new sort
/// while the list is scrolled keep it covered too.
#[test]
fn reveals_refreshes_and_sorting_keep_the_rows_covering_the_list() {
    let h = harness_entries(false, false, system32_like);
    let (left, top, ..) = h.list_bounds(0);
    h.point(left + 100.0, top + 100.0);
    for view in 0..3 {
        h.window.invoke_view_mode(0, view);
        h.advance(1);
        // Down past the bottom edge, PageDown, End, Home, PageUp, Up.
        for (key, presses) in [(3, 40), (5, 3), (7, 1), (6, 1), (5, 2), (4, 1), (2, 3)] {
            for _ in 0..presses {
                h.window.invoke_key(0, key);
                h.advance(1);
                h.assert_rows_cover(0, &format!("view {view}, key {key}"));
            }
        }
    }
    h.window.invoke_view_mode(0, 0);
    h.advance(1);
    // A fractional offset held by touchpad input, then the listing changes under it.
    h.scroll_to(0, -(1200.0 * h.row_height(0)));
    for _ in 0..3 {
        wait_frame(&h);
        assert!(h.wheel(-0.05));
    }
    h.advance(3);
    for (context, count) in [("rows added", 5600u64), ("rows removed", 5104)] {
        // A refresh whose listing adds or removes rows while the list is scrolled.
        h.window.invoke_action(0, 3);
        let navigation = NavigationId(h.presenter.borrow().panes[0].generation);
        let entries = folder_of(count);
        h.presenter.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        crate::refresh(&h.window, &h.presenter.borrow());
        for frame in 0..30 {
            h.advance(1);
            h.assert_rows_cover(0, &format!("{context}, frame {frame}"));
        }
    }
    h.window.invoke_sort(0, 2);
    for frame in 0..30 {
        h.advance(1);
        h.assert_rows_cover(0, &format!("sorted, frame {frame}"));
    }
}

/// The color the second row's model state calls for: the selection fill wins over
/// the cursor fill, then its stripe (`file-entry.slint` `row-color`).
fn second_row_color(h: &Harness) -> slint::Color {
    let theme = h.window.global::<Theme>();
    let row = h.window.get_left_rows().row_data(1).unwrap_or_default();
    if row.selected {
        theme.get_selected_background()
    } else if row.cursor && h.window.get_active_pane() == 0 {
        theme.get_cursor_background()
    } else if row.cursor {
        theme.get_inactive_cursor()
    } else {
        theme.get_alternate_row()
    }
}

#[track_caller]
fn assert_second_row_paints_its_state(h: &Harness, phase: &str) {
    let (left, top, ..) = h.list_bounds(0);
    let row = h.row_height(0);
    let (x, y) = ((left + 8.0) as usize, (top + 1.5 * row) as usize);
    let pixel = h.pixels.borrow()[y * 1340 + x];
    let expected = second_row_color(h);
    let model = h.window.get_left_rows().row_data(1).unwrap_or_default();
    assert_eq!(
        (pixel.r, pixel.g, pixel.b),
        (expected.red(), expected.green(), expected.blue()),
        "{phase}: row 1 selected={} cursor={}",
        model.selected,
        model.cursor
    );
}

/// The native probe's row check after its flings, in software rendering: each row
/// paints the state its model gives it, with no stale cursor or selection fill. A
/// Ctrl+click toggles the row's selection, so it paints the selection fill.
#[test]
fn rows_paint_their_model_state_after_flings() {
    let h = harness_entries(true, false, system32_like);
    let (left, top, ..) = h.list_bounds(0);
    h.point(left + 100.0, top + 100.0);
    // The probe's flick: 12 finger reports of 0.8 line, then 80 momentum reports
    // decaying by 5 % each, with a frame after every second report.
    let reports = std::iter::repeat_n(-0.8f32, 12)
        .chain((0..80).map(|k| -((96.0 * 0.95f64.powi(k)).round() / 120.0) as f32))
        .filter(|lines| *lines != 0.0);
    for (index, lines) in reports.enumerate() {
        h.time.set(h.time.get() + Duration::from_millis(8));
        assert!(h.wheel(lines));
        if index % 2 == 1 {
            h.advance(1);
        }
    }
    h.settle(100);
    assert!(h.offset(0) < -1000.0, "the flick scrolled: {}", h.offset(0));
    h.window.invoke_view_mode(0, 0);
    h.window.set_left_cursor(0);
    h.window.invoke_reveal_left();
    h.settle(60);
    assert_eq!(h.offset(0), 0.0);
    assert_second_row_paints_its_state(&h, "ordinary row");
    h.window.invoke_activate(0);
    h.window.invoke_row_click(0, 1, true, false);
    h.advance(2);
    assert!(
        h.window.get_left_rows().row_data(1).unwrap().selected,
        "Ctrl+click selects the row"
    );
    assert_second_row_paints_its_state(&h, "selected cursor row");
    h.window.global::<Theme>().set_scheme(2);
    h.advance(2);
    assert_second_row_paints_its_state(&h, "changed theme");
}
