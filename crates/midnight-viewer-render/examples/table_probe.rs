//! Render the first pipe table of a local Markdown fixture through the host renderer.
//! This diagnostic is deliberately not a Markdown parser or viewer input path.
use midnight_viewer_render::*;
use std::{io, path::PathBuf};
mod support;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let source = PathBuf::from(arguments.next().ok_or("expected a Markdown fixture")?);
    let output = PathBuf::from(arguments.next().ok_or("expected an output directory")?);
    if std::fs::metadata(&source)?.len() > MAX_TEXT_BYTES as u64 {
        return Err(io::Error::other("fixture exceeds the renderer text limit").into());
    }
    let source = std::fs::read_to_string(source)?;
    let lines: Vec<_> = source
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .collect();
    if lines.len() < 3 {
        return Err("expected a header, separator and body row".into());
    }
    let alignments: Vec<_> = lines[1]
        .trim_matches('|')
        .split('|')
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => CellAlignment::Center,
                (_, true) => CellAlignment::Right,
                _ => CellAlignment::Left,
            }
        })
        .collect();
    let mut text = String::new();
    let mut rows = Vec::new();
    let mut blocks = Vec::new();
    for (index, line) in lines.iter().enumerate().filter(|(index, _)| *index != 1) {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        let start = text.len();
        let mut cells = Vec::new();
        for (column, cell) in line.trim_matches('|').split('|').enumerate() {
            if column > 0 {
                text.push('\t');
            }
            let start = text.len();
            text.push_str(cell.trim());
            cells.push(TableCell {
                range: start..text.len(),
                alignment: *alignments.get(column).ok_or("column count mismatch")?,
            });
        }
        let range = start..text.len();
        blocks.push(DocumentBlock {
            range: range.clone(),
            kind: DocumentKind::Paragraph,
            depth: 0,
        });
        rows.push(TableRow {
            range,
            cells,
            header: index == 0,
        });
    }
    let mut fonts = parley::FontContext::default();
    let families: Vec<_> = fonts
        .collection
        .generic_families(parley::GenericFamily::SystemUi)
        .collect();
    for family in families {
        if let Some(name) = fonts.collection.family_name(family) {
            println!("system-ui: {name}");
        }
    }
    std::fs::create_dir_all(&output)?;
    for (name, width, background, foreground) in [
        ("table-light", 980.0, [255; 4], [31, 35, 40, 255]),
        ("table-narrow", 560.0, [255; 4], [31, 35, 40, 255]),
        ("table-dark", 980.0, [13, 17, 23, 255], [230, 237, 243, 255]),
    ] {
        let mut renderer = TextRenderer::default();
        let page = renderer.layout_document(
            &text,
            &TextOptions {
                family: "system-ui",
                size: 16.0,
                foreground,
                wrap_width: Some(width),
                line_height: Some(1.5),
                ..Default::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background,
                start_of_document: false,
            },
        )?;
        let raster = renderer.rasterize(
            &page,
            Viewport {
                width: page.width().ceil().max(width) as u32 + 48,
                height: page.height().ceil() as u32 + 48,
                origin: [-24.0; 2],
                background,
                selection_color: [50, 110, 190, 150],
            },
            None,
        )?;
        support::write_bmp(&output.join(format!("{name}.bmp")), &raster)?;
        println!(
            "{name}: content {:.1} × {:.1}, requested width {width}",
            page.width(),
            page.height()
        );
    }
    Ok(())
}
