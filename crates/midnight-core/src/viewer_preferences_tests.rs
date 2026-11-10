use super::*;

#[test]
fn choosing_github_resets_only_markdown_typography_and_later_font_edits_remain_available() {
    let mut prefs = ViewerPreferences::default();
    prefs.markdown_font.family = "monospace".into();
    prefs.markdown_font.size = 29;
    prefs.source_font.size = 21;
    let source = prefs.source_font.clone();
    let text = prefs.text_font.clone();
    // Choosing the already-selected preset must also restore its typography.
    prefs
        .apply(&ViewerPreferenceChange::MarkdownPreset(
            MarkdownPreset::GitHub,
        ))
        .unwrap();
    assert_eq!(prefs.markdown_font.family, "system-ui");
    assert_eq!(prefs.markdown_font.size, 16);
    assert_eq!(prefs.source_font, source);
    assert_eq!(prefs.text_font, text);
    prefs
        .apply(&ViewerPreferenceChange::FontFamily(
            ViewerFontKind::Markdown,
            "serif".into(),
        ))
        .unwrap();
    prefs
        .apply(&ViewerPreferenceChange::FontSize(
            ViewerFontKind::Markdown,
            22,
        ))
        .unwrap();
    assert_eq!(prefs.markdown_font.family, "serif");
    assert_eq!(prefs.markdown_font.size, 22);
    prefs
        .apply(&ViewerPreferenceChange::MarkdownPreset(
            MarkdownPreset::Compact,
        ))
        .unwrap();
    assert_eq!(prefs.markdown_font.family, "serif");
    assert_eq!(prefs.markdown_font.size, 22);
}

#[test]
fn defaults_are_readable_and_changes_from_two_windows_do_not_overwrite_each_other() {
    let mut prefs = ViewerPreferences::default();
    assert!(prefs.line_numbers);
    assert_eq!(prefs.markdown_preset, MarkdownPreset::GitHub);
    prefs
        .apply(&ViewerPreferenceChange::FontSize(
            ViewerFontKind::Source,
            19,
        ))
        .unwrap();
    prefs.apply(&ViewerPreferenceChange::Wrap(true)).unwrap();
    assert!(prefs.wrap);
    assert_eq!(prefs.source_font.size, 19);
    assert_eq!(prefs.markdown_font.size, 16);
    assert_eq!(prefs.comparison_font, "Consolas");
    prefs
        .apply(&ViewerPreferenceChange::ComparisonFont(
            "Cascadia Mono".into(),
        ))
        .unwrap();
    assert_eq!(prefs.comparison_font, "Cascadia Mono");
    assert_eq!(prefs.source_font.size, 19);
}

#[test]
fn invalid_changes_never_replace_accepted_preferences() {
    let original = ViewerPreferences::default();
    for change in [
        ViewerPreferenceChange::FontFamily(ViewerFontKind::Text, "\n".into()),
        ViewerPreferenceChange::ComparisonFont("\n".into()),
        ViewerPreferenceChange::ComparisonFont(String::new()),
        ViewerPreferenceChange::FontFamily(ViewerFontKind::Source, "x".repeat(257)),
        ViewerPreferenceChange::FontSize(ViewerFontKind::Markdown, 49),
        ViewerPreferenceChange::ImageZoom(ViewerZoomPreference::ScaleBasisPoints(0)),
        ViewerPreferenceChange::DocumentZoom(ViewerZoomPreference::ScaleBasisPoints(640_001)),
    ] {
        let mut prefs = original.clone();
        assert!(prefs.apply(&change).is_err());
        assert_eq!(prefs, original);
    }
    for value in [f64::NAN, f64::INFINITY, -1.0, 65.0] {
        assert!(ViewerZoomPreference::from_scale(value).is_err());
    }
    assert_eq!(
        ViewerZoomPreference::from_scale(1.25).unwrap(),
        ViewerZoomPreference::ScaleBasisPoints(12_500)
    );
}
