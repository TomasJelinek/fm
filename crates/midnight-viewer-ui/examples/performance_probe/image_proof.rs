//! Image opening snapshots are a separate diagnostic, excluded from latency comparisons.
use midnight_viewer_ui::ViewerWindow;
use slint::ComponentHandle;
use std::{io, path::Path};

pub(super) struct Capture {
    name: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

pub(super) fn capture(window: &ViewerWindow, name: String) -> Result<(Capture, String), String> {
    let snapshot = window
        .window()
        .take_snapshot()
        .map_err(|error| error.to_string())?;
    let scale = window.window().scale_factor();
    let left = (window.get_content_left() * scale).round() as u32;
    let top = (window.get_content_top() * scale).round() as u32;
    let width = (window.get_content_width() * scale).floor() as u32;
    let height = (window.get_content_height() * scale).floor() as u32;
    if width == 0
        || height == 0
        || left + width > snapshot.width()
        || top + height > snapshot.height()
    {
        return Err("image snapshot has invalid canvas bounds".into());
    }
    let background = window.get_background_color();
    let background = [background.red(), background.green(), background.blue()];
    let mut low = [width, height];
    let mut high = [0, 0];
    let mut count = 0usize;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let pixel = snapshot.as_slice()[((top + y) * snapshot.width() + left + x) as usize];
            pixels.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
            if [pixel.r, pixel.g, pixel.b]
                .into_iter()
                .zip(background)
                .any(|(value, expected)| value.abs_diff(expected) > 2)
            {
                low = [low[0].min(x), low[1].min(y)];
                high = [high[0].max(x), high[1].max(y)];
                count += 1;
            }
        }
    }
    if count == 0 {
        return Err("image snapshot contains only canvas background".into());
    }
    let bounds = [low[0], low[1], high[0] - low[0] + 1, high[1] - low[1] + 1];
    let row = format!(
        "image_pixels name={name} scale={scale} canvas_physical={:?} nonbackground_bounds_physical={bounds:?} nonbackground_pixels={count}",
        [width, height],
    );
    Ok((
        Capture {
            name,
            width,
            height,
            pixels,
        },
        row,
    ))
}

pub(super) fn write(directory: &Path, captures: Vec<Capture>) -> io::Result<()> {
    if captures.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(directory)?;
    for capture in captures {
        let file = std::fs::File::create(directory.join(format!("{}.png", capture.name)))?;
        let mut encoder = png::Encoder::new(file, capture.width, capture.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer
            .write_image_data(&capture.pixels)
            .map_err(io::Error::other)?;
    }
    Ok(())
}
