//! Exercise the bridge's retry trigger against the real retained-pixel budget.
use super::*;
use midnight_core::{
    Command, DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId, PanelViewMode,
    UserPreferences,
};
use std::sync::Arc;

#[test]
fn rejected_upgrade_waits_for_changed_view_then_retries_with_its_retained_coverage() {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(super::tests::TestPlatform(native))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            thumbnails_enabled: true,
            thumbnails_network: true,
            panel_view_modes: [PanelViewMode::Thumbnails; 2],
            thumbnail_size: 256,
            ..UserPreferences::default()
        }))));
    let navigation = Rc::new(Cell::new(None));
    let captured = navigation.clone();
    midnight_ui::install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            if let Command::Navigate {
                pane: PaneId::Left,
                navigation: id,
                ..
            } = command
            {
                captured.set(Some(id));
            }
            true
        }),
    );
    window.invoke_navigate(0, root.to_string_lossy().into_owned().into());
    let mut presenter = presenter.borrow_mut();
    let entries: Arc<[Entry]> = (0..10)
        .map(|index| {
            Entry::new(
                EntryId(index),
                root.join(format!("{index}.png")),
                EntryKind::File,
            )
        })
        .collect();
    presenter.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: navigation.get().unwrap(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let rows = presenter.thumbnail_targets([(0, 11), (0, 0)], ["", ""]);
    assert_eq!(rows.len(), 10);
    let large_pixels = vec![255; 512 * 510 * 4];
    for target in &rows[..9] {
        for frame in 0..6 {
            assert!(presenter.receive_thumbnail_at_resolution(
                &target.identity,
                frame,
                512,
                510,
                512,
                &large_pixels
            ));
        }
    }
    let identity = rows[9].identity.clone();
    assert!(presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        96,
        96,
        96,
        &vec![128; 96 * 96 * 4]
    ));
    let upgraded_pixels = vec![255; 512 * 512 * 4];
    assert!(!presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        512,
        512,
        512,
        &upgraded_pixels
    ));
    assert_eq!(presenter.thumbnail_coverage(&identity, 0), 96);

    let mut deliveries = Deliveries::default();
    deliveries.set_view(rows.clone(), 2.0, |_| panic!("no previous rejection"));
    let work = Work {
        generation: 1,
        token: 42,
        edge: 512,
        frame: 0,
    };
    deliveries.record(work, presenter.thumbnail_coverage(&identity, 0));
    for _ in 0..20 {
        deliveries.set_view(rows.clone(), 2.0, |_| {
            panic!("unchanged timer pulses must not retry")
        });
    }
    let less_crowded = presenter.thumbnail_targets([(2, 11), (0, 0)], ["", ""]);
    assert_eq!(less_crowded.len(), 9);
    assert!(
        less_crowded
            .iter()
            .any(|target| target.identity == identity)
    );
    let mut requests = Vec::new();
    deliveries.set_view(less_crowded.clone(), 2.0, |retry| {
        requests.push((retry, presenter.thumbnail_coverage(&identity, retry.frame)));
        true
    });
    assert_eq!(
        requests,
        [(work, 96)],
        "the bridge automatically retries the same immutable source token"
    );
    assert!(presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        512,
        512,
        512,
        &upgraded_pixels
    ));
    deliveries.record(work, presenter.thumbnail_coverage(&identity, 0));
    assert_eq!(presenter.thumbnail_coverage(&identity, 0), 512);
    deliveries.set_view(less_crowded, 1.0, |_| {
        panic!("an accepted upgrade must not be retried again")
    });
}

