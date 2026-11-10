//! Host-owned GitHub-style document layout; semantic ranges remain decoded UTF-8.
use crate::raster::Fill;
use crate::tables::{Composite, PlacedPage, TablePresentation, clipped_colors, clipped_styles};
use crate::{
    ColorSpan, LayoutError, Rect, StyleSpan, TableRow, TextOptions, TextPage, TextRenderer,
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Paragraph,
    Heading(u8),
    Code,
    Quote,
    ListItem,
    Rule,
}

#[derive(Clone)]
pub struct DocumentBlock {
    pub range: Range<usize>,
    pub kind: DocumentKind,
    pub depth: u8,
}

pub struct DocumentStyle<'a> {
    pub colors: &'a [ColorSpan],
    pub styles: &'a [StyleSpan],
    pub tables: &'a [TableRow],
    pub blocks: &'a [DocumentBlock],
    pub background: [u8; 4],
    pub start_of_document: bool,
}

impl TextRenderer {
    pub fn layout_document(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        document: &DocumentStyle<'_>,
    ) -> Result<TextPage, LayoutError> {
        validate(text, options, document)?;
        if document.blocks.is_empty() {
            return self.layout_with_tables(
                text,
                options,
                document.colors,
                document.styles,
                document.tables,
            );
        }
        let em = options.size * options.scale;
        let colors = Palette::new(options.foreground, document.background);
        let mut result = self.layout_with_options("", options, &[])?;
        let mut grid = Composite {
            parts: Vec::new(),
            backgrounds: Vec::new(),
            borders: Vec::new(),
            border_color: colors.border,
            width: 0.0,
            height: if document.start_of_document { em } else { 0.0 },
        };
        let mut index = 0;
        let mut previous = None;
        while let Some(block) = document.blocks.get(index) {
            if index > 0 {
                grid.height += match (previous, block.kind) {
                    (Some(DocumentKind::ListItem), DocumentKind::ListItem) => em * 0.25,
                    (_, DocumentKind::Heading(_)) => em * 1.5,
                    _ => em,
                };
            }
            if document.tables.iter().any(|row| row.range == block.range) {
                let mut end = index + 1;
                while document.blocks.get(end).is_some_and(|next| {
                    document
                        .tables
                        .iter()
                        .any(|row| row.range == next.range && !row.header)
                }) {
                    end += 1;
                }
                let range = block.range.start..document.blocks[end - 1].range.end;
                let rows: Vec<_> = document
                    .tables
                    .iter()
                    .filter(|row| row.range.start >= range.start && row.range.end <= range.end)
                    .map(|row| TableRow {
                        range: row.range.start - range.start..row.range.end - range.start,
                        cells: row
                            .cells
                            .iter()
                            .map(|cell| crate::TableCell {
                                range: cell.range.start - range.start..cell.range.end - range.start,
                                alignment: cell.alignment,
                            })
                            .collect(),
                        header: row.header,
                    })
                    .collect();
                let page = self.layout_tables(
                    &text[range.clone()],
                    options,
                    &clipped_colors(document.colors, &range),
                    &clipped_styles(document.styles, &range),
                    &rows,
                    Some(TablePresentation {
                        border: colors.border,
                        alternate: colors.subtle,
                    }),
                )?;
                place(&mut grid, page, range, [0.0, 0.0]);
                index = end;
            } else {
                self.place_document_block(text, options, document, block, &colors, &mut grid)?;
                index += 1;
            }
            previous = Some(block.kind);
        }
        grid.height += if document.start_of_document { em } else { 0.0 };
        result.text = text.into();
        result.composite = Some(grid);
        Ok(result)
    }

