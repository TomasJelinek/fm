use crate::{LinuxFileSystem, desktop::read_bounded};
use midnight_core::{
    ActionIcon, Cancellation, FsError, IconKey, IconKind, IconPixels, IconService, LocationKind,
    StorageKind,
};
use std::{
    collections::{HashSet, VecDeque},
    io::Cursor,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

#[path = "icons_executable.rs"]
mod executable;

const ICON_ROOTS: &[&str] = &["/usr/share/icons", "/usr/local/share/icons"];
const PNG_LIMIT: usize = 1024 * 1024;

impl IconService for LinuxFileSystem {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        if !(1..=64).contains(&key.size) {
            return Ok(None);
        }
        Ok(load_named_icon(
            self.home.as_deref(),
            &icon_names(key),
            key.size,
            &Cancellation::default(),
        ))
    }
    fn load_executable_icon(
        &self,
        path: &Path,
        size: u16,
        cancel: &Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        Ok(executable::load(self.home.as_deref(), path, size, cancel))
    }
}

fn load_named_icon(
    home: Option<&Path>,
    names: &[&str],
    size: u16,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled()
        || !(1..=64).contains(&size)
        || names.iter().any(|name| !safe_theme_name(name))
    {
        return None;
    }
    let mut themes = VecDeque::new();
    if let Some(theme) = configured_theme(home) {
        themes.push_back(theme);
    }
    themes.extend(["Adwaita".into(), "breeze".into(), "hicolor".into()]);
    let mut seen = HashSet::new();
    let mut attempts = 0;
    // A malformed installed inheritance graph cannot recurse indefinitely.
    while let Some(theme) = themes.pop_front() {
        if cancel.is_cancelled() || seen.len() >= 16 || attempts >= 1024 {
            break;
        }
        if !safe_theme_name(&theme) || !seen.insert(theme.clone()) {
            continue;
        }
        for root in ICON_ROOTS {
            if cancel.is_cancelled() {
                return None;
            }
            let theme_root = Path::new(root).join(&theme);
            let manifest = trusted_path(&theme_root.join("index.theme"))
                .and_then(|path| read_bounded(&path, 256 * 1024))
                .and_then(|bytes| String::from_utf8(bytes).ok());
            let Some(manifest) = manifest else {
                continue;
            };
            for inherited in manifest_field(&manifest, "Inherits")
                .unwrap_or("")
                .split(',')
                .take(16)
            {
                if safe_theme_name(inherited) && !seen.contains(inherited) {
                    themes.push_back(inherited.into());
                }
            }
            let directories = theme_directories(&manifest, size);
            // Specific MIME names are preferred across all sizes within this theme.
            for name in names {
                for directory in &directories {
                    attempts += 1;
                    if cancel.is_cancelled() || attempts > 1024 {
                        return None;
                    }
                    let path = theme_root.join(directory).join(format!("{name}.png"));
                    if let Some(mut icon) = load_png(&path, size) {
                        icon.generic = *name == "text-x-generic";
                        return Some(icon);
                    }
                }
            }
        }
    }
    for name in names {
        if cancel.is_cancelled() {
            return None;
        }
        if let Some(mut icon) = load_png(
            &Path::new("/usr/share/pixmaps").join(format!("{name}.png")),
            size,
        ) {
            icon.generic = *name == "text-x-generic";
            return Some(icon);
        }
    }
    None
}

fn icon_names(key: &IconKey) -> Vec<&'static str> {
    icon_names_with_storage(key, crate::storage_kind::detect)
}