#[test]
fn zoom_releases_hidden_pane_pressure_and_delivers_rejected_visible_frame_without_input() {
    use slint::Model;

    let ui = super::tests::ThumbnailUi::new([9, 1], 256);
    ui.window.invoke_activate(1);
    ui.window.set_panel_zoomed(false);
    // Even a one-column hidden pane must retain all nine raw candidates after zoom.
    ui.resize(1200, 4000);
    ui.draw();
    assert_eq!(ui.window.get_active_pane(), 1);
    assert!(!ui.window.get_compact_window());
    let window_size = ui.window.window().size();
    let window_scale = ui.window.window().scale_factor();
    let scroll = [
        ui.window.get_left_list_content_x(),
        ui.window.get_left_list_content_y(),
        ui.window.get_right_list_content_x(),
        ui.window.get_right_list_content_y(),
    ];
    let hovered = [
        ui.window.get_left_hovered_row(),
        ui.window.get_right_hovered_row(),
    ];
    assert!(hovered.iter().all(|identity| identity.is_empty()));

    let rows = ui.targets();
    let pressure = ui.panel_targets(&rows, PaneId::Left);
    let visible = ui.panel_targets(&rows, PaneId::Right);
    assert_eq!(pressure.len(), 9);
    assert_eq!(visible.len(), 1);
    let command_count = rows
        .iter()
        .filter(|target| target.identity.starts_with("command-icon:"))
        .count();
    assert_eq!(command_count, 1);
    assert_eq!(rows.len(), 11);
    let identity = visible[0].identity.clone();
    let model = ui.window.get_right_rows();
    let row_index = (0..model.row_count())
        .find(|index| model.row_data(*index).unwrap().name == "image-000.png")
        .unwrap();
    let row_identity = model.row_data(row_index).unwrap().identity;
    let work = Work {
        generation: 1,
        token: 42,
        edge: 512,
        frame: 0,
    };
    let mut tokens = HashMap::from([(identity.clone(), work.token)]);
    assert_eq!(physical_edge(&visible[0], 2.0), work.edge);
    let large_pixels = vec![255; 512 * 510 * 4];
    let visible_pixels = vec![255; 512 * 512 * 4];
    let mut deliveries = Deliveries::default();
    deliveries.set_view(rows.clone(), 2.0, |_| panic!("no previous rejection"));
    {
        let mut presenter = ui.presenter.borrow_mut();
        // These 54 protected frames leave less than one 512-by-512 frame of space.
        for target in &pressure {
            for frame in 0..6 {
                assert!(presenter.receive_thumbnail_at_resolution(
                    &target.identity,
                    frame,
                    512,
                    510,
                    512,
                    &large_pixels
                ));
            }
        }
        assert_eq!(presenter.thumbnail_coverage(&identity, 0), 0);
        assert!(!presenter.receive_thumbnail_at_resolution(
            &identity,
            0,
            512,
            512,
            512,
            &visible_pixels
        ));
        assert_eq!(presenter.thumbnail_coverage(&identity, 0), 0);
        deliveries.record(work, 0);
    }
    for _ in 0..20 {
        deliveries.set_view(ui.targets(), 2.0, |_| {
            panic!("unchanged timer pulses must not retry a zero-coverage rejection")
        });
    }

    // This is the only presentation change after the refusal; no new input follows it.
    ui.window.set_panel_zoomed(true);
    ui.draw();
    assert_eq!(ui.window.get_active_pane(), 1);
    assert_eq!(ui.window.window().size(), window_size);
    assert_eq!(ui.window.window().scale_factor(), window_scale);
    assert_eq!(
        [
            ui.window.get_left_list_content_x(),
            ui.window.get_left_list_content_y(),
            ui.window.get_right_list_content_x(),
            ui.window.get_right_list_content_y(),
        ],
        scroll
    );
    let zoomed_hovered = [
        ui.window.get_left_hovered_row(),
        ui.window.get_right_hovered_row(),
    ];
    assert_eq!(zoomed_hovered, hovered);
    // Read every Slint property before borrowing Presenter. The raw sample proves
    // that a geometry/selection/hover change cannot accidentally release pressure.
    let raw_ranges = [
        (
            ui.window.get_left_thumbnail_row_start().max(0) as usize,
            ui.window.get_left_thumbnail_row_end().max(0) as usize,
        ),
        (
            ui.window.get_right_thumbnail_row_start().max(0) as usize,
            ui.window.get_right_thumbnail_row_end().max(0) as usize,
        ),
    ];
    let raw_rows = ui.presenter.borrow_mut().thumbnail_targets(
        raw_ranges,
        [zoomed_hovered[0].as_str(), zoomed_hovered[1].as_str()],
    );
    let raw_pressure_count = ui.panel_targets(&raw_rows, PaneId::Left).len();
    assert_eq!(
        raw_pressure_count, 9,
        "the old raw range must still protect every pressure identity after zoom"
    );
    assert!(
        raw_rows == rows,
        "all raw source identities, requested edges and priority flags must remain unchanged"
    );

    // Restore admission through the exact production window sampler after the raw probe.
    let zoomed_rows = ui.targets();
    let hidden_count = ui.panel_targets(&zoomed_rows, PaneId::Left).len();
    let surviving = ui.panel_targets(&zoomed_rows, PaneId::Right);
    assert!(
        surviving == visible,
        "the visible source must remain unchanged"
    );
    retain_source_tokens(&mut tokens, &rows, &zoomed_rows);
    assert_eq!(tokens.get(&identity), Some(&work.token));
    let mut requests = Vec::new();
    deliveries.set_view(zoomed_rows.clone(), 2.0, |retry| {
        requests.push((
            retry,
            ui.presenter
                .borrow()
                .thumbnail_coverage(&identity, retry.frame),
        ));
        true
    });
    let (accepted, coverage) = {
        let mut presenter = ui.presenter.borrow_mut();
        let accepted =
            presenter.receive_thumbnail_at_resolution(&identity, 0, 512, 512, 512, &visible_pixels);
        (accepted, presenter.thumbnail_coverage(&identity, 0))
    };
    deliveries.record(work, coverage);
    eprintln!(
        "hidden-pane pressure: before_left={} before_right={} command={} \
         raw_zoom_left={} sampled_zoom_left={} sampled_zoom_right={} retries={} coverage={}",
        pressure.len(),
        visible.len(),
        command_count,
        raw_pressure_count,
        hidden_count,
        surviving.len(),
        requests.len(),
        coverage
    );
    // Keep this before the hidden-count assertion so RED exercises real cache admission.
    assert!(
        accepted,
        "one zoom must release enough hidden pressure to retain the rejected visible frame"
    );
    assert_eq!(
        requests,
        [(work, 0)],
        "the changed view retries the same immutable generation/token exactly once"
    );
    assert_eq!(coverage, 512);
    midnight_ui::refresh_thumbnail_rows(
        &ui.window,
        &ui.presenter.borrow(),
        std::slice::from_ref(&identity),
    );
    ui.draw();
    let row = ui.window.get_right_rows().row_data(row_index).unwrap();
    assert!(row.identity == row_identity);
    assert_eq!((row.icon.size().width, row.icon.size().height), (512, 512));
    let delivered = row.icon.to_rgba8().unwrap();
    assert!(
        delivered.as_bytes() == visible_pixels.as_slice(),
        "the actual visible row must expose the delivered pixels"
    );
    assert_eq!(hidden_count, 0);
    assert_eq!(zoomed_rows.len(), 2);
    for _ in 0..20 {
        let unchanged = ui.targets();
        assert!(unchanged == zoomed_rows);
        deliveries.set_view(unchanged, 2.0, |_| {
            panic!("unchanged pulses must not retry the accepted visible frame")
        });
    }
    assert_eq!(ui.presenter.borrow().thumbnail_coverage(&identity, 0), 512);
}
