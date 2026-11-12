use midnight_core::UserPreferences;
use midnight_core::{
    EntryKind, FilenameColorRule, FilenameColorTarget, FilenameStyle, FilterMode, RgbColor,
};
use midnight_platform_common::filename_colors::CompiledFilenameColors;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn version_fourteen_round_trips_ordered_filename_colors() {
    let mut stored: serde_json::Value =
        serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    stored["version"] = 14.into();
    stored["previews"] = serde_json::json!({"enabled": true, "network": false});
    stored["archive_association_overrides"] = serde_json::json!({});
    stored["filename_colors"] = serde_json::json!([{
        "enabled": true, "mode": "masks", "expression": "*.rs;*.txt|secret*",
        "case_sensitive": false, "target": "files", "foreground": "#29a3ef", "background": null
    }]);
    let decoded = decode_preferences(&serde_json::to_vec(&stored).unwrap()).unwrap();
    let roundtrip: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(roundtrip["filename_colors"], stored["filename_colors"]);
    assert_eq!(roundtrip["version"], 14);
}

fn rule(expression: &str, color: RgbColor) -> FilenameColorRule {
    FilenameColorRule {
        expression: expression.into(),
        style: FilenameStyle {
            foreground: Some(color),
            background: None,
        },
        ..Default::default()
    }
}