fn icon_names_with_storage(
    key: &IconKey,
    detect: impl FnOnce(&Path) -> StorageKind,
) -> Vec<&'static str> {
    match key.kind {
        IconKind::Action(role) => return action_names(role).to_vec(),
        IconKind::Location(kind) => {
            return match kind {
                LocationKind::Home => vec!["user-home", "folder"],
                LocationKind::Desktop => vec!["user-desktop", "folder"],
                LocationKind::Documents => vec!["folder-documents", "folder"],
                LocationKind::Downloads => vec!["folder-download", "folder"],
                LocationKind::Pictures => vec!["folder-pictures", "folder"],
                LocationKind::Videos => vec!["folder-videos", "folder"],
                LocationKind::Music => vec!["folder-music", "folder"],
                LocationKind::Temporary => vec!["folder-temp", "folder"],
                LocationKind::OneDrive => vec!["folder-remote", "folder"],
                LocationKind::Volume => vec!["drive-harddisk"],
            };
        }
        IconKind::Folder => return vec!["folder"],
        IconKind::FolderLink => {
            return vec![
                "folder-symbolic-link",
                "folder-link",
                "emblem-symbolic-link",
            ];
        }
        IconKind::Drive => {
            // detect checks locally parsed mountinfo for network filesystems
            // before any target stat/canonicalize; no media/file contents are read.
            let kind = key
                .drive_root
                .as_deref()
                .filter(|root| root.is_absolute() && !root.as_os_str().as_bytes().contains(&0))
                .map(detect);
            return if kind == Some(StorageKind::Network) {
                vec!["network-server", "drive-harddisk"]
            } else {
                vec!["drive-harddisk"]
            };
        }
        IconKind::Link => return vec!["emblem-symbolic-link"],
        IconKind::File => {}
    }
    // Fixed MIME names prevent filename-controlled path construction or file sniffing.
    let extension = key
        .extension
        .as_ref()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.len() <= 128)
        .map(str::to_ascii_lowercase);
    let mut names = match extension.as_deref() {
        Some("png") => vec!["image-png", "image-x-generic"],
        Some("jpg" | "jpeg") => vec!["image-jpeg", "image-x-generic"],
        Some("gif" | "bmp" | "webp" | "svg" | "tif" | "tiff") => vec!["image-x-generic"],
        Some("pdf") => vec!["application-pdf", "x-office-document"],
        Some("zip" | "7z" | "gz" | "xz" | "bz2" | "tar" | "rar") => vec!["package-x-generic"],
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a") => vec!["audio-x-generic"],
        Some("mp4" | "mkv" | "webm" | "avi" | "mov") => vec!["video-x-generic"],
        Some("rs" | "c" | "cpp" | "h" | "py" | "js" | "ts" | "json" | "toml" | "xml" | "sh") => {
            vec!["text-x-script"]
        }
        Some("doc" | "docx" | "odt" | "rtf") => vec!["x-office-document"],
        Some("xls" | "xlsx" | "ods" | "csv") => vec!["x-office-spreadsheet"],
        Some("ppt" | "pptx" | "odp") => vec!["x-office-presentation"],
        _ => vec![],
    };
    names.push("text-x-generic");
    names
}

fn action_names(role: ActionIcon) -> &'static [&'static str] {
    match role {
        ActionIcon::Back => &["go-previous"],
        ActionIcon::Forward => &["go-next"],
        ActionIcon::Parent => &["go-up"],
        ActionIcon::Refresh => &["view-refresh"],
        ActionIcon::OpenPath => &["go-jump", "document-open"],
        ActionIcon::Bookmarks => &["user-bookmarks"],
        ActionIcon::Terminal => &["utilities-terminal"],
        ActionIcon::Options => &["preferences-system"],
        ActionIcon::Run => &["system-run"],
        ActionIcon::TabAdd => &["tab-new"],
        ActionIcon::CloseTab | ActionIcon::Close => &["window-close"],
        ActionIcon::RemoveBookmark => &["bookmark-remove"],
        ActionIcon::AddBookmark => &["bookmark-new"],
        ActionIcon::Help => &["help-browser"],
        ActionIcon::Rename => &["edit-rename"],
        ActionIcon::View => &["document-preview"],
        ActionIcon::Find => &["system-search", "edit-find"],
        ActionIcon::Edit => &["document-edit", "accessories-text-editor"],
        ActionIcon::Copy => &["edit-copy"],
        ActionIcon::Move => &["folder-move"],
        ActionIcon::NewFolder => &["folder-new"],
        ActionIcon::NewFile => &["document-new"],
        ActionIcon::Delete => &["edit-delete"],
        ActionIcon::UserMenu => &["applications-other"],
        ActionIcon::Menu => &["open-menu"],
        ActionIcon::SaveAgain => &["document-save"],
        ActionIcon::Done => &["dialog-ok"],
        ActionIcon::Properties => &["document-properties"],
        ActionIcon::FolderSizes => &["folder-properties"],
        ActionIcon::About => &["help-about"],
        ActionIcon::Exit => &["application-exit"],
        ActionIcon::SortAscending => &["view-sort-ascending"],
        ActionIcon::SortDescending => &["view-sort-descending"],
        ActionIcon::Network => &["network-server", "network-workgroup", "drive-harddisk"],
    }
}

