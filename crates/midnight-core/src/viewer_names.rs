//! Short presentation names never change the provider identity used for routing.

/// Prefer the publisher's explicit name and give older manifests a readable
/// fallback. Descriptions are deliberately not treated as list titles.
pub fn viewer_plugin_name(id: &str, name: Option<&str>) -> String {
    let supplied = name.map(str::trim).filter(|name| !name.is_empty());
    // A bundled viewer's title: shown when there is no manifest name (retained settings of
    // an uninstalled viewer have none) or the manifest still has its older short name.
    let legacy = match id {
        "midnight.viewer.basic" => Some(("Text and binary viewer", Some("Basic"))),
        "midnight.viewer.source" => Some(("Source viewer", Some("Source"))),
        "midnight.viewer.markdown" => Some(("Markdown viewer", Some("Markdown"))),
        "midnight.viewer.image" => Some(("Image viewer", Some("Images"))),
        "midnight.viewer.pdf" => Some(("PDF viewer", Some("PDF"))),
        // No released HTML viewer manifest had another name.
        "midnight.viewer.html" => Some(("HTML viewer", None)),
        _ => None,
    };
    let title = if let Some((current, old)) = legacy
        && supplied.is_none_or(|value| Some(value) == old)
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
            ("midnight.viewer.html", None, "HTML viewer"),
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
