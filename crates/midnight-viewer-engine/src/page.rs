use midnight_core::ViewerAppearance;
use midnight_plugin_host::api::{document, viewer};
use midnight_viewer_render::{
    CellAlignment, ColorSpan, DocumentBlock, DocumentKind, DocumentStyle, StyleSpan, TableCell,
    TableRow, TextOptions, TextPage, TextRenderer,
};

#[cfg(test)]
#[path = "page_color_tests.rs"]
mod color_tests;
#[cfg(test)]
#[path = "page_document_tests.rs"]
mod document_tests;

#[derive(Clone)]
pub(crate) struct Page {
    pub text: String,
    pub spans: Vec<ColorSpan>,
    pub styles: Vec<StyleSpan>,
    pub tables: Vec<TableRow>,
    pub document_blocks: Vec<DocumentBlock>,
    /// Font size used when converting semantic styles; reflow scales these styles.
    pub style_size: f32,
    pub anchors: Vec<viewer::Anchor>,
    pub start: u64,
    pub end: u64,
    pub before: bool,
    pub after: bool,
}

fn role_color(role: u16, appearance: &ViewerAppearance) -> [u8; 4] {
    let tint = match role {
        viewer::STYLE_KEYWORD | viewer::STYLE_HEADING => appearance.accent,
        viewer::STYLE_STRING => [150, 220, 95, 255],
        viewer::STYLE_COMMENT => [130, 162, 210, 255],
        viewer::STYLE_NUMBER => [255, 160, 90, 255],
        viewer::STYLE_TYPE => [255, 190, 80, 255],
        viewer::STYLE_FUNCTION | viewer::STYLE_LINK => [90, 190, 255, 255],
        viewer::STYLE_INVALID => [255, 105, 105, 255],
        _ => return appearance.foreground,
    };
    let color = midnight_core::readable_rgb(
        [tint[0], tint[1], tint[2]],
        [
            appearance.background[0],
            appearance.background[1],
            appearance.background[2],
        ],
    );
    [color[0], color[1], color[2], 255]
}

struct Palette {
    roles: [[u8; 4]; 12],
    foreground: [u8; 4],
}

impl Palette {
    fn new(appearance: &ViewerAppearance) -> Self {
        Self {
            roles: std::array::from_fn(|role| role_color(role as u16, appearance)),
            foreground: appearance.foreground,
        }
    }

    fn color(&self, role: u16) -> [u8; 4] {
        self.roles
            .get(usize::from(role))
            .copied()
            .unwrap_or(self.foreground)
    }
}

impl Page {
    pub fn text(page: viewer::TextPage, appearance: &ViewerAppearance) -> Self {
        let palette = Palette::new(appearance);
        Self {
            tables: Vec::new(),
            document_blocks: Vec::new(),
            style_size: appearance.font_size,
            anchors: page.anchors,
            styles: page
                .spans
                .iter()
                .filter(|span| span.flags != 0)
                .map(|span| StyleSpan {
                    range: span.start as usize..span.end as usize,
                    size: appearance.font_size,
                    weight: if span.flags & 1 != 0 { 700 } else { 400 },
                    italic: span.flags & 2 != 0,
                    underline: span.flags & 4 != 0,
                    strikethrough: false,
                    monospace: false,
                })
                .collect(),
            spans: page
                .spans
                .iter()
                .map(|span| ColorSpan {
                    range: span.start as usize..span.end as usize,
                    color: palette.color(span.role),
                })
                .collect(),
            text: page.text,
            start: page.source.offset,
            end: page.source.offset.saturating_add(page.source.length),
            before: page.has_before,
            after: page.has_after,
        }
    }

    pub fn binary(page: viewer::BinaryPage, bytes: &[u8]) -> Self {
        use std::fmt::Write;
        let mut text = String::new();
        let mut anchors = Vec::with_capacity(bytes.len() + 1);
        for (row, line) in bytes.chunks(16).enumerate() {
            let offset = page.source.offset + (row as u64 * 16);
            let _ = write!(text, "{offset:016X}  ");
            for index in 0..16 {
                if let Some(byte) = line.get(index) {
                    anchors.push(viewer::Anchor {
                        text_offset: text.len() as u32,
                        source_offset: offset + index as u64,
                    });
                    let _ = write!(text, "{byte:02X} ");
                } else {
                    text.push_str("   ");
                }
            }
            if row * 16 + line.len() == bytes.len() {
                anchors.push(viewer::Anchor {
                    text_offset: text.len() as u32,
                    source_offset: page.source.offset + bytes.len() as u64,
                });
            }
            text.push_str(" │ ");
            for byte in line {
                text.push(if (32..127).contains(byte) {
                    char::from(*byte)
                } else {
                    '·'
                });
            }
            text.push('\n');
        }
        Self {
            text,
            tables: Vec::new(),
            document_blocks: Vec::new(),
            spans: vec![],
            styles: vec![],
            style_size: 14.0,
            anchors,
            start: page.source.offset,
            end: page.source.offset + page.source.length,
            before: page.has_before,
            after: page.has_after,
        }
    }