#[test]
fn ordered_enabled_rules_share_filter_masks_and_case_without_matching_folders_implicitly() {
    let red = RgbColor(230, 10, 30);
    let blue = RgbColor(10, 30, 230);
    let mut first = rule("*.rs;文档*|test*", red);
    let second = rule("*", blue);
    let compiled = CompiledFilenameColors::new(&[first.clone(), second.clone()]).unwrap();
    assert_eq!(
        compiled
            .style("MAIN.RS".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(red)
    );
    assert_eq!(
        compiled
            .style("test.rs".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(blue)
    );
    assert_eq!(
        compiled
            .style("文档.txt".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(red)
    );
    assert!(
        compiled
            .style("dir.rs".as_ref(), EntryKind::Directory)
            .is_none()
    );
    first.enabled = false;
    let compiled = CompiledFilenameColors::new(&[first.clone(), second]).unwrap();
    assert_eq!(
        compiled
            .style("main.rs".as_ref(), EntryKind::File)
            .unwrap()
            .foreground,
        Some(blue)
    );
    first.enabled = true;
    first.case_sensitive = true;
    first.target = FilenameColorTarget::Folders;
    let compiled = CompiledFilenameColors::new(&[first]).unwrap();
    assert!(
        compiled
            .style("MAIN.RS".as_ref(), EntryKind::Directory)
            .is_none()
    );
    assert_eq!(
        compiled
            .style("main.rs".as_ref(), EntryKind::DirectoryLink)
            .unwrap()
            .foreground,
        Some(red)
    );
}

#[test]
fn invalid_disabled_expressions_and_excessive_rules_fail_atomically() {
    let invalid = FilenameColorRule {
        mode: FilterMode::Regex,
        expression: "[".into(),
        enabled: false,
        ..Default::default()
    };
    assert!(CompiledFilenameColors::new(std::slice::from_ref(&invalid)).is_err());
    assert!(
        encode_preferences(UserPreferences {
            filename_colors: vec![invalid].into(),
            ..Default::default()
        })
        .is_err()
    );
    assert!(CompiledFilenameColors::new(&vec![FilenameColorRule::default(); 33]).is_err());
    let valid = UserPreferences {
        filename_colors: vec![rule("*", RgbColor(10, 20, 30))].into(),
        ..Default::default()
    };
    let original: serde_json::Value =
        serde_json::from_slice(&encode_preferences(valid).unwrap()).unwrap();
    for (key, value) in [
        ("foreground", serde_json::json!("#123")),
        ("target", serde_json::json!("unknown")),
        ("mode", serde_json::json!("glob")),
        ("unknown", true.into()),
        ("expression", "x\u{0}".into()),
    ] {
        let mut malformed = original.clone();
        malformed["filename_colors"][0][key] = value;
        assert!(
            decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err(),
            "{key}"
        );
    }
    let mut malformed = original.clone();
    malformed["filename_colors"][0]
        .as_object_mut()
        .unwrap()
        .remove("foreground");
    assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
    malformed = original.clone();
    malformed["filename_colors"][0]
        .as_object_mut()
        .unwrap()
        .remove("background");
    assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
    malformed = original.clone();
    malformed["filename_colors"] = serde_json::Value::Null;
    assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
    malformed = original;
    malformed["version"] = 13.into();
    assert!(decode_preferences(&serde_json::to_vec(&malformed).unwrap()).is_err());
}

#[test]
fn defaults_color_common_file_categories_and_preserve_folder_theme() {
    let preferences =
        decode_preferences(&encode_preferences(UserPreferences::default()).unwrap()).unwrap();
    assert_eq!(preferences.filename_colors.len(), 11);
    let compiled = CompiledFilenameColors::new(&preferences.filename_colors).unwrap();
    let categories = ["main.RS", "app.exe", "photo.jpg", "movie.mp4", "report.pdf"];
    let colors: std::collections::HashSet<_> = categories
        .iter()
        .map(|name| {
            let color = compiled
                .style(name.as_ref(), EntryKind::File)
                .unwrap()
                .foreground
                .unwrap();
            (color.0, color.1, color.2)
        })
        .collect();
    assert_eq!(colors.len(), categories.len());
    for (names, expected) in [
        (
            &["main.RS", "Makefile", "config.json", "README.rs"][..],
            "#3b82f6",
        ),
        (&["movie.MP4", "clip.mkv"][..], "#8b5cf6"),
        (&["report.pdf", "letter.docx", "book.epub"][..], "#c4cfe0"),
        (&["photo.jpg", "vector.svg", "image.avif"][..], "#ec4899"),
        (
            &["app.exe", "installer.msi", "program.AppImage"][..],
            "#f97316",
        ),
        (
            &["table.xlsx", "data.CSV", "table.tsv", "sheet.ods"][..],
            "#22c55e",
        ),
        (&["music.mp3", "sound.flac", "track.ogg"][..], "#14b8a6"),
        (
            &["backup.zip", "archive.7z", "source.tar.gz"][..],
            "#eab308",
        ),
        (&["slides.pptx", "talk.odp"][..], "#ef4444"),
        (
            &[
                "readme.txt",
                "README",
                "README.md",
                "README.release",
                "notes.rst",
            ][..],
            "#c4cfe0",
        ),
        (&["type.ttf", "font.otf", "web.woff2"][..], "#92400e"),
    ] {
        for name in names {
            let style = compiled.style(name.as_ref(), EntryKind::File).unwrap();
            assert_eq!(style.foreground.unwrap().hex(), expected, "{name}");
            assert_eq!(style.background, None, "keep theme state backgrounds");
        }
    }
    assert!(
        compiled
            .style("folder.rs".as_ref(), EntryKind::Directory)
            .is_none()
    );
    assert!(
        compiled
            .style("unclassified.xyz".as_ref(), EntryKind::File)
            .is_none()
    );
}

#[test]
fn deleting_all_presets_survives_restart() {
    let preferences = UserPreferences {
        filename_colors: Vec::new().into(),
        ..Default::default()
    };
    let bytes = encode_preferences(preferences).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["version"], 14);
    let restored = decode_preferences(&bytes).unwrap();
    assert!(restored.filename_colors.is_empty());
    assert!(
        CompiledFilenameColors::new(&[])
            .unwrap()
            .style("file.txt".as_ref(), EntryKind::File)
            .is_none()
    );
}

#[cfg(unix)]
#[path = "support/filename_colors_unix.rs"]
mod native;

#[cfg(windows)]
#[path = "support/filename_colors_windows.rs"]
mod native;
#[path = "support/filename_palette.rs"]
mod palette;
