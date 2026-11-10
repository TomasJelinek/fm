//! Short presentation names never change the provider identity used for routing.

/// Prefer the publisher's explicit name and give older manifests a readable
/// fallback. Descriptions are deliberately not treated as list titles.
pub fn viewer_plugin_name(id: &str, name: Option<&str>) -> String {
    let supplied = name.map(str::trim).filter(|name| !name.is_empty());
    let legacy = match id {
        "midnight.viewer.basic" => Some(("Text and binary viewer", "Basic")),
        "midnight.viewer.source" => Some(("Source viewer", "Source")),
        "midnight.viewer.markdown" => Some(("Markdown viewer", "Markdown")),
        "midnight.viewer.image" => Some(("Image viewer", "Images")),
        "midnight.viewer.pdf" => Some(("PDF viewer", "PDF")),
        "midnight.viewer.html" => Some(("HTML viewer", "Chromium HTML")),
        _ => None,
    };
    let title = if let Some((current, old)) = legacy
        && supplied.is_none_or(|value| value == old)
    {
        current.to_owned()
    } else if let Some(name) = supplied {
        name.to_owned()
    } else {
        let words = id.rsplit('.').next().unwrap_or(id).replace(['-', '_'], " ");
        let mut chars = words.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => "Viewer".into(),
        }
    };
    if title
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case("viewer"))
    {
        title
    } else {
        format!("{title} viewer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_titles_and_retained_settings_use_short_viewer_names() {
        for (id, old, expected) in [
            ("midnight.viewer.basic", None, "Text and binary viewer"),
            ("midnight.viewer.source", None, "Source viewer"),
            ("midnight.viewer.image", Some("Images"), "Image viewer"),
            ("midnight.viewer.html", Some("Chromium HTML"), "HTML viewer"),
            ("vendor.data_tools", None, "Data tools viewer"),
        ] {
            assert_eq!(viewer_plugin_name(id, old), expected);
        }
    }

    #[test]
    fn explicit_publisher_titles_keep_their_spelling_and_identify_the_viewer() {
        assert_eq!(
            viewer_plugin_name("vendor.x", Some("Žlutý VIEWER")),
            "Žlutý VIEWER"
        );
        assert_eq!(
            viewer_plugin_name("vendor.x", Some("Custom")),
            "Custom viewer"
        );
        assert_eq!(
            viewer_plugin_name("midnight.viewer.html", Some("Publisher HTML viewer")),
            "Publisher HTML viewer"
        );
    }
}
