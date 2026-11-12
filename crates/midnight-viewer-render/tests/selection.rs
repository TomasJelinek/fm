use midnight_viewer_render::{ColorSpan, MAX_TEXT_BYTES, TextRenderer};

#[test]
fn styled_bidi_selection_uses_the_same_layout_as_hit_testing() {
    let text = "Latin العربية עברית e\u{301} 👩\u{200d}💻 end";
    let start = text.find("العربية").unwrap();
    let selected = start..start + "العربية".len();
    let mut renderer = TextRenderer::default();
    let plain = renderer.layout(text, 22.0, 1.0).unwrap();
    let styled = renderer
        .layout_styled(
            text,
            22.0,
            1.0,
            &[ColorSpan {
                range: selected.clone(),
                color: [255, 80, 30, 255],
            }],
        )
        .unwrap();
    assert_eq!(
        plain.width(),
        styled.width(),
        "paint must not change shaping"
    );
    assert_eq!(styled.selected_text(selected.clone()).unwrap(), "العربية");
    let boxes = styled.selection_geometry(selected.clone()).unwrap();
    assert!(!boxes.is_empty(), "selected glyphs have visual geometry");
    assert_eq!(boxes, plain.selection_geometry(selected.clone()).unwrap());
    for rect in boxes {
        let x = ((rect.x0 + rect.x1) / 2.0) as f32;
        let y = ((rect.y0 + rect.y1) / 2.0) as f32;
        let byte = styled.hit_test(x, y).unwrap();
        assert!((selected.start..=selected.end).contains(&byte));
        assert_eq!(plain.hit_test(x, y).unwrap(), byte);
        assert!(text.is_char_boundary(byte));
    }
}

#[test]
fn selection_preserves_combining_marks_emoji_and_both_line_endings() {
    let text = "e\u{301} 👩\u{200d}💻\r\nالعربية\nlast";
    let page = TextRenderer::default().layout(text, 18.0, 1.5).unwrap();
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
    assert!(page.selection_geometry(0..text.len()).unwrap().len() >= 3);
    assert!(page.selected_text(1..2).is_err(), "no UTF-8 splitting");
    assert!(page.selected_text(0..usize::MAX).is_err());
    assert!(page.hit_test(f32::NAN, 0.0).is_err());
}

#[test]
fn layout_checks_input_budgets_before_shaping_or_font_lookup() {
    let mut renderer = TextRenderer::default();
    assert!(
        renderer
            .layout(&"x".repeat(MAX_TEXT_BYTES + 1), 18.0, 1.0)
            .is_err()
    );
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert!(renderer.layout("abc", size, 1.0).is_err());
    }
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert!(renderer.layout("abc", 18.0, scale).is_err());
    }
    for range in [1..2, 0..usize::MAX, std::ops::Range { start: 3, end: 1 }] {
        assert!(
            renderer
                .layout_styled(
                    "文档",
                    18.0,
                    1.0,
                    &[ColorSpan {
                        range,
                        color: [255; 4],
                    }]
                )
                .is_err()
        );
    }
}

#[test]
fn line_budget_counts_crlf_once_and_includes_unicode_line_breaks() {
    let mut renderer = TextRenderer::default();
    for separator in ["\u{2028}", "\u{2029}", "\u{0085}", "\u{000b}", "\u{000c}"] {
        assert!(renderer.layout(&separator.repeat(4096), 18.0, 1.0).is_err());
    }
    let page = renderer.layout(&"\r\n".repeat(4095), 18.0, 1.0).unwrap();
    assert_eq!(page.line_count(), 4096);
}
