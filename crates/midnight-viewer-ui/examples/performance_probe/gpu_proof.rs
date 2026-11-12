//! GPU readback proof, deliberately excluded from published timing comparisons.
use midnight_viewer_ui::ViewerWindow;
use slint::ComponentHandle;
use std::{io, path::Path};

pub(super) struct Capture {
    name: String,
    width: usize,
    height: usize,
    pixels: Vec<slint::Rgba8Pixel>,
}

pub(super) struct Proof {
    pub rows: Vec<String>,
    pub captures: Vec<Capture>,
    pub error: Option<String>,
}

fn snapshot(window: &ViewerWindow, name: String) -> Result<Capture, String> {
    let image = window
        .window()
        .take_snapshot()
        .map_err(|error| error.to_string())?;
    let scale = window.window().scale_factor();
    let x = (window.get_content_left() * scale).round() as usize;
    let y = ((window.get_content_top() + 24.0) * scale).round() as usize;
    let width = (window.get_content_width().min(512.0) * scale).round() as usize;
    let height = ((window.get_content_height() - 48.0).min(256.0) * scale).round() as usize;
    if x + width > image.width() as usize
        || y + height > image.height() as usize
        || width < 64
        || height < 64
    {
        return Err("invalid GPU snapshot crop".into());
    }
    let pixels = (y..y + height)
        .flat_map(|row| {
            let start = row * image.width() as usize + x;
            image.as_slice()[start..start + width].iter().copied()
        })
        .collect();
    Ok(Capture {
        name,
        width,
        height,
        pixels,
    })
}

// Compare actual rendered RGB pixels. Sample lines across the orthogonal axis,
// retaining all pixels along the movement axis, then fit the best subpixel shift.
fn displacement(
    before: &Capture,
    after: &Capture,
    x0: usize,
    x1: usize,
    horizontal: bool,
    maximum: usize,
) -> Result<(f64, f64), String> {
    let margin = maximum + 2;
    let length = if horizontal { x1 - x0 } else { before.height };
    let cross = if horizontal { before.height } else { x1 - x0 };
    if length <= 2 * margin || cross < 4 {
        return Err("GPU proof region too small".into());
    }
    let error = |shift: isize| {
        let mut total = 0u64;
        for coordinate in margin..length - margin {
            for sample in 0..48 {
                let other = 1 + sample * (cross - 2) / 48;
                let (x, y) = if horizontal {
                    (x0 + coordinate, other)
                } else {
                    (x0 + other, coordinate)
                };
                let (old_x, old_y) = if horizontal {
                    ((x as isize + shift) as usize, y)
                } else {
                    (x, (y as isize + shift) as usize)
                };
                let a = before.pixels[old_y * before.width + old_x];
                let b = after.pixels[y * after.width + x];
                for (a, b) in [(a.r, b.r), (a.g, b.g), (a.b, b.b)] {
                    let difference = i32::from(a) - i32::from(b);
                    total += (difference * difference) as u64;
                }
            }
        }
        total as f64
    };
    let errors: Vec<_> = (-(maximum as isize)..=maximum as isize)
        .map(error)
        .collect();
    let (index, best) = errors
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    let mut shift = index as f64 - maximum as f64;
    if index > 0 && index + 1 < errors.len() {
        let curvature = errors[index - 1] - 2.0 * best + errors[index + 1];
        if curvature > 0.0 {
            shift += ((errors[index - 1] - errors[index + 1]) / (2.0 * curvature)).clamp(-0.5, 0.5);
        }
    }
    Ok((
        shift,
        if errors[maximum] == 0.0 {
            1.0
        } else {
            best / errors[maximum]
        },
    ))
}

