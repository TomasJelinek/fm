use crate::{LinuxFileSystem, desktop::read_bounded};
use fm_core::{FsError, IconKey, IconKind, IconPixels, IconService, StorageKind};
use std::{
    collections::{HashSet, VecDeque},
    io::Cursor,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

const ICON_ROOTS: &[&str] = &["/usr/share/icons", "/usr/local/share/icons"];
const PNG_LIMIT: usize = 1024 * 1024;

impl IconService for LinuxFileSystem {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        if !(1..=64).contains(&key.size) {
            return Ok(None);
        }
        let names = icon_names(key);
        let mut themes = VecDeque::new();
        if let Some(theme) = configured_theme(self.home.as_deref()) {
            themes.push_back(theme);
        }
        themes.extend(["Adwaita".into(), "breeze".into(), "hicolor".into()]);
        let mut seen = HashSet::new();
        let mut attempts = 0;
        // A malformed installed inheritance graph cannot recurse indefinitely.
        while let Some(theme) = themes.pop_front() {
            if seen.len() >= 16 || attempts >= 1024 {
                break;
            }
            if !safe_theme_name(&theme) || !seen.insert(theme.clone()) {
                continue;
            }
            for root in ICON_ROOTS {
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
                let directories = theme_directories(&manifest, key.size);
                // Specific MIME names are preferred across all sizes within this theme.
                for name in &names {
                    for directory in &directories {
                        attempts += 1;
                        if attempts > 1024 {
                            return Ok(None);
                        }
                        let path = theme_root.join(directory).join(format!("{name}.png"));
                        if let Some(icon) = load_png(&path, key.size) {
                            return Ok(Some(icon));
                        }
                    }
                }
            }
        }
        for name in names {
            if let Some(icon) = load_png(
                &Path::new("/usr/share/pixmaps").join(format!("{name}.png")),
                key.size,
            ) {
                return Ok(Some(icon));
            }
        }
        Ok(None)
    }
}

fn icon_names(key: &IconKey) -> Vec<&'static str> {
    icon_names_with_storage(key, crate::storage_kind::detect)
}

fn icon_names_with_storage(
    key: &IconKey,
    detect: impl FnOnce(&Path) -> StorageKind,
) -> Vec<&'static str> {
    match key.kind {
        IconKind::Folder => return vec!["folder"],
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
        IconKind::Link => return vec!["emblem-symbolic-link", "text-x-generic"],
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
    use fm_core::IconKind;

    #[test]
    fn discovered_network_volume_uses_the_host_network_icon_before_generic_drive() {
        let key = IconKey::for_drive(Path::new("/mnt/文档"));
        let names = icon_names_with_storage(&key, |root| {
            assert_eq!(root, Path::new("/mnt/文档"));
            fm_core::StorageKind::Network
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
