//! Diagnostic-only inputs, frame summaries and post-timing pixel evidence.
use midnight_core::{Entry, EntryId, EntryKind, SortSpec, sort_entries};
use midnight_ui::{FileRow, MainWindow};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, platform::WindowEvent};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) fn run_with_app_renderer() -> io::Result<std::process::ExitStatus> {
    // UI-only builds do not expose Slint's unstable WGPU configuration API.
    // Configure the same Automatic defaults as window_backend_windows.rs in a
    // fresh child, before WGPU initializes; preserve explicit diagnostic opts.
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(std::env::args_os().skip(1));
    for (name, default) in [
        ("WGPU_BACKEND", "dx12"),
        ("WGPU_DEBUG", "0"),
        ("WGPU_VALIDATION", "0"),
    ] {
        if std::env::var_os(name).is_none() {
            command.env(name, default);
        }
    }
    command
        .env("WGPU_VALIDATION_INDIRECT_CALL", "1")
        .env("MIDNIGHT_SCROLL_APP_RENDERER_CHILD", "1")
        .status()
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn GetThreadTimes(
        thread: *mut std::ffi::c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}

pub(super) fn thread_cpu_ms() -> f64 {
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: the current-thread pseudo handle needs no close; each output is
    // a live, correctly laid-out FILETIME for the duration of the native call.
    let valid = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if valid == 0 {
        return 0.0;
    }
    let ticks = |time: FileTime| ((u64::from(time.high) << 32) | u64::from(time.low)) as f64;
    (ticks(kernel) + ticks(user)) / 10_000.0
}

pub(super) fn load_entries(path: PathBuf, real: bool) -> io::Result<Arc<[Entry]>> {
    std::thread::spawn(move || {
        if !real {
            return Ok((0..10_000)
                .map(|id| {
                    Entry::new(
                        EntryId(id),
                        path.join(format!("entry-{id:06}-文档.txt")),
                        EntryKind::File,
                    )
                })
                .collect());
        }
        let mut entries = Vec::new();
        for (id, result) in std::fs::read_dir(&path)?.enumerate() {
            let file = result?;
            let metadata = file.metadata()?;
            let mut entry = Entry::new(
                EntryId(id as u64),
                file.path(),
                if metadata.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            );
            entry.metadata.size = Some(metadata.len());
            entry.metadata.modified = metadata.modified().ok();
            entries.push(entry);
        }
        sort_entries(&mut entries, SortSpec::default());
        Ok(entries.into())
    })
    .join()
    .map_err(|_| io::Error::other("directory fixture worker panicked"))?
}

fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return Value::Null;
    }
    json!({"samples": values.len(), "min": values[0],
        "p50": values[values.len() / 2],
        "p95": values[(values.len() * 95 / 100).min(values.len() - 1)],
        "max": values[values.len() - 1],
        "mean": values.iter().sum::<f64>() / values.len() as f64})
}

pub(super) fn report(injections: &[f64], frames: &[(f64, f32, bool, f64, f64)], row: f32) {
    for (index, &start) in injections.iter().enumerate() {
        let end = injections.get(index + 1).copied().unwrap_or(f64::INFINITY);
        let active: Vec<_> = frames
            .iter()
            .filter(|f| f.0 >= start && f.0 < end && f.2)
            .collect();
        let Some(first) = active.first() else {
            continue;
        };
        let last = active.last().unwrap();
        let initial = frames.iter().rfind(|f| f.0 < start).map_or(0.0, |f| f.1);
        let mut previous = initial;
        let deltas: Vec<_> = active
            .iter()
            .map(|f| {
                let delta = f.1 - previous;
                previous = f.1;
                f64::from(delta)
            })
            .collect();
        let intervals: Vec<_> = active
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .collect();
        let completion = frames.iter().find(|f| f.0 > last.0 && f.0 < end && !f.2);
        eprintln!(
            "frame_metrics {}",
            json!({
                "notch": index + 1, "active_frames": active.len(),
                "first_input_to_submission_ms": first.0 - start,
                "input_to_completed_motion_ms": completion.map(|f| f.0 - start),
                "active_updates_per_second": if last.0 > first.0 { 1000.0 * (active.len() - 1) as f64 / (last.0 - first.0) } else { 0.0 },
                "fractional_row_frames": active.iter().filter(|f| (f.1 / row - (f.1 / row).round()).abs() > 0.01).count(),
                "interval_ms": distribution(intervals.clone()),
                "redraw_submission_ms": distribution(active.iter().map(|f| f.3).collect()),
                "ui_thread_cpu_ms": distribution(active.iter().map(|f| f.4).collect()),
                "ui_thread_cpu_total_ms": active.iter().map(|f| f.4).sum::<f64>(),
                "raw_offset_deltas": deltas, "raw_interval_ms": intervals,
                "measurement": "Winit redraw dispatch/submission, not compositor presentation or scanout",
            })
        );
    }
}

pub(super) struct PixelProof {
    name: &'static str,
    width: u32,
    height: u32,
    pixels: Vec<slint::Rgba8Pixel>,
}