pub(super) fn capture(window: &ViewerWindow, stage: usize) -> Result<Proof, String> {
    if !window.get_retained_raster() && !window.get_tiled() {
        return Err("mode has neither a retained raster nor tiled image".into());
    }
    let raster = [window.get_raster_offset_x(), window.get_raster_offset_y()];
    let tile = [window.get_tile_offset_x(), window.get_tile_offset_y()];
    let uploads = [
        window.get_raster_upload_count(),
        window.get_tile_upload_count(),
    ];
    let scale = window.window().scale_factor();
    let expected = f64::from(5.5 * scale);
    let maximum = expected.ceil() as usize + 4;
    let mut captures = Vec::new();
    let mut rows = Vec::new();
    // Direct property translations isolate the Slint arrangement/rendering proof.
    // Actual native input and frontend camera movement are measured separately.
    let result = (|| {
        for (index, logical_y) in [0.0, 5.5, 11.0].into_iter().enumerate() {
            if window.get_retained_raster() {
                window.set_raster_offset_y(raster[1] - logical_y / window.get_content_height());
            } else {
                window.set_tile_offset_y(tile[1] + logical_y / window.get_content_height());
            }
            captures.push(snapshot(window, format!("stage-{stage}-y-{index}"))?);
        }
        let gutter = if window.get_retained_raster() {
            (window.get_raster_gutter_width() * window.get_content_width() * scale).round() as usize
        } else {
            0
        };
        let mut regions = vec![("body", gutter.min(captures[0].width), captures[0].width)];
        if gutter > 4 {
            regions.push(("gutter", 0, gutter.min(captures[0].width)));
        }
        for (index, pair) in captures.windows(2).enumerate() {
            for &(name, x0, x1) in &regions {
                let (shift, ratio) = displacement(&pair[0], &pair[1], x0, x1, false, maximum)?;
                let valid = (shift - expected).abs() < 1.25 && ratio < 0.75;
                rows.push(format!("gpu_pixels stage={stage} axis=y region={name} pair={index} expected_physical={expected:.3} measured_physical={shift:.3} error_vs_unmoved={ratio:.5} passed={valid}"));
                if !valid {
                    return Err(format!(
                        "stage={stage} GPU {name} vertical displacement {shift:.3}, expected {expected:.3}, error ratio {ratio:.5}"
                    ));
                }
            }
        }
        if gutter > 4 {
            window.set_raster_offset_y(raster[1]);
            window.set_raster_offset_x(raster[0] - 5.5 / window.get_content_width());
            let horizontal = snapshot(window, format!("stage-{stage}-x-5_5"))?;
            let baseline = &captures[0];
            let unchanged = (0..baseline.height).all(|y| {
                (0..gutter.saturating_sub(2)).all(|x| {
                    baseline.pixels[y * baseline.width + x]
                        == horizontal.pixels[y * baseline.width + x]
                })
            });
            let (shift, ratio) =
                displacement(baseline, &horizontal, gutter, baseline.width, true, maximum)?;
            rows.push(format!("gpu_pixels stage={stage} axis=x gutter_unchanged={unchanged} expected_body_physical={expected:.3} measured_body_physical={shift:.3} error_vs_unmoved={ratio:.5}"));
            if !unchanged || (shift - expected).abs() >= 1.25 || ratio >= 0.75 {
                return Err(format!(
                    "stage={stage} source gutter/body horizontal displacement failed"
                ));
            }
            captures.push(horizontal);
        }
        Ok(())
    })();
    window.set_raster_offset_x(raster[0]);
    window.set_raster_offset_y(raster[1]);
    window.set_tile_offset_x(tile[0]);
    window.set_tile_offset_y(tile[1]);
    let mut error = result.err();
    let after = [
        window.get_raster_upload_count(),
        window.get_tile_upload_count(),
    ];
    rows.push(format!("gpu_proof stage={stage} scale={scale} retained={} uploads_before={uploads:?} uploads_after={after:?} renderer_translation_only=true", window.get_retained_raster()));
    if after != uploads {
        error.get_or_insert_with(|| {
            format!("stage={stage} GPU proof unexpectedly uploaded new pixels")
        });
    }
    if let Some(error) = &error {
        rows.push(format!("gpu_proof_failure stage={stage}: {error}"));
    }
    Ok(Proof {
        rows,
        captures,
        error,
    })
}

pub(super) fn write(directory: &Path, captures: Vec<Capture>) -> io::Result<()> {
    if captures.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(directory)?;
    for capture in captures {
        let stride = (capture.width * 3 + 3) & !3;
        let length = 54 + stride * capture.height;
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(length as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(capture.width as u32).to_le_bytes());
        bytes.extend_from_slice(&(-(capture.height as i32)).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        for row in capture.pixels.chunks(capture.width) {
            for pixel in row {
                bytes.extend_from_slice(&[pixel.b, pixel.g, pixel.r]);
            }
            bytes.resize(bytes.len() + stride - capture.width * 3, 0);
        }
        std::fs::write(directory.join(format!("{}.bmp", capture.name)), bytes)?;
    }
    Ok(())
}
