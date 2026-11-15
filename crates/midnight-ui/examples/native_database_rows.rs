//! Explicit desktop probe for filename elision/vertical alignment; uses only synthetic rows.
#[path = "../tests/support/bitmap.rs"]
mod bitmap;
#[path = "../tests/support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{FileRow, MainWindow};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc, time::Duration};

const NAMES: [&str; 2] = [
    "mb-h56-static-dups.ovclash.ovstage.clashdata.ovclashdb",
    "mb-h56-static-dups.ovclash.ovstage.clashdata.ovclashdb-journal",
];

fn inspect(
    window: &MainWindow,
    rows: &VecModel<FileRow>,
    case: &str,
    step: u32,
) -> Result<(), String> {
    let snapshot = window.window().take_snapshot().map_err(|e| e.to_string())?;
    let scale = window.window().scale_factor();
    let mut bands = Vec::new();
    for index in 1..=2 {
        let name = rows.row_data(index).unwrap().name;
        let row = ui::root_item(window, name.as_str(), AccessibleRole::ListItem);
        let rect = row.geometry();
        let origin = row.map_to_native_window(rect.origin);
        // Fixed gutter at this density: sample only the identical filename prefix,
        // away from icons, metadata and the elided suffix.
        let x0 = ((origin.x + 52.0) * scale).round() as usize;
        let y0 = (origin.y * scale).round() as usize;
        let height = (rect.height() * scale).round() as usize;
        let end = (x0 + (80.0 * scale) as usize).min(snapshot.width() as usize);
        let ink = (0..height)
            .filter(|dy| {
                let y = y0 + dy;
                y < snapshot.height() as usize
                    && (x0..end).any(|x| {
                        let p = snapshot.as_slice()[y * snapshot.width() as usize + x];
                        p.r.max(p.g).max(p.b) > 150
                    })
            })
            .collect::<Vec<_>>();
        let band = ink
            .first()
            .zip(ink.last())
            .map(|(a, b)| (*a, *b))
            .ok_or_else(|| format!("no filename ink at step {step}: {name}"))?;
        eprintln!(
            "step={step} name={name:?} row_y={} row_h={} ink={band:?}",
            origin.y,
            rect.height()
        );
        bands.push(band);
    }
    let pixels = snapshot
        .as_slice()
        .iter()
        .map(|p| slint::Rgb8Pixel {
            r: p.r,
            g: p.g,
            b: p.b,
        })
        .collect::<Vec<_>>();
    bitmap::snapshot(
        &pixels,
        snapshot.width() as usize,
        snapshot.height() as usize,
        &format!("native-database-rows-{case}-{step}"),
    );
    if bands[0].0.abs_diff(bands[1].0) > 1 || bands[0].1.abs_diff(bands[1].1) > 1 {
        return Err(format!(
            "filename ink is misaligned inside equal rows: {bands:?}"
        ));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let case = std::env::args()
        .find_map(|arg| arg.strip_prefix("--case=").map(str::to_owned))
        .unwrap_or_else(|| "ascii".into());
    let suffix = match case.as_str() {
        "ascii" => "",
        "lf" => "\n",
        "cr" => "\r",
        _ => return Err("Use --case=ascii|lf|cr".into()),
    };
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .select()?;
    let window = MainWindow::new()?;
    window.invoke_apply_fonts("".into(), 13, 15);
    window.set_active_pane(1);
    window.set_right_size_column_sample("3,603,169,280".into());
    let rows = Rc::new(VecModel::from(vec![
        FileRow {
            identity: "parent".into(),
            name: "..".into(),
            parent_entry: true,
            directory: true,
            cursor: true,
            ..Default::default()
        },
        FileRow {
            identity: "database".into(),
            name: NAMES[0].into(),
            size: "3,603,169,280".into(),
            modified_date: "11/13/2026".into(),
            modified_time: "01:16:31 PM".into(),
            ..Default::default()
        },
        FileRow {
            identity: "journal".into(),
            name: format!("{}{suffix}", NAMES[1]).into(),
            size: "591,360".into(),
            modified_date: "11/13/2026".into(),
            modified_time: "01:16:02 PM".into(),
            ..Default::default()
        },
    ]));
    window.set_right_rows(ModelRc::from(rows.clone()));
    window
        .window()
        .set_size(slint::LogicalSize::new(1711.0, 900.0));
    let outcome = Rc::new(RefCell::new(Ok(())));
    let result = outcome.clone();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(500),
        move || {
            let Some(window) = weak.upgrade() else { return };
            if let Err(e) = inspect(&window, &rows, &case, step) {
                *result.borrow_mut() = Err(e);
                slint::quit_event_loop().unwrap();
                return;
            }
            match step {
                0 => {
                    let mut row = rows.row_data(2).unwrap();
                    row.size = "12,345,678".into();
                    rows.set_row_data(2, row);
                }
                1 => window
                    .window()
                    .set_size(slint::LogicalSize::new(1550.0, 900.0)),
                2 => window
                    .window()
                    .set_size(slint::LogicalSize::new(2100.0, 900.0)),
                _ => {
                    slint::quit_event_loop().unwrap();
                    return;
                }
            }
            step += 1;
        },
    );
    window.run()?;
    let result = outcome.borrow_mut().clone();
    result.map_err(Into::into)
}
