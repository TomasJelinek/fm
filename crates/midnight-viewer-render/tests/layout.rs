use midnight_viewer_render::TextRenderer;

#[test]
fn unicode_text_keeps_explicit_lines_without_viewport_wrapping() {
    let text = format!(
        "{}\nČeština עברית العربية e\u{301} 👩\u{200d}💻",
        "source code ".repeat(80)
    );
    let page = TextRenderer::default()
        .layout(&text, 18.0, 1.0)
        .expect("valid Unicode text must produce a shared layout");
    assert_eq!(page.text(), text, "layout must never rewrite source text");
    assert_eq!(page.line_count(), 2, "only explicit newlines create rows");
    assert!(
        page.width() > 1_000.0,
        "long lines remain horizontally scrollable"
    );
}