    pub fn document(page: document::DocumentPage, appearance: &ViewerAppearance) -> Self {
        let palette = Palette::new(appearance);
        let mut text = String::new();
        let mut spans = Vec::new();
        let mut styles = Vec::new();
        let mut anchors = Vec::new();
        let mut tables = Vec::new();
        let mut document_blocks = Vec::new();
        for block in page.blocks {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let prefix = match block.kind {
                document::BLOCK_LIST_ITEM => "• ",
                document::BLOCK_QUOTE => "│ ",
                document::BLOCK_RULE => "────────────",
                _ => "",
            };
            let block_start = text.len();
            text.push_str(prefix);
            let start = text.len();
            text.push_str(&block.text.text);
            document_blocks.push(DocumentBlock {
                range: block_start..text.len(),
                kind: match block.kind {
                    document::BLOCK_HEADING => DocumentKind::Heading(block.heading_level),
                    document::BLOCK_CODE => DocumentKind::Code,
                    document::BLOCK_QUOTE => DocumentKind::Quote,
                    document::BLOCK_LIST_ITEM => DocumentKind::ListItem,
                    document::BLOCK_RULE => DocumentKind::Rule,
                    _ => DocumentKind::Paragraph,
                },
                depth: block.depth,
            });
            if let Some(row) = &block.table {
                tables.push(TableRow {
                    range: start..text.len(),
                    header: row.header,
                    cells: row
                        .cells
                        .iter()
                        .map(|cell| TableCell {
                            range: start + cell.start as usize..start + cell.end as usize,
                            alignment: match cell.alignment {
                                document::TABLE_ALIGN_CENTER => CellAlignment::Center,
                                document::TABLE_ALIGN_RIGHT => CellAlignment::Right,
                                _ => CellAlignment::Left,
                            },
                        })
                        .collect(),
                });
            }
            anchors.extend(block.text.anchors.iter().map(|anchor| viewer::Anchor {
                text_offset: start as u32 + anchor.text_offset,
                source_offset: anchor.source_offset,
            }));
            if block.kind == document::BLOCK_HEADING && start < text.len() {
                spans.push(ColorSpan {
                    range: start..text.len(),
                    color: palette.color(viewer::STYLE_HEADING),
                });
            } else {
                spans.extend(block.text.spans.iter().map(|span| ColorSpan {
                    range: start + span.start as usize..start + span.end as usize,
                    color: palette.color(span.role),
                }));
            }
            append_document_styles(&block, start, appearance.font_size, &mut styles);
        }
        Self {
            text,
            spans,
            styles,
            tables,
            document_blocks,
            style_size: appearance.font_size,
            anchors,
            start: page.source.offset,
            end: page.source.offset + page.source.length,
            before: page.has_before,
            after: page.has_after,
        }
    }

    /// Reflow the accepted decoded page without changing text, source anchors or the
    /// chosen font. GitHub alone uses semantic block geometry and decoration.
    pub(crate) fn layout(
        &self,
        renderer: &mut TextRenderer,
        options: &TextOptions<'_>,
        github: bool,
        background: [u8; 4],
    ) -> Result<TextPage, midnight_viewer_render::LayoutError> {
        let styles: Vec<_> = self
            .styles
            .iter()
            .cloned()
            .map(|mut style| {
                style.size = (style.size * options.size / self.style_size.max(1.0))
                    .min(256.0 / options.scale);
                style
            })
            .collect();
        if github && !self.document_blocks.is_empty() {
            renderer.layout_document(
                &self.text,
                options,
                &DocumentStyle {
                    colors: &self.spans,
                    styles: &styles,
                    tables: &self.tables,
                    blocks: &self.document_blocks,
                    background,
                    start_of_document: !self.before,
                },
            )
        } else {
            renderer.layout_with_tables(&self.text, options, &self.spans, &styles, &self.tables)
        }
    }
}