fn safe_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        && name != "."
        && name != ".."
}

fn configured_theme(home: Option<&Path>) -> Option<String> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))?;
    let input =
        String::from_utf8(read_bounded(&root.join("gtk-3.0/settings.ini"), 64 * 1024)?).ok()?;
    input
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .find_map(|(key, value)| {
            let value = value.trim().trim_matches('"');
            (key.trim() == "gtk-icon-theme-name" && safe_theme_name(value)).then(|| value.into())
        })
}

fn manifest_field<'a>(input: &'a str, field: &str) -> Option<&'a str> {
    let mut in_theme = false;
    for line in input.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_theme = line == "[Icon Theme]";
            continue;
        }
        if in_theme
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == field
        {
            return Some(value.trim());
        }
    }
    None
}

fn theme_directories(input: &str, size: u16) -> Vec<String> {
    let mut directories: Vec<_> = ["Directories", "ScaledDirectories"]
        .into_iter()
        .filter_map(|key| manifest_field(input, key))
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 256
                && Path::new(name)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
        })
        .take(128)
        .map(String::from)
        .collect();
    // Prefer exact nominal raster size. PNG-only lookup deliberately permits graceful
    // fallback for scalable/SVG-only themes rather than loading external SVG resources.
    let exact = format!("{size}x{size}/");
    directories.sort_by_key(|directory| !directory.starts_with(&exact));
    directories.dedup();
    directories
}

fn trusted_path(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    ICON_ROOTS
        .iter()
        .copied()
        .chain(Some("/usr/share/pixmaps"))
        .any(|root| path.starts_with(root))
        .then_some(path)
}

fn load_png(path: &Path, size: u16) -> Option<IconPixels> {
    decode_png(read_bounded(&trusted_path(path)?, PNG_LIMIT)?, size)
}

