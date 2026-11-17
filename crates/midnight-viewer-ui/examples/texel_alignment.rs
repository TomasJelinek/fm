//! Native check that viewer rasters stay texel-exact when the canvas origin and
//! size fall between physical pixels. A real viewer window shows one-pixel
//! checkerboards, first as a viewport-only frame and then as a retained body
//! beside a pinned gutter, and every inner canvas pixel of a renderer snapshot is
//! verified: a resampled texel blends colors, a stretched one repeats. Run with a
//! fractional scale, for example in PowerShell:
//!
//! ```text
//! $env:SLINT_SCALE_FACTOR = '1.25'
//! cargo run -p midnight-viewer-ui --example texel_alignment -- --femtovg
//! ```
//!
//! `--femtovg` selects FemtoVG on wgpu with the application's default backend
//! (DX12 on Windows; `WGPU_BACKEND` overrides it), `--software` the software
//! renderer. The window is shown without activation and closes by itself. A
//! final control repeats the former mapping, which stretched the frame over the
//! fractional canvas, and only reports whether the renderer shows it.
use midnight_viewer_ui::ViewerWindow;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use std::{cell::RefCell, rc::Rc, time::Duration};

const BODY: [[u8; 3]; 2] = [[200, 40, 40], [40, 200, 220]];
const GUTTER: [[u8; 3]; 2] = [[250, 250, 60], [60, 60, 250]];
const GUTTER_WIDTH: u32 = 35;
const GUARD: u32 = 64;

fn checker(width: u32, height: u32, colors: [[u8; 3]; 2]) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let width = width as usize;
    for (index, pixel) in buffer.make_mut_slice().iter_mut().enumerate() {
        let [r, g, b] = colors[(index % width + index / width) % 2];
        *pixel = Rgba8Pixel { r, g, b, a: 255 };
    }
    Image::from_rgba8_premultiplied(buffer)
}

/// Verify a physical `[x0, y0, x1, y1)` snapshot region; returns its pixel count.
fn verify(
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
    region: [usize; 4],
    colors: [[u8; 3]; 2],
    what: &str,
) -> Result<usize, String> {
    let stride = snapshot.width() as usize;
    if region[0] + 16 >= region[2]
        || region[1] + 16 >= region[3]
        || region[2] > stride
        || region[3] > snapshot.height() as usize
    {
        return Err(format!("{what}: invalid region {region:?}"));
    }
    let rgb = |x: usize, y: usize| {
        let pixel = snapshot.as_slice()[y * stride + x];
        [pixel.r, pixel.g, pixel.b]
    };
    for y in region[1]..region[3] {
        for x in region[0]..region[2] {
            let color = rgb(x, y);
            if !colors.contains(&color) {
                return Err(format!("{what}: resampled texel {color:?} at {x},{y}"));
            }
            if x + 1 < region[2] && rgb(x + 1, y) == color {
                return Err(format!("{what}: repeated column at {x},{y}"));
            }
            if y + 1 < region[3] && rgb(x, y + 1) == color {
                return Err(format!("{what}: repeated row at {x},{y}"));
            }
        }
    }
    Ok((region[2] - region[0]) * (region[3] - region[1]))
}

fn check(window: &ViewerWindow) -> Result<String, String> {
    let scale = window.window().scale_factor();
    let [left, top, width, height] = [
        window.get_content_left(),
        window.get_content_top(),
        window.get_content_width(),
        window.get_content_height(),
    ]
    .map(|extent| extent * scale);
    // The backend renders whole physical pixels; the UI maps them 1:1.
    let native = [width.round() as u32, height.round() as u32];
    let inner = [
        left.ceil() as usize + 1,
        top.ceil() as usize + 1,
        (left + width).floor() as usize - 1,
        (top + height).floor() as usize - 1,
    ];
    let snapshot = || {
        window
            .window()
            .take_snapshot()
            .map_err(|error| error.to_string())
    };

    window.set_retained_raster(false);
    window.set_surface(checker(native[0], native[1], BODY));
    window.set_raster_viewport_width(native[0] as f32 / scale);
    window.set_raster_viewport_height(native[1] as f32 / scale);
    let mut checked = verify(&snapshot()?, inner, BODY, "viewport-only frame")?;

    let body = native.map(|extent| extent + 2 * GUARD);
    window.set_surface(checker(body[0], body[1], BODY));
    window.set_raster_gutter(checker(GUTTER_WIDTH, body[1], GUTTER));
    window.set_raster_width(body[0] as f32 / native[0] as f32);
    window.set_raster_height(body[1] as f32 / native[1] as f32);
    window.set_raster_offset_x(-(GUARD as f32) / native[0] as f32);
    window.set_raster_offset_y(-(GUARD as f32) / native[1] as f32);
    window.set_raster_gutter_width(GUTTER_WIDTH as f32 / native[0] as f32);
    window.set_retained_raster(true);
    let retained = snapshot()?;
    let split = left + GUTTER_WIDTH as f32;
    checked += verify(
        &retained,
        [inner[0], inner[1], split.floor() as usize - 1, inner[3]],
        GUTTER,
        "retained gutter",
    )?;
    checked += verify(
        &retained,
        [split.ceil() as usize + 1, inner[1], inner[2], inner[3]],
        BODY,
        "retained body",
    )?;

    // Control: the former mapping stretched the frame over the fractional
    // canvas. Report whether this renderer shows that as resampled pixels.
    window.set_retained_raster(false);
    window.set_surface(checker(native[0], native[1], BODY));
    window.set_raster_viewport_width(window.get_content_width());
    window.set_raster_viewport_height(window.get_content_height());
    let stretched = verify(&snapshot()?, inner, BODY, "stretched control").err();
    Ok(format!(
        "scale={scale} canvas_origin=({left:.2},{top:.2}) canvas_size=({width:.2},{height:.2}) \
         native={native:?} checked_pixels={checked} stretched_control={}",
        stretched.as_deref().unwrap_or("not detected")
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let flags: Vec<String> = std::env::args().skip(1).collect();
    let femtovg = flags.iter().any(|flag| flag == "--femtovg");
    let software = flags.iter().any(|flag| flag == "--software");
    if femtovg == software || flags.len() != 1 {
        return Err("usage: texel_alignment --femtovg|--software".into());
    }
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false));
    let selector = if femtovg {
        let mut graphics = slint::wgpu_30::WGPUSettings::default();
        graphics.backends =
            slint::wgpu_30::wgpu::Backends::from_env().unwrap_or(if cfg!(windows) {
                slint::wgpu_30::wgpu::Backends::DX12
            } else {
                slint::wgpu_30::wgpu::Backends::PRIMARY
            });
        selector
            .renderer_name("femtovg-wgpu".into())
            .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(graphics))
    } else {
        selector.renderer_name("software".into())
    };
    selector.select()?;
    let window = ViewerWindow::new()?;
    window.show()?;
    let result = Rc::new(RefCell::new(None));
    let sink = result.clone();
    let weak = window.as_weak();
    // Let the first native layout and DPI settle before reading the canvas.
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        if let Some(window) = weak.upgrade() {
            *sink.borrow_mut() = Some(check(&window));
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()?;
    window.hide()?;
    let outcome = result.borrow_mut().take();
    match outcome {
        Some(Ok(report)) => {
            println!("texel_alignment renderer={} ok {report}", flags[0]);
            Ok(())
        }
        Some(Err(error)) => Err(error.into()),
        None => Err("the window closed before the check ran".into()),
    }
}
