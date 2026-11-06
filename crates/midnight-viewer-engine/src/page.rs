use midnight_core::ViewerAppearance;
use midnight_plugin_host::api::{document, viewer};
use midnight_viewer_render::{ColorSpan, StyleSpan};

#[derive(Clone)]
pub(crate) struct Page {
    pub text: String,
    pub spans: Vec<ColorSpan>,
    pub styles: Vec<StyleSpan>,
    pub anchors: Vec<viewer::Anchor>,
    pub start: u64,
    pub end: u64,
    pub before: bool,
    pub after: bool,
}

fn role_color(role: u16, appearance: &ViewerAppearance) -> [u8; 4] {
    // Blend with the theme foreground to keep every semantic role readable.
    let tint = match role {
        viewer::STYLE_KEYWORD | viewer::STYLE_HEADING => appearance.accent,
        viewer::STYLE_STRING => [130, 188, 100, 255],
        viewer::STYLE_COMMENT => [136, 150, 154, 255],
        viewer::STYLE_NUMBER => [204, 155, 110, 255],
        viewer::STYLE_TYPE | viewer::STYLE_FUNCTION | viewer::STYLE_LINK => [116, 180, 200, 255],
        viewer::STYLE_INVALID => [230, 100, 100, 255],
        _ => appearance.foreground,
    };
    let mut color = tint;
    for channel in 0..3 {
        color[channel] =
            ((u16::from(tint[channel]) + u16::from(appearance.foreground[channel])) / 2) as u8;
    }
    color
}

impl Page {
    pub fn text(page: viewer::TextPage, appearance: &ViewerAppearance) -> Self {
        Self {
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
                })
                .collect(),
            spans: page
                .spans
                .iter()
                .map(|span| ColorSpan {
                    range: span.start as usize..span.end as usize,
                    color: role_color(span.role, appearance),
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
            spans: vec![],
            styles: vec![],
            anchors,
            start: page.source.offset,
            end: page.source.offset + page.source.length,
            before: page.has_before,
            after: page.has_after,
        }
    }

    pub fn document(page: document::DocumentPage, appearance: &ViewerAppearance) -> Self {
        let mut text = String::new();
        let mut spans = Vec::new();
        let mut styles = Vec::new();
        let mut anchors = Vec::new();
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
            text.push_str(prefix);
            let start = text.len();
            text.push_str(&block.text.text);
            anchors.extend(block.text.anchors.iter().map(|anchor| viewer::Anchor {
                text_offset: start as u32 + anchor.text_offset,
                source_offset: anchor.source_offset,
            }));
            if block.kind == document::BLOCK_HEADING && start < text.len() {
                spans.push(ColorSpan {
                    range: start..text.len(),
                    color: role_color(viewer::STYLE_HEADING, appearance),
                });
                styles.push(StyleSpan {
                    range: start..text.len(),
                    weight: 700,
                    italic: false,
                    underline: false,
                    size: appearance.font_size
                        * (1.75 - f32::from(block.heading_level.min(6)) * 0.1),
                });
            } else {
                spans.extend(block.text.spans.iter().map(|span| ColorSpan {
                    range: start + span.start as usize..start + span.end as usize,
                    color: role_color(span.role, appearance),
                }));
                styles.extend(
                    block
                        .text
                        .spans
                        .iter()
                        .filter(|span| span.flags != 0)
                        .map(|span| StyleSpan {
                            range: start + span.start as usize..start + span.end as usize,
                            size: appearance.font_size,
                            weight: if span.flags & 1 != 0 { 700 } else { 400 },
                            italic: span.flags & 2 != 0,
                            underline: span.flags & 4 != 0,
                        }),
                );
            }
        }
        Self {
            text,
            spans,
            styles,
            anchors,
            start: page.source.offset,
            end: page.source.offset + page.source.length,
            before: page.has_before,
            after: page.has_after,
        }
    }
}