    fn place_document_block(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        document: &DocumentStyle<'_>,
        block: &DocumentBlock,
        colors: &Palette,
        grid: &mut Composite,
    ) -> Result<(), LayoutError> {
        let em = options.size * options.scale;
        let mut range = block.range.clone();
        // Legacy flattening prefixes stay in the copied text and source map. Visual
        // quote/rule decoration is drawn geometrically instead of as text characters.
        if block.kind == DocumentKind::Quote && text[range.clone()].starts_with("│ ") {
            range.start += "│ ".len();
        }
        if block.kind == DocumentKind::Rule {
            range.end = range.start;
        } else if block.kind == DocumentKind::Code && text[range.clone()].ends_with('\n') {
            range.end -= 1;
            if text[range.clone()].ends_with('\r') {
                range.end -= 1;
            }
        }
        let (indent, padding) = match block.kind {
            DocumentKind::Code => (em, em),
            DocumentKind::Quote => (em * f32::from(block.depth.max(1)), 0.0),
            DocumentKind::ListItem => (em * 2.0 * f32::from(block.depth.max(1)), 0.0),
            _ => (0.0, 0.0),
        };
        let ratio = match block.kind {
            DocumentKind::Heading(level) => heading_ratio(level),
            DocumentKind::Code => 0.85,
            _ => 1.0,
        };
        let block_options = TextOptions {
            size: (options.size * ratio).min(256.0 / options.scale),
            weight: if matches!(block.kind, DocumentKind::Heading(_)) {
                600
            } else {
                options.weight
            },
            line_height: match block.kind {
                DocumentKind::Heading(_) => Some(1.25),
                DocumentKind::Code => Some(1.45),
                _ => options.line_height,
            },
            family: if block.kind == DocumentKind::Code {
                "monospace"
            } else {
                options.family
            },
            literal_family: block.kind != DocumentKind::Code && options.literal_family,
            wrap_width: if block.kind == DocumentKind::Code {
                None
            } else {
                options
                    .wrap_width
                    .map(|width| (width - indent - padding).max(em))
            },
            foreground: if matches!(block.kind, DocumentKind::Quote | DocumentKind::Heading(6)) {
                colors.muted
            } else {
                options.foreground
            },
            ..*options
        };
        let mut styles = clipped_styles(document.styles, &range);
        for style in &mut styles {
            if matches!(block.kind, DocumentKind::Heading(_)) {
                style.size = block_options.size;
                style.weight = 600;
            } else if style.monospace {
                style.size = options.size * 0.85;
            }
        }
        let mut spans = clipped_colors(document.colors, &range);
        if matches!(block.kind, DocumentKind::Heading(_) | DocumentKind::Quote) {
            spans.clear();
        }
        let page =
            self.layout_with_styles(&text[range.clone()], &block_options, &spans, &styles)?;
        let top = grid.height;
        let height = if block.kind == DocumentKind::Rule {
            options.scale
        } else {
            page.height() + padding * 2.0
        };
        let width = options
            .wrap_width
            .unwrap_or(page.width() + indent + padding)
            .max(page.width() + indent + padding);
        match block.kind {
            DocumentKind::Code => grid.backgrounds.push(Fill {
                rect: rect(0.0, top, width, top + height),
                color: colors.subtle,
                radius: 6.0 * options.scale,
            }),
            DocumentKind::Quote => grid.borders.push(rect(0.0, top, em * 0.25, top + height)),
            DocumentKind::Rule => grid.borders.push(rect(0.0, top, width, top + height)),
            _ => {}
        }
        if block.kind != DocumentKind::Code {
            for style in styles.iter().filter(|style| style.monospace) {
                for rectangle in page.selection_geometry(style.range.clone())? {
                    grid.backgrounds.push(Fill {
                        rect: Rect {
                            x0: rectangle.x0 + f64::from(indent - em * 0.15),
                            x1: rectangle.x1 + f64::from(indent + em * 0.15),
                            y0: rectangle.y0 + f64::from(top),
                            y1: rectangle.y1 + f64::from(top),
                        },
                        color: colors.subtle,
                        radius: 3.0 * options.scale,
                    });
                }
            }
        }
        let page_width = page.width();
        grid.parts.push(PlacedPage {
            range,
            table_row: None,
            origin: [indent, top + padding],
            width: page_width,
            page,
        });
        grid.width = grid.width.max(width);
        grid.height += height;
        if matches!(block.kind, DocumentKind::Heading(1 | 2)) {
            grid.height += block_options.size * options.scale * 0.3;
            grid.borders
                .push(rect(0.0, grid.height, width, grid.height + options.scale));
            grid.height += options.scale;
        }
        Ok(())
    }
}

fn place(grid: &mut Composite, page: TextPage, range: Range<usize>, inset: [f32; 2]) {
    grid.width = grid.width.max(page.width() + inset[0]);
    let top = grid.height;
    grid.height += page.height() + inset[1];
    grid.parts.push(PlacedPage {
        range,
        table_row: None,
        origin: [inset[0], top + inset[1]],
        width: page.width(),
        page,
    });
}

fn heading_ratio(level: u8) -> f32 {
    match level {
        1 => 2.0,
        2 => 1.5,
        3 => 1.25,
        4 => 1.0,
        5 => 0.875,
        _ => 0.85,
    }
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect {
        x0: x0.into(),
        y0: y0.into(),
        x1: x1.into(),
        y1: y1.into(),
    }
}

struct Palette {
    border: [u8; 4],
    subtle: [u8; 4],
    muted: [u8; 4],
}
impl Palette {
    fn new(foreground: [u8; 4], background: [u8; 4]) -> Self {
        let mixed = |amount: f32| {
            std::array::from_fn(|index| {
                if index == 3 {
                    255
                } else {
                    (f32::from(background[index]) * (1.0 - amount)
                        + f32::from(foreground[index]) * amount)
                        .round() as u8
                }
            })
        };
        let muted = mixed(0.72);
        Self {
            border: mixed(0.2),
            subtle: mixed(0.045),
            muted: if contrast(muted, background) >= 4.5 {
                muted
            } else {
                foreground
            },
        }
    }
}

fn contrast(a: [u8; 4], b: [u8; 4]) -> f32 {
    let luminance = |color: [u8; 4]| {
        [0.2126, 0.7152, 0.0722]
            .into_iter()
            .zip(color)
            .map(|(weight, channel)| {
                let v = f32::from(channel) / 255.0;
                weight
                    * if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f32>()
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn validate(
    text: &str,
    options: &TextOptions<'_>,
    document: &DocumentStyle<'_>,
) -> Result<(), LayoutError> {
    crate::validate_input(text, options.size, options.scale, document.colors)?;
    crate::validate_styles(text, options.scale, document.styles)?;
    crate::tables::validate_tables(
        text,
        options,
        document.colors,
        document.styles,
        document.tables,
    )?;
    if document.blocks.len() > 512 {
        return Err(LayoutError::LimitExceeded);
    }
    let mut end = 0;
    for block in document.blocks {
        if block.range.start < end
            || text.get(block.range.clone()).is_none()
            || block.depth > 32
            || !text[end..block.range.start].trim().is_empty()
            || matches!(block.kind,DocumentKind::Heading(level) if !(1..=6).contains(&level))
        {
            return Err(LayoutError::InvalidRange);
        }
        end = block.range.end;
    }
    if !document.blocks.is_empty() && !text[end..].trim().is_empty() {
        return Err(LayoutError::InvalidRange);
    }
    // The host resolves overlapping semantic decorations into disjoint runs. This
    // also bounds inline background geometry by page size, not spans × page lines.
    let mut end = 0;
    for style in document.styles {
        if style.range.start < end {
            return Err(LayoutError::InvalidRange);
        }
        end = style.range.end;
    }
    Ok(())
}