fn append_document_styles(
    block: &document::Block,
    start: usize,
    size: f32,
    styles: &mut Vec<StyleSpan>,
) {
    let strikes = block.strikethrough.as_deref().unwrap_or_default();
    let mut boundaries = vec![0, block.text.text.len()];
    for span in &block.text.spans {
        boundaries.extend([span.start as usize, span.end as usize]);
    }
    for strike in strikes {
        boundaries.extend([strike.start as usize, strike.end as usize]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut span_index = 0;
    let mut strike_index = 0;
    for range in boundaries.windows(2) {
        let [a, b] = [range[0], range[1]];
        while block
            .text
            .spans
            .get(span_index)
            .is_some_and(|span| span.end as usize <= a)
        {
            span_index += 1;
        }
        while strikes
            .get(strike_index)
            .is_some_and(|strike| strike.end as usize <= a)
        {
            strike_index += 1;
        }
        let span = block
            .text
            .spans
            .get(span_index)
            .filter(|span| span.start as usize <= a);
        let flags = span.map_or(0, |span| span.flags);
        let strikethrough = strikes
            .get(strike_index)
            .is_some_and(|strike| strike.start as usize <= a);
        let monospace = block.kind == document::BLOCK_CODE
            || span.is_some_and(|span| span.role == viewer::STYLE_CODE);
        let heading = block.kind == document::BLOCK_HEADING;
        if flags != 0 || strikethrough || monospace || heading {
            styles.push(StyleSpan {
                range: start + a..start + b,
                size: if heading {
                    size * (1.75 - f32::from(block.heading_level.min(6)) * 0.1)
                } else {
                    size
                },
                weight: if flags & 1 != 0 || heading { 700 } else { 400 },
                italic: flags & 2 != 0,
                underline: flags & 4 != 0,
                strikethrough,
                monospace,
            });
        }
    }
}

#[cfg(test)]
mod github_layout_tests {
    use super::*;

    #[test]
    fn github_layout_keeps_source_anchors_and_font_scaling_without_changing_other_presets() {
        let appearance = ViewerAppearance {
            family: "sans-serif".into(),
            font_size: 16.0,
            foreground: [31, 35, 40, 255],
            background: [255; 4],
            selection: [50, 100, 180, 255],
            accent: [200, 80, 40, 255],
        };
        let page = Page::document(
            document::DocumentPage {
                revision: 1,
                source: viewer::SourceRange {
                    offset: 0,
                    length: 22,
                },
                has_before: false,
                has_after: false,
                blocks: vec![
                    block(document::BLOCK_HEADING, 1, "Heading", 0),
                    block(document::BLOCK_CODE, 0, "let value = 1;", 8),
                ],
            },
            &appearance,
        );
        let options = TextOptions {
            family: "sans-serif",
            size: 16.0,
            foreground: appearance.foreground,
            wrap_width: Some(400.0),
            line_height: Some(1.5),
            ..Default::default()
        };
        let mut renderer = TextRenderer::default();
        let github = page
            .layout(&mut renderer, &options, true, appearance.background)
            .unwrap();
        let legacy = page
            .layout(&mut renderer, &options, false, appearance.background)
            .unwrap();
        let large = page
            .layout(
                &mut renderer,
                &TextOptions {
                    size: 24.0,
                    ..options
                },
                true,
                appearance.background,
            )
            .unwrap();
        assert_eq!(legacy.caret_geometry(9).unwrap().x0, 0.0);
        assert_eq!(github.caret_geometry(9).unwrap().x0, 16.0);
        assert_eq!(large.caret_geometry(9).unwrap().x0, 24.0);
        assert_eq!(github.selected_text(0..page.text.len()).unwrap(), page.text);
        assert_eq!(legacy.selected_text(0..page.text.len()).unwrap(), page.text);
        assert_eq!(
            (page.anchors[1].text_offset, page.anchors[1].source_offset),
            (9, 8)
        );
    }

    fn block(kind: u16, heading_level: u8, text: &str, offset: u64) -> document::Block {
        document::Block {
            kind,
            heading_level,
            depth: 0,
            links: vec![],
            table: None,
            strikethrough: None,
            text: viewer::TextPage {
                revision: 1,
                source: viewer::SourceRange {
                    offset,
                    length: text.len() as u64,
                },
                text: text.into(),
                spans: vec![],
                anchors: vec![viewer::Anchor {
                    text_offset: 0,
                    source_offset: offset,
                }],
                has_before: false,
                has_after: false,
                partial_line: false,
            },
        }
    }
}
