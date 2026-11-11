//! Opening snapshots are isolated from timing runs and never scroll the document.
use midnight_viewer_ui::ViewerWindow;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Probe {
    first: Option<Instant>,
    signature: Option<Signature>,
    drawing: Option<Option<DrawState>>,
    captures: Vec<Capture>,
    rows: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Signature {
    scale: f32,
    window_size: [u32; 2],
    native_window: Option<([u32; 2], f64)>,
    chrome: Chrome,
    viewport: [f32; 2],
    canvas: [f32; 4],
    raster: [f32; 4],
    surface: [u32; 2],
    gutter: f32,
    gutter_surface: [u32; 2],
    background: [u8; 4],
    line_numbers: bool,
    uploads: i32,
    retained: bool,
    source: bool,
    markdown: bool,
    font: String,
    font_size: i32,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Chrome {
    image: bool,
    web: bool,
    tiled: bool,
    compact: bool,
    search_visible: bool,
    font_available: bool,
    typography_visible: bool,
    narrow_typography: bool,
    wrap_enabled: bool,
    wrap_available: bool,
    mode_index: i32,
    markdown_preset: i32,
    ui_font: String,
    ui_font_size: f32,
}
struct DrawState {
    revision: u64,
    signature: Signature,
}
struct Capture {
    name: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl DrawState {
    fn matches_after_layout(&self, revision: u64, rendered: &Signature) -> bool {
        // Slint resolves pending toolbar layout inside the observed draw. Only
        // the resulting canvas may differ; frame and layout inputs remain exact.
        let mut resolved = self.signature.clone();
        resolved.canvas = rendered.canvas;
        self.revision == revision && resolved == *rendered
    }
}
impl Signature {
    fn capture(window: &ViewerWindow) -> Self {
        let surface = window.get_surface().size();
        let gutter = window.get_raster_gutter().size();
        let background = window.get_background_color();
        let size = window.window().size();
        Self {
            scale: window.window().scale_factor(),
            window_size: [size.width, size.height],
            native_window: window.window().with_winit_window(|native| {
                let size = native.inner_size();
                ([size.width, size.height], native.scale_factor())
            }),
            chrome: Chrome {
                image: window.get_image_mode(),
                web: window.get_web_mode(),
                tiled: window.get_tiled(),
                compact: window.get_compact(),
                search_visible: window.get_search_visible(),
                font_available: window.get_font_available(),
                typography_visible: window.get_typography_visible(),
                narrow_typography: window.get_narrow_typography(),
                wrap_enabled: window.get_wrap_enabled(),
                wrap_available: window.get_wrap_available(),
                mode_index: window.get_mode_index(),
                markdown_preset: window.get_markdown_preset(),
                ui_font: window.get_ui_font_family().to_string(),
                ui_font_size: window.get_ui_font_size(),
            },
            viewport: [
                window.get_raster_viewport_width(),
                window.get_raster_viewport_height(),
            ],
            canvas: [
                window.get_content_left(),
                window.get_content_top(),
                window.get_content_width(),
                window.get_content_height(),
            ],
            raster: [
                window.get_raster_width(),
                window.get_raster_height(),
                window.get_raster_offset_x(),
                window.get_raster_offset_y(),
            ],
            surface: [surface.width, surface.height],
            gutter: window.get_raster_gutter_width(),
            gutter_surface: [gutter.width, gutter.height],
            background: [
                background.red(),
                background.green(),
                background.blue(),
                background.alpha(),
            ],
            line_numbers: window.get_line_numbers(),
            uploads: window.get_raster_upload_count(),
            retained: window.get_retained_raster(),
            source: window.get_source_mode(),
            markdown: window.get_markdown_mode(),
            font: window.get_viewer_font_family().to_string(),
            font_size: window.get_viewer_font_size(),
        }
    }
}
impl Probe {
    pub fn beginning(&mut self, window: &ViewerWindow, revision: u64, active: bool) {
        self.drawing = Some(active.then(|| DrawState {
            revision,
            signature: Signature::capture(window),
        }));
    }

    pub fn rendered(
        &mut self,
        window: &ViewerWindow,
        elapsed_ms: f64,
        revision: u64,
    ) -> Result<bool, String> {
        let Some(drawing) = self.drawing.take() else {
            return Err("opening proof has no matching observed redraw start".into());
        };
        let Some(drawing) = drawing else {
            // A source publication after a blank redraw has not been displayed yet.
            return Ok(false);
        };
        let signature = Signature::capture(window);
        if !drawing.matches_after_layout(revision, &signature) {
            return Err(format!(
                "opening redraw state was superseded before capture: revision {} -> {revision}, before={:?}, after={signature:?}",
                drawing.revision, drawing.signature,
            ));
        }
        self.first.get_or_insert_with(Instant::now);
        if self.signature.as_ref() == Some(&signature) || self.captures.len() >= 8 {
            return Ok(true);
        }
        let index = self.captures.len();
        if drawing.signature.canvas != signature.canvas {
            self.rows.push(format!(
                "opening_layout index={index} frame_revision={revision} before_canvas_logical={:?} rendered_canvas_logical={:?} immutable_state_matched=true native_window={:?}",
                drawing.signature.canvas, signature.canvas, signature.native_window,
            ));
        }
        let scale = signature.scale;
        let display = if signature.retained {
            [
                signature.raster[0] * signature.viewport[0] * scale,
                signature.raster[1] * signature.viewport[1] * scale,
            ]
        } else {
            [signature.viewport[0] * scale, signature.viewport[1] * scale]
        };
        self.rows.push(format!(
            "opening_frame index={index} elapsed_ms={elapsed_ms:.3} scale={scale} canvas_logical={:?} frame_viewport_logical={:?} surface_physical={:?} raster_ratios={:?} ratio_implied_surface_physical={display:?} ratio_implied_pixel_scale={:?} retained={} source={} markdown={} uploads={} font={:?} font_size={} snapshot_timing_excluded=true",
            signature.canvas, signature.viewport, signature.surface, signature.raster,
            [display[0] / signature.surface[0] as f32, display[1] / signature.surface[1] as f32],
            signature.retained, signature.source, signature.markdown, signature.uploads, signature.font, signature.font_size,
        ));
        let snapshot = window
            .window()
            .take_snapshot()
            .map_err(|error| error.to_string())?;
        if Signature::capture(window) != signature {
            return Err("opening snapshot changed the matched redraw geometry".into());
        }
        // Slint renders current state into an offscreen snapshot; the latch above
        // verifies it matches that draw after layout, not a newer publication.
        // Retained gutter geometry uses the same fixed extent as the actual renderer.
        let gutter = if signature.retained {
            signature.gutter * signature.viewport[0]
        } else {
            0.0
        };
        let left = ((signature.canvas[0] + gutter + 4.0) * scale).round() as u32;
        let top = (signature.canvas[1] * scale).round() as u32;
        let width = ((signature.canvas[2] - gutter - 8.0).min(480.0) * scale).floor() as u32;
        let height = (signature.canvas[3].min(180.0) * scale).floor() as u32;
        if width < 16
            || height < 16
            || left + width > snapshot.width()
            || top + height > snapshot.height()
        {
            return Err("opening snapshot crop exceeds native canvas".into());
        }
        let background = [
            signature.background[0],
            signature.background[1],
            signature.background[2],
        ];
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        let mut bands = Vec::new();
        let mut active = None;
        for y in 0..height {
            let mut ink = 0;
            for x in 0..width {
                let pixel = snapshot.as_slice()[((top + y) * snapshot.width() + left + x) as usize];
                pixels.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
                if [pixel.r, pixel.g, pixel.b]
                    .into_iter()
                    .zip(background)
                    .any(|(a, b)| a.abs_diff(b) > 32)
                {
                    ink += 1;
                }
            }
            if ink >= 3 {
                active.get_or_insert(y);
            } else if let Some(start) = active.take() {
                bands.push([start, y - start]);
            }
        }
        if let Some(start) = active {
            bands.push([start, height - start]);
        }
        self.rows.push(format!("opening_pixels index={index} crop_origin_physical={:?} crop_size_physical={:?} ink_row_bands_physical={bands:?} actual_gpu_readback=false capture=slint_rerender_matching_observed_redraw matched_frame_revision={revision}", [left, top], [width, height]));
        self.captures.push(Capture {
            name: format!("opening-{index}"),
            width,
            height,
            pixels,
        });
        self.signature = Some(signature);
        Ok(true)
    }
    pub fn settled(&self) -> bool {
        self.first
            .is_some_and(|first| first.elapsed() >= Duration::from_millis(900))
    }
    pub fn rows(&self) -> impl Iterator<Item = String> + '_ {
        self.rows.iter().cloned()
    }
    pub fn write(&self, directory: &Path) -> io::Result<()> {
        std::fs::create_dir_all(directory)?;
        for capture in &self.captures {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawing() -> DrawState {
        DrawState {
            revision: 1,
            signature: Signature {
                scale: 2.25,
                window_size: [2250, 1620],
                native_window: Some(([2250, 1620], 2.25)),
                viewport: [972.0, 631.0],
                canvas: [6.0, 10.0, 972.0, 631.0],
                surface: [2079, 1786],
                font: "monospace".into(),
                font_size: 14,
                source: true,
                ..Signature::default()
            },
        }
    }

    #[test]
    fn opening_latch_accepts_toolbar_layout_resolved_inside_the_same_draw() {
        let drawing = drawing();
        let mut rendered = drawing.signature.clone();
        rendered.canvas = [6.0, 46.0, 972.0, 595.0];
        assert!(drawing.matches_after_layout(1, &rendered));
        assert!(!drawing.matches_after_layout(2, &rendered));
    }

    #[test]
    fn opening_latch_rejects_content_dpi_window_and_toolbar_changes_during_layout() {
        let drawing = drawing();
        let mutations: &[fn(&mut Signature)] = &[
            |s| s.scale = 1.5,
            |s| s.window_size[0] += 1,
            |s| s.native_window.as_mut().unwrap().0[0] += 1,
            |s| s.native_window.as_mut().unwrap().1 = 1.5,
            |s| s.viewport[1] -= 36.0,
            |s| s.raster[3] = -0.1,
            |s| s.surface[1] += 1,
            |s| s.uploads += 1,
            |s| s.gutter = 0.1,
            |s| s.font_size += 1,
            |s| s.font = "system-ui".into(),
            |s| s.source = false,
            |s| s.markdown = true,
            |s| s.chrome.search_visible = true,
            |s| s.chrome.typography_visible = true,
            |s| s.chrome.ui_font_size = 15.0,
        ];
        for (index, change) in mutations.iter().enumerate() {
            let mut rendered = drawing.signature.clone();
            rendered.canvas = [6.0, 46.0, 972.0, 595.0];
            change(&mut rendered);
            assert!(
                !drawing.matches_after_layout(1, &rendered),
                "mutation {index}"
            );
        }
    }
}
