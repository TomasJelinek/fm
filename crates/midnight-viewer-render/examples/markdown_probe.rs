//! Light/dark visual evidence from the same semantic layout used by the viewer.
use midnight_viewer_render::*;
use std::{ops::Range, path::PathBuf, time::Instant};
mod support;

#[derive(Default)]
struct Fixture {
    text: String,
    blocks: Vec<DocumentBlock>,
    styles: Vec<StyleSpan>,
    tables: Vec<TableRow>,
}
impl Fixture {
    fn block(&mut self, kind: DocumentKind, depth: u8, text: &str) -> Range<usize> {
        if !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        let start = self.text.len();
        self.text.push_str(text);
        let range = start..self.text.len();
        self.blocks.push(DocumentBlock {
            range: range.clone(),
            kind,
            depth,
        });
        range
    }
    fn style(&mut self, text: &str, bold: bool, code: bool, underline: bool) {
        let start = self.text.find(text).unwrap();
        self.styles.push(StyleSpan {
            range: start..start + text.len(),
            size: 16.0,
            weight: if bold { 700 } else { 400 },
            italic: false,
            underline,
            strikethrough: false,
            monospace: code,
        });
    }
    fn table(&mut self, header: bool, left: &str, right: &str) {
        let row = self.block(DocumentKind::Paragraph, 0, &format!("{left}\t{right}"));
        self.tables.push(TableRow {
            range: row.clone(),
            header,
            cells: vec![
                TableCell {
                    range: row.start..row.start + left.len(),
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: row.start + left.len() + 1..row.end,
                    alignment: CellAlignment::Left,
                },
            ],
        });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("_build/qa/markdown-style"));
    std::fs::create_dir_all(&output)?;
    let mut fixture = Fixture::default();
    fixture.block(DocumentKind::Heading(1), 0, "Midnight Explorer");
    fixture.block(DocumentKind::Paragraph,0,
        "A fast, Unicode file manager. Read documentation with bold emphasis, inline_code(), and a useful link. Font overrides remain available after choosing a preset.");
    fixture.style("bold emphasis", true, false, false);
    fixture.style("inline_code()", false, true, false);
    fixture.style("useful link", false, false, true);
    fixture.block(DocumentKind::Heading(2), 0, "Getting started");
    fixture.block(DocumentKind::Paragraph,0,"The GitHub preset uses a familiar heading hierarchy, readable paragraphs and a quiet palette. Čeština and Unicode remain searchable and selectable.");
    fixture.block(DocumentKind::Quote,1,"The viewer renders semantic content locally. It never runs scripts or loads remote content automatically.");
    fixture.block(
        DocumentKind::Code,
        0,
        "fn main() {\n    println!(\"Hello, Midnight Explorer!\");\n}\n",
    );
    fixture.block(DocumentKind::Heading(3), 0, "Designed for everyday work");
    fixture.block(
        DocumentKind::ListItem,
        1,
        "• Browse files and archives with the same keyboard controls.",
    );
    fixture.block(
        DocumentKind::ListItem,
        2,
        "• Keep source positions stable while scrolling.",
    );
    fixture.block(
        DocumentKind::ListItem,
        1,
        "• Copy Unicode text exactly as displayed.",
    );
    fixture.block(DocumentKind::Heading(2), 0, "Available profiles");
    fixture.table(true, "Profile", "Presentation");
    fixture.table(
        false,
        "GitHub",
        "Structured headings, code panels and clear tables",
    );
    fixture.table(false, "Compact", "More content in less space");
    fixture.table(
        false,
        "Comfortable",
        "A narrow reading column and generous line spacing",
    );
    fixture.styles.sort_by_key(|style| style.range.start);
    let link = fixture.text.find("useful link").unwrap();
    for (name, background, foreground, link_color) in [
        (
            "github-light",
            [255, 255, 255, 255],
            [31, 35, 40, 255],
            [9, 105, 218, 255],
        ),
        (
            "github-dark",
            [13, 17, 23, 255],
            [230, 237, 243, 255],
            [88, 166, 255, 255],
        ),
    ] {
        let mut renderer = TextRenderer::default();
        let started = Instant::now();
        let page = renderer.layout_document(
            &fixture.text,
            &TextOptions {
                family: "system-ui",
                size: 16.0,
                foreground,
                wrap_width: Some(736.0),
                line_height: Some(1.5),
                ..Default::default()
            },
            &DocumentStyle {
                blocks: &fixture.blocks,
                colors: &[ColorSpan {
                    range: link..link + 11,
                    color: link_color,
                }],
                styles: &fixture.styles,
                tables: &fixture.tables,
                background,
                start_of_document: true,
            },
        )?;
        let view = Viewport {
            width: 800,
            height: page.height().ceil() as u32 + 32,
            origin: [-32.0, -16.0],
            background,
            selection_color: [50, 110, 190, 150],
        };
        let image = renderer.rasterize(&page, view, None)?;
        support::write_bmp(&output.join(format!("{name}.bmp")), &image)?;
        println!(
            "{name}: {}x{}, layout+raster {:?}",
            image.width,
            image.height,
            started.elapsed()
        );
        let started = Instant::now();
        for step in 0..20 {
            std::hint::black_box(renderer.rasterize(
                &page,
                Viewport {
                    origin: [view.origin[0], view.origin[1] + step as f32 * 0.25],
                    ..view
                },
                None,
            )?);
        }
        println!(
            "{name}: warmed scroll raster {:?}/frame",
            started.elapsed() / 20
        );
    }
    Ok(())
}