fn decode_png(bytes: Vec<u8>, size: u16) -> Option<IconPixels> {
    if bytes.len() > PNG_LIMIT || !(1..=64).contains(&size) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    reader.limits(limits);
    let source = reader.decode().ok()?.into_rgba8();
    let pixels = image::imageops::resize(
        &source,
        u32::from(size),
        u32::from(size),
        image::imageops::FilterType::Triangle,
    );
    IconPixels::new(u32::from(size), u32::from(size), pixels.into_raw()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::IconKind;

    #[test]
    fn folder_links_request_only_fixed_host_link_art_without_probing_the_target() {
        let key = IconKey {
            extension: Some("../../evil.png".into()),
            drive_root: Some("/mnt/untrusted".into()),
            ..IconKey::folder_link()
        };
        let names = icon_names_with_storage(&key, |_| panic!("folder link target was probed"));
        assert_eq!(names[0], "folder-symbolic-link");
        assert!(names.contains(&"emblem-symbolic-link"));
        assert!(
            !names.contains(&"folder"),
            "generic folder art would erase the link indication"
        );
        assert!(names.iter().all(|name| safe_theme_name(name)));
    }

    #[test]
    fn semantic_names_are_fixed_host_roles_and_never_probe_supplied_paths() {
        for (kind, expected) in [
            (
                IconKind::Action(midnight_core::ActionIcon::Back),
                "go-previous",
            ),
            (
                IconKind::Action(midnight_core::ActionIcon::Network),
                "network-server",
            ),
            (
                IconKind::Action(midnight_core::ActionIcon::SortDescending),
                "view-sort-descending",
            ),
            (
                IconKind::Location(midnight_core::LocationKind::Home),
                "user-home",
            ),
            (
                IconKind::Location(midnight_core::LocationKind::Downloads),
                "folder-download",
            ),
        ] {
            let key = IconKey {
                kind,
                extension: Some("../../arbitrary.png".into()),
                drive_root: Some("/mnt/never-probe".into()),
                size: 32,
            };
            let names = icon_names_with_storage(&key, |_| panic!("semantic role probed a path"));
            assert_eq!(names[0], expected);
            assert!(names.iter().all(|name| safe_theme_name(name)));
        }
        for role in midnight_core::ActionIcon::ALL {
            let names = icon_names(&IconKey::action(role));
            assert!(!names.is_empty(), "missing fixed names for {role:?}");
            assert!(names.iter().all(|name| safe_theme_name(name)));
        }
    }

    #[test]
    fn discovered_network_volume_uses_the_host_network_icon_before_generic_drive() {
        let key = IconKey::for_drive(Path::new("/mnt/文档"));
        let names = icon_names_with_storage(&key, |root| {
            assert_eq!(root, Path::new("/mnt/文档"));
            midnight_core::StorageKind::Network
        });
        assert_eq!(names, ["network-server", "drive-harddisk"]);
    }

    #[test]
    fn type_only_and_invalid_volume_keys_do_not_probe_the_target() {
        for key in [
            IconKey::drive(),
            IconKey::for_drive(Path::new("relative")),
            IconKey::for_drive(Path::new("/mnt/invalid\0root")),
        ] {
            assert_eq!(
                icon_names_with_storage(&key, |_| panic!("invalid volume was probed")),
                ["drive-harddisk"]
            );
        }
        assert_eq!(
            icon_names_with_storage(&IconKey::folder(), |_| panic!("folder was probed")),
            ["folder"]
        );
    }

    #[test]
    fn theme_directories_are_bounded_relative_names_only() {
        let input = "[Icon Theme]\nDirectories=../evil,/tmp/evil,16x16/mimetypes,32x32/places\n[Other]\nDirectories=outside\n";
        assert_eq!(
            theme_directories(input, 32),
            ["32x32/places", "16x16/mimetypes"]
        );
    }

    #[test]
    fn png_decode_rejects_oversized_input_and_dimensions_and_resizes_bounded_output() {
        use std::io::Cursor;
        fn png(width: u32) -> Vec<u8> {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(width, 1)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            bytes.into_inner()
        }
        let result = decode_png(png(1), 24).unwrap();
        assert_eq!(
            (result.width, result.height, result.rgba.len()),
            (24, 24, 24 * 24 * 4)
        );
        assert!(decode_png(png(513), 32).is_none());
        assert!(decode_png(png(1), 65).is_none());
        assert!(decode_png(vec![0; 1024 * 1024 + 1], 32).is_none());
    }

    #[test]
    fn mime_icon_names_are_allowlisted_not_interpolated_from_extensions() {
        let key = IconKey {
            kind: IconKind::File,
            extension: Some("PNG".into()),
            drive_root: None,
            size: 32,
        };
        assert_eq!(
            icon_names(&key),
            ["image-png", "image-x-generic", "text-x-generic"]
        );
        let unsafe_key = IconKey {
            extension: Some("../../evil".into()),
            ..key
        };
        assert_eq!(icon_names(&unsafe_key), ["text-x-generic"]);
        assert_eq!(icon_names(&IconKey::folder()), ["folder"]);
    }
}
