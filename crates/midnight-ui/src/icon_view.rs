//! Where the main window draws native icons, read from the window itself.
use crate::*;

/// The Slint index of `PanelViewMode::Thumbnails`.
const THUMBNAILS: i32 = 2;
/// The largest box, in logical pixels, that draws native action art: the title icon
/// of a dialog frame (`dialog-frame.slint`). Toolbars and menus draw 14 to 24 px.
const ACTION_BOX: f32 = 32.0;

/// The device-pixel edges at which `window` draws native icons right now, and
/// the panes whose entries are tiles. Density, font size, thumbnail size, view
/// modes, a compact window and the display's scale factor all change them; the
/// caller passes the result to `Presenter::sync_icon_view` whenever one may have.
pub fn icon_view(window: &MainWindow) -> IconView {
    let scale = window.window().scale_factor();
    let theme = window.global::<Theme>();
    // A compact window lists every pane in rows.
    let tiles = [window.get_left_view_mode(), window.get_right_view_mode()]
        .map(|mode| mode == THUMBNAILS && !window.get_compact_window());
    IconView {
        edges: IconEdges {
            entry: device_edge(theme.get_file_icon_size(), scale),
            tile: if tiles.contains(&true) {
                device_edge(window.get_thumbnail_size() as f32, scale)
            } else {
                0
            },
            location: device_edge(theme.get_location_icon_size(), scale),
            action: device_edge(ACTION_BOX, scale),
        },
        tiles,
    }
}

/// The longest side, in logical pixels, at which each pane's Thumbnails tiles draw a
/// preview (`ThumbnailTile` in `file-entry.slint`): it follows the tile's width and
/// the thumbnail size. 0 for a pane without tiles. The thumbnail pulse passes it to
/// `Presenter::set_tile_preview_edges`; the scale factor applies later, per request.
pub fn tile_preview_edges(window: &MainWindow) -> [u32; 2] {
    [
        window.get_left_preview_edge(),
        window.get_right_preview_edge(),
    ]
    .map(|edge| {
        if edge.is_finite() && edge > 0.0 {
            edge.ceil().min(f32::from(u16::MAX)) as u32
        } else {
            0
        }
    })
}

/// A box of `logical` pixels in device pixels. A bitmap is drawn one to one
/// only in a box of whole device pixels; any other box gets the nearest edge.
fn device_edge(logical: f32, scale: f32) -> u16 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    // `max` and `min` also turn a non-finite product into a valid edge.
    (logical * scale)
        .round()
        .max(1.0)
        .min(f32::from(MAX_ICON_EDGE)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_harness::Harness;
    use slint::platform::WindowEvent;

    #[test]
    fn device_edges_are_whole_bounded_pixels() {
        assert_eq!(device_edge(20.0, 1.0), 20);
        assert_eq!(device_edge(20.0, 1.25), 25);
        assert_eq!(device_edge(20.0, 1.5), 30);
        assert_eq!(device_edge(14.0, 1.25), 18, "17.5 has no exact bitmap");
        assert_eq!(device_edge(96.0, 2.0), 192);
        assert_eq!(device_edge(256.0, 2.0), MAX_ICON_EDGE);
        assert_eq!(device_edge(0.2, 1.0), 1);
        assert_eq!(device_edge(f32::NAN, 1.0), 1);
        for scale in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(device_edge(20.0, scale), 20, "{scale}");
        }
    }

    #[test]
    fn edges_follow_density_view_modes_window_size_and_scale() {
        let mut h = Harness::new(UserPreferences::default());
        let rows = IconView {
            edges: IconEdges {
                entry: 20,
                tile: 0,
                location: 20,
                action: 32,
            },
            tiles: [false, false],
        };
        assert_eq!(icon_view(&h.window), rows);

        // The right pane shows tiles of the preferred thumbnail size.
        h.presenter.borrow_mut().preferences.values.panel_view_modes[1] = PanelViewMode::Thumbnails;
        h.refresh();
        let tiles = IconView {
            edges: IconEdges {
                tile: 96,
                ..rows.edges
            },
            tiles: [false, true],
        };
        assert_eq!(icon_view(&h.window), tiles);
        h.presenter.borrow_mut().preferences.values.thumbnail_size = 128;
        h.refresh();
        assert_eq!(icon_view(&h.window).edges.tile, 128);

        // A compact window lists both panes in rows.
        h.resize(600, 400);
        assert_eq!(icon_view(&h.window), rows);
        h.resize(1340, 820);
        assert_eq!(icon_view(&h.window).tiles, [false, true]);

        // Touch density draws larger rows and location buttons.
        h.presenter.borrow_mut().preferences.values.density = UiDensity::Touch;
        h.refresh();
        let touch = icon_view(&h.window).edges;
        assert_eq!((touch.entry, touch.location), (38, 26));

        // A denser display doubles every box. Nothing is drawn afterwards: the
        // harness renders into a buffer of the original size.
        h.window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
        let dense = icon_view(&h.window).edges;
        assert_eq!((dense.entry, dense.location), (76, 52));
        assert_eq!(dense.action, 64, "action art follows the scale as well");
        h.window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.25 });
        assert_eq!(icon_view(&h.window).edges.action, 72);
    }

    #[test]
    fn edges_are_sent_once_and_again_after_a_refused_send() {
        let mut p = Presenter::new("left".into(), "right".into());
        let sent = std::cell::RefCell::new(Vec::new());
        let accept = std::cell::Cell::new(false);
        let send = |command: Command| {
            if accept.get() {
                sent.borrow_mut().push(command);
            }
            accept.get()
        };
        let view = IconView {
            edges: IconEdges {
                entry: 20,
                tile: 96,
                location: 20,
                action: 32,
            },
            tiles: [false, true],
        };
        // The backend's queue is full: lookups follow the window at once, and
        // the edges are offered again on the next pass.
        assert!(p.sync_icon_view(view, &send));
        assert!(!p.sync_icon_view(view, &send));
        accept.set(true);
        assert!(!p.sync_icon_view(view, &send));
        assert!(!p.sync_icon_view(view, &send));
        // Swapping the panes' modes changes lookups but not the edges.
        let swapped = IconView {
            tiles: [true, false],
            ..view
        };
        assert!(p.sync_icon_view(swapped, &send));
        let dense = IconView {
            edges: IconEdges {
                entry: 25,
                ..view.edges
            },
            ..swapped
        };
        assert!(p.sync_icon_view(dense, &send));
        let sent: Vec<_> = sent
            .borrow()
            .iter()
            .map(|command| match command {
                Command::SetIconEdges(edges) => *edges,
                other => panic!("unexpected command: {other:?}"),
            })
            .collect();
        assert_eq!(sent, [view.edges, dense.edges]);
    }
}