pub(super) fn capture_pixel_proof(window: &MainWindow) -> Vec<PixelProof> {
    let mut captures = Vec::new();
    // GPU readback changes timing: these snapshots run only after measurement.
    // Public pixel events exercise the actual renderer at precise fractional offsets.
    for name in ["offset-0", "offset-5_5", "offset-11"] {
        if !captures.is_empty() {
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        window.get_left_list_left() + 100.0,
                        window.get_left_list_top() + 100.0,
                    ),
                    delta_x: 0.0,
                    delta_y: -5.5,
                });
        }
        let snapshot = window.window().take_snapshot().unwrap();
        let scale = window.window().scale_factor();
        let x = (window.get_left_list_left() * scale).round() as u32;
        let y = (window.get_left_list_top() * scale).round() as u32;
        let width = (384.0 * scale).round() as u32;
        let height = (128.0 * scale).round() as u32;
        let width = width.min(snapshot.width().saturating_sub(x));
        let height = height.min(snapshot.height().saturating_sub(y));
        let pixels: Vec<_> = (y..y + height)
            .flat_map(|row| {
                let start = (row * snapshot.width() + x) as usize;
                snapshot.as_slice()[start..start + width as usize]
                    .iter()
                    .copied()
            })
            .collect();
        let column = (8.0 * scale).round() as usize;
        let boundaries: Vec<_> = (1..height as usize)
            .filter(|&row| {
                pixels[row * width as usize + column] != pixels[(row - 1) * width as usize + column]
            })
            .take(16)
            .collect();
        eprintln!(
            "pixel_proof name={name} content_y={} scale={scale} row_boundary_physical_y={boundaries:?}",
            window.get_left_list_content_y()
        );
        captures.push(PixelProof {
            name,
            width,
            height,
            pixels,
        });
    }
    captures
}

pub(super) fn write_pixel_proof(directory: &Path, captures: Vec<PixelProof>) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    for proof in captures {
        let stride = (proof.width * 3 + 3) & !3;
        let length = 54 + stride * proof.height;
        let mut bytes = Vec::with_capacity(length as usize);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&proof.width.to_le_bytes());
        bytes.extend_from_slice(&(-(proof.height as i32)).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        for row in proof.pixels.chunks(proof.width as usize) {
            for pixel in row {
                bytes.extend_from_slice(&[pixel.b, pixel.g, pixel.r]);
            }
            bytes.resize(bytes.len() + (stride - proof.width * 3) as usize, 0);
        }
        std::fs::write(directory.join(format!("{}.bmp", proof.name)), bytes)?;
    }
    Ok(())
}

pub(super) fn check_row_invalidation(window: &MainWindow) -> bool {
    fn icon(size: u32, color: [u8; 3]) -> slint::Image {
        let pixels = color.repeat((size * size) as usize);
        slint::Image::from_rgb8(
            slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(&pixels, size, size),
        )
    }
    fn row_pixels(window: &MainWindow, left: f32, right: f32) -> Vec<slint::Rgba8Pixel> {
        let snapshot = window.window().take_snapshot().unwrap();
        let scale = window.window().scale_factor();
        let x0 = ((window.get_left_list_left() + left) * scale).round() as usize;
        let x1 = ((window.get_left_list_left() + right) * scale).round() as usize;
        let y0 = ((window.get_left_list_top() + window.get_left_view_row_height()) * scale).round()
            as usize;
        let y1 = ((window.get_left_list_top() + 2.0 * window.get_left_view_row_height()) * scale)
            .round() as usize;
        (y0..y1)
            .flat_map(|y| {
                snapshot.as_slice()
                    [y * snapshot.width() as usize + x0..y * snapshot.width() as usize + x1]
                    .iter()
                    .copied()
            })
            .collect()
    }
    fn count_color(pixels: &[slint::Rgba8Pixel], color: [u8; 3]) -> usize {
        pixels.iter().filter(|p| [p.r, p.g, p.b] == color).count()
    }

    // Post-timing fixture: the same actual FileEntry receives row_changed
    // notifications. Neither its model nor its mode changes between updates.
    window.invoke_view_mode(0, 0);
    window.set_left_cursor(0);
    window.invoke_reveal_left();
    let red = [240, 30, 30];
    let blue = [30, 70, 240];
    let mut row = FileRow {
        identity: "native-cache-invalidation".into(),
        name: "Before cached repaint".into(),
        icon: icon(8, red),
        ..Default::default()
    };
    let rows = std::rc::Rc::new(slint::VecModel::from(vec![FileRow::default(), row.clone()]));
    window.set_left_rows(slint::ModelRc::from(rows.clone()));
    let before_name = row_pixels(window, 60.0, 380.0);
    let before_icon = row_pixels(window, 0.0, 60.0);
    let initial_red_pixels = count_color(&before_icon, red);

    row.name = "After row-data notification".into();
    rows.set_row_data(1, row.clone());
    let after_name = row_pixels(window, 60.0, 380.0);
    let changed = before_name
        .iter()
        .zip(&after_name)
        .filter(|(a, b)| a != b)
        .count();

    row.icon = icon(13, blue);
    rows.set_row_data(1, row);
    let after_icon = row_pixels(window, 0.0, 60.0);
    let blue_pixels = count_color(&after_icon, blue);
    let old_red_pixels = count_color(&after_icon, red);
    let name_preserved = after_name == row_pixels(window, 60.0, 380.0);
    eprintln!(
        "native row invalidation changed_name_pixels={changed} initial_red_pixels={initial_red_pixels} blue_icon_pixels={blue_pixels} old_red_pixels={old_red_pixels} name_preserved={name_preserved}"
    );
    initial_red_pixels > 20
        && changed > 20
        && blue_pixels > 20
        && old_red_pixels == 0
        && name_preserved
}
