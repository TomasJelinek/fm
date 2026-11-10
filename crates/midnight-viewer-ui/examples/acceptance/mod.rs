//! Observable native acceptance steps; all file output runs off the UI event loop.
mod controls;
mod fixtures;
pub use fixtures::fixtures;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, window::WindowInner,
};
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const COMPLETE: usize = 13;

pub struct Probe {
    pub stage: usize,
    pub error: Option<String>,
    pub writes: Vec<tokio::task::JoinHandle<std::result::Result<(), String>>>,
    output: PathBuf,
    writer: tokio::runtime::Handle,
    started: Instant,
    controls: controls::Controls,
    image_ready_since: Option<Instant>,
}

impl Probe {
    pub fn new(output: PathBuf, writer: tokio::runtime::Handle) -> Self {
        Self {
            stage: 0,
            error: None,
            writes: Vec::new(),
            output,
            writer,
            started: Instant::now(),
            controls: Default::default(),
            image_ready_since: None,
        }
    }

    pub fn step(&mut self, window: &ViewerWindow, parent: &ViewerWindow) -> Result<bool> {
        if self.started.elapsed() > Duration::from_secs(60) {
            return Err(format!("Stage {} timed out: {}", self.stage, window.get_status()).into());
        }
        let text = window.get_accessible_text();
        match self.stage {
            0 if text.starts_with("FIRST") && window.get_horizontal_extent() < 1.0 => {
                require(
                    window.window().size() == parent.window().size(),
                    &format!(
                        "Viewer must cover parent's client size: viewer {:?}, parent {:?}",
                        window.window().size(),
                        parent.window().size()
                    ),
                )?;
                require(
                    window.window().position() == parent.window().position(),
                    "Viewer must match parent's outer origin",
                )?;
                require(
                    pixels(window)?
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[0] > 150),
                    "Text must paint visible glyphs",
                )?;
                self.snapshot(window, "01-text")?;
                click(window, "Word wrap")?;
            }
            1 if window.get_wrap_enabled() && window.get_horizontal_extent() == 1.0 => {
                require(text.starts_with("FIRST"), "Wrap must retain text")?;
                self.snapshot(window, "02-wrap")?;
                control(window, "f");
                key(window, "needle");
                key(window, " ");
                key(window, "target");
                require(
                    window.get_query() == "needle target",
                    "Search widget must retain spaces",
                )?;
                key(window, Key::Return);
            }
            2 if text.contains("needle target") && window.get_position() > 0.5 => {
                self.snapshot(window, "03-search")?;
                window.invoke_focus_content();
                key(window, " ");
            }
            3 if window.get_caption().contains("binary.bin") && text.contains("00 01 02 FF 41") => {
                require(text.contains("│ ····A"), "Binary needs printable column")?;
                require(
                    !window.get_wrap_available(),
                    "Binary must keep fixed columns",
                )?;
                self.snapshot(window, "04-binary")?;
                let index = window
                    .get_modes()
                    .iter()
                    .position(|mode| mode == "Text")
                    .ok_or("Text mode unavailable")?;
                window.invoke_change_mode(index as i32);
            }
            4 if window.get_caption().contains("binary.bin") && window.get_wrap_available() => {
                require(
                    !text.contains("00 01 02 FF 41"),
                    "Text mode must change the binary presentation",
                )?;
                window.invoke_focus_content();
                key(window, " ");
            }
            5 if window.get_caption().contains("source.rs") && text.contains("fn main") => {
                if !self.controls.source(window)? {
                    return Ok(false);
                }
                let surface = pixels(window)?;
                require(
                    surface.as_bytes().as_chunks::<4>().0.iter().any(|pixel| {
                        pixel[1] > pixel[0].saturating_add(25)
                            && pixel[1] > pixel[2].saturating_add(25)
                    }),
                    "Source strings must paint green syntax pixels",
                )?;
                self.snapshot(window, "05-source")?;
                window.invoke_focus_content();
                key(window, " ");
            }
            6 if window.get_caption().contains("markdown.md")
                && window.get_markdown_mode()
                && !text.is_empty() =>
            {
                if !self.controls.markdown(window)? {
                    return Ok(false);
                }
                require(
                    !text.contains("**") && !text.contains("# Heading"),
                    "Markdown must show parsed text",
                )?;
                require(
                    text.contains("Café 世界")
                        && text.contains("ready")
                        && text.contains("Done")
                        && !text.contains("~~old~~"),
                    "Markdown must retain Unicode table cells and parse tasks and strikeout",
                )?;
                require(
                    pixels(window)?
                        .as_bytes()
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel[2] > pixel[0].saturating_add(50)),
                    "Markdown heading must paint styled pixels",
                )?;
                self.snapshot(window, "06-markdown")?;
                window.invoke_focus_content();
                key(window, " ");
            }
            7 if window.get_caption().contains("image.bmp") && window.get_image_mode() => {
                // Hiding text controls changes the physical viewport. Zoom
                // intentionally rejects input until that geometry is acknowledged.
                if self
                    .image_ready_since
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    < Duration::from_millis(300)
                    || !window.get_position_context().starts_with("image:")
                {
                    return Ok(false);
                }
                require(
                    window.get_document_page() == 0,
                    "Ordinary image cannot expose PDF paging",
                )?;
                require(
                    !window.get_search_visible(),
                    "Image hides the text search row",
                )?;
                require(
                    has_pixel(window, [255, 50, 100, 255])?,
                    "Image must contain decoded source pixels",
                )?;
                self.snapshot(window, "07-image")?;
                click(window, "Zoom image in")?;
            }
            8 if window.get_caption().contains("image.bmp")
                && window.get_zoom_percent() > 100.0 =>
            {
                self.snapshot(window, "08-image-zoom")?;
                click(window, "Actual image pixels")?;
            }
            9 if window.get_caption().contains("image.bmp")
                && window.get_zoom_percent() == 100.0 =>
            {
                key(window, " ");
            }
            10 if window.get_caption().contains("document.pdf")
                && window.get_document_page() == 1 =>
            {
                require(
                    window.get_document_pages() == 2,
                    "PDF must discover both pages",
                )?;
                require(
                    has_pixel(window, [255, 0, 0, 255])?,
                    "PDF first page must render red",
                )?;
                self.snapshot(window, "09-pdf-first")?;
                click(window, "Next document page (Ctrl+PageDown)")?;
            }
            11 if window.get_document_page() == 2 => {
                require(
                    has_pixel(window, [0, 0, 255, 255])?,
                    "PDF second page must render blue",
                )?;
                self.snapshot(window, "10-pdf-second")?;
                control(window, Key::PageUp);
            }
            12 if window.get_document_page() == 1 => {
                require(
                    has_pixel(window, [255, 0, 0, 255])?,
                    "PDF keyboard previous must render the original page",
                )?;
                key(window, Key::Escape);
                require(!window.window().is_visible(), "Escape must close viewer")?;
                require(
                    parent.window().is_visible(),
                    "Closing viewer must preserve parent",
                )?;
            }
            _ => return Ok(false),
        }
        eprintln!(
            "Native stage {} passed after {:?}: {}",
            self.stage,
            self.started.elapsed(),
            window.get_status()
        );
        self.stage += 1;
        Ok(self.stage == COMPLETE)
    }

    fn snapshot(&mut self, window: &ViewerWindow, name: &str) -> Result<()> {
        let image = window.window().take_snapshot()?;
        let path = self.output.join(format!("{name}.png"));
        self.writes.push(self.writer.spawn_blocking(move || {
            let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
            let mut encoder = png::Encoder::new(file, image.width(), image.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .map_err(|error| error.to_string())?
                .write_image_data(image.as_bytes())
                .map_err(|error| error.to_string())
        }));
        Ok(())
    }
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn pixels(window: &ViewerWindow) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    if window.get_tiled() {
        // Image panning retains independent textures rather than a single
        // raster. Verify the actual composed native window in that mode.
        return Ok(window.window().take_snapshot()?);
    }
    window
        .get_surface()
        .to_rgba8()
        .ok_or_else(|| "Viewer did not expose raster pixels".into())
}

fn has_pixel(window: &ViewerWindow, expected: [u8; 4]) -> Result<bool> {
    Ok(pixels(window)?
        .as_bytes()
        .as_chunks::<4>()
        .0
        .contains(&expected))
}

fn key(window: &ViewerWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn control(window: &ViewerWindow, text: impl Into<slint::SharedString>) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(window, text);
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn find_control(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_control(item, label) {
            return Some(found);
        }
    }
    None
}

fn click(window: &ViewerWindow, label: &str) -> Result<()> {
    // Conditional controls appear when a backend mode frame arrives. Resolve
    // their layout before inspecting the accessibility tree or pointer bounds.
    let _ = window.window().take_snapshot()?;
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let item = find_control(root, label).ok_or_else(|| {
        format!(
            "Missing visible control: {label}; source={}, markdown={}, status={}",
            window.get_source_mode(),
            window.get_markdown_mode(),
            window.get_status()
        )
    })?;
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    Ok(())
}
