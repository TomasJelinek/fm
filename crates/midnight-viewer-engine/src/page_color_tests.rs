use super::*;

fn appearance(background: [u8; 4], foreground: [u8; 4]) -> ViewerAppearance {
    ViewerAppearance {
        family: String::new(),
        font_size: 14.0,
        foreground,
        background,
        selection: [70, 90, 130, 255],
        accent: [200, 120, 255, 255],
    }
}

#[test]
fn readable_keywords_keep_their_saturated_theme_accent() {
    let appearance = appearance([40, 42, 54, 255], [248, 248, 242, 255]);
    assert_eq!(
        role_color(viewer::STYLE_KEYWORD, &appearance),
        appearance.accent
    );
}

#[test]
fn plain_text_remains_the_configured_foreground() {
    let appearance = appearance([40, 42, 54, 255], [248, 248, 242, 255]);
    assert_eq!(role_color(0, &appearance), appearance.foreground);
}

#[test]
fn all_semantic_colors_are_readable_on_light_dark_and_mid_tone_surfaces() {
    let luminance = |rgb: [u8; 4]| {
        rgb[..3]
            .iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(&c, weight)| {
                let c = f64::from(c) / 255.0;
                weight
                    * if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    for background in [[255; 4], [40, 42, 54, 255], [128, 128, 128, 255]] {
        let appearance = appearance(background, [248, 248, 242, 255]);
        let palette = Palette::new(&appearance);
        for role in [1, 2, 3, 4, 5, 6, 8, 9, 11] {
            let (fg, bg) = (luminance(palette.color(role)), luminance(background));
            assert!((fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5);
        }
        assert_eq!(palette.color(u16::MAX), appearance.foreground);
    }
}

#[test]
fn source_recolor_preserves_roles_ranges_typography_text_and_raw_anchors() {
    let dark = appearance([30, 31, 32, 255], [240; 4]);
    let light = appearance([250; 4], [20, 21, 22, 255]);
    let packet = viewer::TextPage {
        revision: 3,
        source: viewer::SourceRange {
            offset: 9000,
            length: 8,
        },
        text: "if value".into(),
        spans: vec![
            viewer::Span {
                start: 0,
                end: 2,
                role: viewer::STYLE_KEYWORD,
                flags: 1,
            },
            viewer::Span {
                start: 3,
                end: 8,
                role: viewer::STYLE_STRING,
                flags: 2,
            },
        ],
        anchors: vec![
            viewer::Anchor {
                text_offset: 0,
                source_offset: 9000,
            },
            viewer::Anchor {
                text_offset: 8,
                source_offset: 9008,
            },
        ],
        has_before: true,
        has_after: true,
        partial_line: false,
    };
    let mut page = Page::text(packet, &dark);
    let roles = page.roles.clone();
    let old_colors: Vec<_> = page.spans.iter().map(|span| span.color).collect();
    page.recolor(&light);
    assert_eq!(page.roles, roles);
    assert_eq!(page.text, "if value");
    assert_eq!(
        (page.start, page.end, page.anchors[1].source_offset),
        (9000, 9008, 9008)
    );
    assert_eq!(page.styles.len(), 2);
    assert_eq!(page.styles[0].weight, 700);
    assert!(page.styles[1].italic);
    for (span, role) in page.spans.iter().zip(&page.roles) {
        assert_eq!(span.range, role.range);
        assert_eq!(span.color, Palette::new(&light).color(role.role));
    }
    assert_ne!(
        page.spans.iter().map(|span| span.color).collect::<Vec<_>>(),
        old_colors
    );
}

#[test]
fn markdown_heading_and_inline_roles_survive_palette_rebuild_without_font_rewrite() {
    let dark = appearance([30, 31, 32, 255], [240; 4]);
    let mut light = appearance([250; 4], [20, 21, 22, 255]);
    light.accent = [140, 20, 40, 255];
    let block = |kind, text: &str, offset, role| document::Block {
        kind,
        heading_level: 2,
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
            spans: vec![viewer::Span {
                start: 0,
                end: text.len() as u32,
                role,
                flags: 4,
            }],
            anchors: vec![viewer::Anchor {
                text_offset: 0,
                source_offset: offset,
            }],
            has_before: false,
            has_after: false,
            partial_line: false,
        },
    };
    let mut page = Page::document(
        document::DocumentPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: 0,
                length: 40,
            },
            has_before: false,
            has_after: false,
            blocks: vec![
                block(document::BLOCK_HEADING, "Heading", 0, viewer::STYLE_NUMBER),
                block(document::BLOCK_PARAGRAPH, "link", 20, viewer::STYLE_LINK),
            ],
        },
        &dark,
    );
    let roles = page.roles.clone();
    let style_size = page.style_size;
    let sizes: Vec<_> = page.styles.iter().map(|style| style.size).collect();
    page.recolor(&light);
    assert_eq!(page.roles, roles);
    assert_eq!(page.roles[0].role, viewer::STYLE_HEADING);
    assert_eq!(page.roles[1].role, viewer::STYLE_LINK);
    assert_eq!(page.style_size, style_size);
    assert_eq!(
        page.styles
            .iter()
            .map(|style| style.size)
            .collect::<Vec<_>>(),
        sizes
    );
    assert_eq!(
        page.spans[0].color,
        Palette::new(&light).color(viewer::STYLE_HEADING)
    );
    assert_eq!(page.anchors[1].source_offset, 20);
}
