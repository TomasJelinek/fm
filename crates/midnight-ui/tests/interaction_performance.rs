//! Reproducible debug-only interaction timings; no machine-specific pass thresholds.
use crate::ui;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, Model, platform::WindowEvent};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn summary(name: &str, samples: &mut [f64]) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}: median_ms={:.3} p95_ms={:.3} n={}",
        samples[samples.len() / 2],
        samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
        samples.len()
    );
}
#[test]
#[ignore = "explicit debug software-renderer benchmark"]
fn interaction_performance() {
    let mut ui = ui::TestUi::new();
    let location = std::env::temp_dir().join("midnight-interaction-benchmark");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        location.clone(),
        location.clone(),
    )));
    let commands = Rc::new(RefCell::new(Vec::<Command>::new()));
    let capture = commands.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |cmd| {
            capture.borrow_mut().push(cmd);
            true
        }),
    );
    for pane in [PaneId::Left, PaneId::Right] {
        ui.window.invoke_action(pane.index() as i32, 3);
        let navigation = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|cmd| match cmd {
                Command::Navigate {
                    pane: p,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: p,
                    navigation,
                    ..
                } if *p == pane => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = (0..10_000)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    location.join(format!("entry-{id:06}-文档.txt")),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let mut callback = Vec::new();
    let mut frame = Vec::new();
    let mut visible = Vec::new();
    let mut immediate = 0;
    for row in 1..=24 {
        let started = Instant::now();
        ui.window.invoke_row_click(0, row, true, false);
        callback.push(started.elapsed().as_secs_f64() * 1000.0);
        ui.draw();
        frame.push(started.elapsed().as_secs_f64() * 1000.0);
        let shown = ui
            .window
            .get_left_rows()
            .row_data(row as usize)
            .unwrap()
            .selected;
        if shown {
            immediate += 1;
            visible.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        std::thread::sleep(Duration::from_millis(25));
        let req = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|cmd| {
                if let Command::Selection(req) = cmd {
                    Some(req.clone())
                } else {
                    None
                }
            })
            .unwrap();
        let basis = req.view.clone().unwrap();
        let mut bits = vec![0; basis.by_id.len().div_ceil(64)];
        bits[0] = (1u64 << row) - 1;
        presenter.borrow_mut().apply(Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: req.navigation,
            revision: req.revision,
            selection: Arc::new(SelectionSnapshot::from_bits(basis, bits).unwrap()),
            deferred: false,
        });
        refresh(&ui.window, &presenter.borrow());
        ui.draw();
        if !shown {
            visible.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let mut resize = Vec::new();
    let mut geometry = Vec::new();
    let mut raster = Vec::new();
    for round in 0..24 {
        let started = Instant::now();
        ui.width = if round % 2 == 0 { 1240 } else { 1340 };
        ui.native
            .set_size(slint::PhysicalSize::new(ui.width as u32, 820));
        ui.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(ui.width as f32, 820.0),
        });
        ui.pixels
            .resize(ui.width * 820, slint::Rgb8Pixel::default());
        std::hint::black_box((
            ui.window.get_left_list_width(),
            ui.window.get_right_list_width(),
            ui.window.get_left_list_top(),
            ui.window.get_right_list_top(),
        ));
        geometry.push(started.elapsed().as_secs_f64() * 1000.0);
        let draw_started = Instant::now();
        ui.draw();
        raster.push(draw_started.elapsed().as_secs_f64() * 1000.0);
        resize.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "workload: debug software renderer, 10000 rows/pane, 1340x820 / 1240x820, controlled acknowledgement delay 25ms; immediate_selection={immediate}/24"
    );
    summary("click_callback", &mut callback);
    summary("click_first_frame", &mut frame);
    summary("click_visible_selection", &mut visible);
    summary("resize_layout_render", &mut resize);
    summary("resize_dispatch_forced_geometry", &mut geometry);
    summary("resize_software_raster", &mut raster);
}
