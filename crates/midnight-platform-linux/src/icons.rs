use crate::{LinuxFileSystem, desktop::read_bounded};
use midnight_core::{
    ActionIcon, Cancellation, FsError, IconKey, IconKind, IconPixels, IconService, LocationKind,
    MAX_ICON_EDGE, StorageKind,
};
use std::{
    collections::{HashSet, VecDeque},
    io::Cursor,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[path = "icons_executable.rs"]
mod executable;
#[path = "icons_theme.rs"]
mod theme;
use theme::{configured_theme, manifest_field, theme_directories};

const PNG_LIMIT: usize = 1024 * 1024;

/// The only places icons are read from: fixed system locations, never a
/// directory a user or a listed folder can choose.
#[derive(Clone, Copy)]
struct Roots<'a> {
    themes: &'a [&'a str],
    pixmaps: &'a str,
}

const SYSTEM: Roots<'static> = Roots {
    themes: &["/usr/share/icons", "/usr/local/share/icons"],
    pixmaps: "/usr/share/pixmaps",
};

impl Roots<'_> {
    /// The resolved `path`, if it is still inside one of these places.
    fn trusted(&self, path: &Path) -> Option<PathBuf> {
        let path = path.canonicalize().ok()?;
        self.themes
            .iter()
            .copied()
            .chain(Some(self.pixmaps))
            .any(|root| path.starts_with(root))
            .then_some(path)
    }

    fn load_png(&self, path: &Path, size: u16) -> Option<IconPixels> {
        // Most probes miss: one `lstat` answers them before the path is resolved.
        path.symlink_metadata().ok()?;
        decode_png(read_bounded(&self.trusted(path)?, PNG_LIMIT)?, size)
    }
}

impl IconService for LinuxFileSystem {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        if !(1..=MAX_ICON_EDGE).contains(&key.size) {
            return Ok(None);
        }
        Ok(load_named_icon(
            self.home.as_deref(),
            &icon_names(key),
            key.size,
            &Cancellation::default(),
        ))
    }
    /// Theme artwork is looked up and scaled for the exact edge it is drawn at.
    fn icon_edge(&self, drawn: u16) -> u16 {
        drawn.clamp(1, MAX_ICON_EDGE)
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

pub(crate) fn theme_directory(home: Option<&Path>) -> Option<PathBuf> {
    let themes = configured_theme(home)
        .into_iter()
        .chain(["Adwaita".into(), "hicolor".into()]);
    for theme in themes {
        for root in SYSTEM.themes {
            if let Some(path) = SYSTEM.trusted(&Path::new(root).join(&theme))
                && path.is_dir()
            {
                return Some(path);
            }
        }
    }
    SYSTEM
        .trusted(Path::new("/usr/share/icons"))
        .filter(|path| path.is_dir())
}
/// Probes for one lookup, across every theme and name. A name that no theme
/// has costs one probe per listed directory: 131 in Yaru, 649 in hicolor.
const MAX_PROBES: usize = 8192;
/// Themes followed from the desktop's theme through `Inherits`.
const MAX_INHERITED: usize = 16;

fn load_named_icon(
    home: Option<&Path>,
    names: &[&str],
    size: u16,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled() {
        return None;
    }
    find_icon(SYSTEM, configured_theme(home), names, size, cancel)
}

/// The first of `names` that `theme` or a theme behind it has as a PNG, at
/// `size` device pixels.
fn find_icon(
    roots: Roots<'_>,
    theme: Option<String>,
    names: &[&str],
    size: u16,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled()
        || !(1..=MAX_ICON_EDGE).contains(&size)
        || names.iter().any(|name| !safe_theme_name(name))
    {
        return None;
    }
    // The desktop's theme, then the themes it inherits, depth first in their
    // listed order, so a variant such as Yaru-blue reaches Yaru before any
    // built-in fallback. hicolor is the last resort of every theme.
    let mut themes: VecDeque<String> = theme.into_iter().collect();
    let mut fallbacks = ["Adwaita", "breeze", "hicolor"].into_iter();
    let mut seen = HashSet::new();
    let (mut followed, mut probes) = (0, 0);
    'themes: loop {
        let theme = match themes.pop_front() {
            Some(theme) if followed < MAX_INHERITED => {
                followed += 1;
                theme
            }
            // A malformed installed inheritance graph cannot recurse
            // indefinitely, and it does not use up the fallbacks either.
            Some(_) => {
                themes.clear();
                continue;
            }
            None => match fallbacks.next() {
                Some(theme) => theme.to_owned(),
                None => break,
            },
        };
        if cancel.is_cancelled() {
            break;
        }
        if !safe_theme_name(&theme) || !seen.insert(theme.clone()) {
            continue;
        }
        for root in roots.themes {
            if cancel.is_cancelled() {
                return None;
            }
            let theme_root = Path::new(root).join(&theme);
            let manifest = roots
                .trusted(&theme_root.join("index.theme"))
                .and_then(|path| read_bounded(&path, 256 * 1024))
                .and_then(|bytes| String::from_utf8(bytes).ok());
            let Some(manifest) = manifest else {
                continue;
            };
            let inherited: Vec<&str> = manifest_field(&manifest, "Inherits")
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .take(16)
                .filter(|parent| {
                    safe_theme_name(parent) && *parent != "hicolor" && !seen.contains(*parent)
                })
                .collect();
            for parent in inherited.into_iter().rev() {
                themes.push_front(parent.into());
            }
            let directories = theme_directories(&manifest, size);
            // Specific MIME names are preferred across all sizes within this theme.
            for name in names {
                for directory in &directories {
                    if cancel.is_cancelled() {
                        return None;
                    }
                    probes += 1;
                    if probes > MAX_PROBES {
                        break 'themes;
                    }
                    let path = theme_root.join(directory).join(format!("{name}.png"));
                    if let Some(mut icon) = roots.load_png(&path, size) {
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
        if let Some(mut icon) =
            roots.load_png(&Path::new(roots.pixmaps).join(format!("{name}.png")), size)
        {
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
        ActionIcon::RecycleBin => &["user-trash"],
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

/// Decodes a bounded PNG to a `size` by `size` icon. Artwork that already has
/// that size is returned pixel for pixel.
pub(crate) fn decode_png(bytes: Vec<u8>, size: u16) -> Option<IconPixels> {
    if bytes.len() > PNG_LIMIT || !(1..=MAX_ICON_EDGE).contains(&size) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    reader.limits(limits);
    let source = reader.decode().ok()?.into_rgba8();
    let edge = u32::from(size);
    let pixels = if source.dimensions() == (edge, edge) {
        source
    } else {
        resize(&source, edge)
    };
    IconPixels::new(edge, edge, pixels.into_raw()).ok()
}

/// Scales in premultiplied alpha, so the hidden color of transparent pixels
/// neither darkens nor tints the edges of the artwork.
fn resize(source: &image::RgbaImage, edge: u32) -> image::RgbaImage {
    let mut premultiplied = image::Rgba32FImage::new(source.width(), source.height());
    for (from, to) in source.pixels().zip(premultiplied.pixels_mut()) {
        let alpha = f32::from(from[3]) / 255.0;
        let channel = |index: usize| f32::from(from[index]) / 255.0 * alpha;
        *to = image::Rgba([channel(0), channel(1), channel(2), alpha]);
    }
    // Lanczos keeps reduced artwork sharp; it would ring around enlarged pixels.
    let filter = if edge < source.width().max(source.height()) {
        image::imageops::FilterType::Lanczos3
    } else {
        image::imageops::FilterType::CatmullRom
    };
    let scaled = image::imageops::resize(&premultiplied, edge, edge, filter);
    let mut result = image::RgbaImage::new(edge, edge);
    for (from, to) in scaled.pixels().zip(result.pixels_mut()) {
        let alpha = from[3].clamp(0.0, 1.0);
        let opacity = (alpha * 255.0).round() as u8;
        if opacity == 0 {
            continue;
        }
        let channel = |index: usize| ((from[index] / alpha).clamp(0.0, 1.0) * 255.0).round() as u8;
        *to = image::Rgba([channel(0), channel(1), channel(2), opacity]);
    }
    result
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
        assert!(decode_png(png(1), 256).is_some());
        assert!(decode_png(png(1), 257).is_none());
        assert!(decode_png(png(1), 0).is_none());
        assert!(decode_png(vec![0; 1024 * 1024 + 1], 32).is_none());
    }

    /// A theme tree under a temporary directory: `(theme, Inherits, [(directory,
    /// Size, Scale, icon names)])`. Each icon is a solid square of its
    /// directory's pixel size whose red channel is the index of its theme and
    /// whose green channel is the index of its directory.
    struct Themes {
        root: tempfile::TempDir,
        icons: String,
        pixmaps: String,
    }

    /// Name, `Size`, `Scale` and the icons it holds.
    type Directory = (&'static str, u32, u32, Vec<&'static str>);

    impl Themes {
        fn new(themes: Vec<(&str, &str, Vec<Directory>)>) -> Self {
            let root = tempfile::tempdir().unwrap();
            let base = root.path().canonicalize().unwrap();
            let (icons, pixmaps) = (base.join("icons"), base.join("pixmaps"));
            std::fs::create_dir_all(&pixmaps).unwrap();
            for (index, (theme, inherits, directories)) in themes.iter().enumerate() {
                let listed: Vec<_> = directories.iter().map(|directory| directory.0).collect();
                let mut manifest = format!(
                    "[Icon Theme]\nName={theme}\nInherits={inherits}\nDirectories={}\n",
                    listed.join(",")
                );
                for (position, (directory, size, scale, names)) in directories.iter().enumerate() {
                    manifest += &format!("[{directory}]\nSize={size}\nScale={scale}\n");
                    let path = icons.join(theme).join(directory);
                    std::fs::create_dir_all(&path).unwrap();
                    for name in names {
                        let edge = size * scale;
                        let pixels = image::RgbaImage::from_pixel(
                            edge,
                            edge,
                            image::Rgba([index as u8, position as u8, 0, 255]),
                        );
                        std::fs::write(path.join(format!("{name}.png")), encoded(&pixels)).unwrap();
                    }
                }
                std::fs::create_dir_all(icons.join(theme)).unwrap();
                std::fs::write(icons.join(theme).join("index.theme"), manifest).unwrap();
            }
            Self {
                root,
                icons: icons.to_str().unwrap().into(),
                pixmaps: pixmaps.to_str().unwrap().into(),
            }
        }

        /// The indexes of the theme and of the directory that served the first
        /// of `names`, and the edge of the delivered icon.
        fn find(&self, theme: Option<&str>, names: &[&str], size: u16) -> Option<(u8, u8, u32)> {
            let roots = Roots {
                themes: &[&self.icons],
                pixmaps: &self.pixmaps,
            };
            let icon = find_icon(
                roots,
                theme.map(String::from),
                names,
                size,
                &Cancellation::default(),
            )?;
            assert_eq!(icon.width, icon.height);
            Some((icon.rgba[0], icon.rgba[1], icon.width))
        }
    }

    #[test]
    fn a_variant_theme_reaches_its_parents_depth_first_before_any_fallback() {
        let themes = Themes::new(vec![
            // hicolor comes last however early a theme names it.
            (
                "Accent",
                "hicolor,Base,Side",
                vec![("24x24/places", 24, 1, vec![])],
            ),
            (
                "Base",
                "Deep",
                vec![
                    ("16x16/places", 16, 1, vec!["folder"]),
                    ("24x24/places", 24, 1, vec!["folder"]),
                    ("48x48@2x/places", 48, 2, vec!["folder"]),
                ],
            ),
            (
                "Deep",
                "",
                vec![("32x32/places", 32, 1, vec!["folder", "deep-or-side"])],
            ),
            (
                "Side",
                "",
                vec![("32x32/places", 32, 1, vec!["deep-or-side", "side"])],
            ),
            (
                "Adwaita",
                "hicolor",
                vec![("16x16/places", 16, 1, vec!["folder", "fallback-or-last"])],
            ),
            (
                "hicolor",
                "",
                vec![(
                    "32x32/apps",
                    32,
                    1,
                    vec!["folder", "side", "fallback-or-last", "last"],
                )],
            ),
        ]);
        let find = |names: &[&str], size| themes.find(Some("Accent"), names, size);
        // Base (theme 1) serves every size at exactly the requested edge, from
        // its exact directory, else the nearest larger one, else the largest.
        for (size, directory) in [
            (16, 0),
            (17, 1),
            (20, 1),
            (24, 1),
            (32, 2),
            (96, 2),
            (200, 2),
        ] {
            assert_eq!(
                find(&["folder"], size),
                Some((1, directory, u32::from(size))),
                "{size}"
            );
        }
        // Depth first: Base's parent precedes Accent's second parent.
        assert_eq!(find(&["deep-or-side"], 20), Some((2, 0, 20)));
        assert_eq!(find(&["side"], 20), Some((3, 0, 20)));
        // Then the built-in fallbacks, with hicolor after them.
        assert_eq!(find(&["fallback-or-last"], 20), Some((4, 0, 20)));
        assert_eq!(find(&["last"], 20), Some((5, 0, 20)));
        // A specific name in a later theme does not beat a general one in an earlier.
        assert_eq!(find(&["last", "folder"], 20), Some((1, 1, 20)));
        assert_eq!(find(&["missing"], 20), None);
        // Without a desktop theme the fallbacks are searched as before.
        assert_eq!(themes.find(None, &["folder"], 16), Some((4, 0, 16)));
        assert_eq!(
            themes.find(Some("NotInstalled"), &["folder"], 16),
            Some((4, 0, 16))
        );
    }

    #[test]
    fn long_or_circular_inheritance_ends_and_still_reaches_the_fallbacks() {
        // Twenty themes in a chain, the last one inheriting the first again.
        let names: Vec<String> = (0..20).map(|index| format!("Chain{index}")).collect();
        let mut tree: Vec<(&str, &str, Vec<Directory>)> = (0..20)
            .map(|index| {
                let icons = match index {
                    15 => vec!["within"],
                    19 => vec!["beyond", "beyond-or-fallback"],
                    _ => vec![],
                };
                (
                    names[index].as_str(),
                    names[(index + 1) % 20].as_str(),
                    vec![("16x16/apps", 16, 1, icons)],
                )
            })
            .collect();
        tree.push((
            "Adwaita",
            "",
            vec![("16x16/apps", 16, 1, vec!["beyond-or-fallback"])],
        ));
        let themes = Themes::new(tree);
        let find = |name| themes.find(Some("Chain0"), &[name], 16);
        assert_eq!(find("within"), Some((15, 0, 16)), "the sixteenth theme");
        assert_eq!(find("beyond"), None, "the twentieth is not followed");
        assert_eq!(find("beyond-or-fallback"), Some((20, 0, 16)));
    }

    #[test]
    fn an_exhausted_probe_budget_still_searches_pixmaps() {
        let themes = Themes::new(vec![
            ("Huge", "", vec![]),
            ("Adwaita", "", vec![("16x16/apps", 16, 1, vec!["name8"])]),
        ]);
        // A manifest that lists far more directories than exist on disk.
        let listed: Vec<String> = (0..1024)
            .map(|index| format!("{index}x{index}/apps"))
            .collect();
        std::fs::write(
            Path::new(&themes.icons).join("Huge/index.theme"),
            format!("[Icon Theme]\nDirectories={}\n", listed.join(",")),
        )
        .unwrap();
        std::fs::write(
            Path::new(&themes.pixmaps).join("name8.png"),
            encoded(&image::RgbaImage::from_pixel(
                16,
                16,
                image::Rgba([9, 9, 0, 255]),
            )),
        )
        .unwrap();
        let names: Vec<String> = (0..9).map(|index| format!("name{index}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        // Nine names in 1024 directories use up the budget inside Huge, so
        // Adwaita is never reached; the legacy directory still is.
        assert_eq!(themes.find(Some("Huge"), &names, 16), Some((9, 9, 16)));
        assert_eq!(themes.find(Some("Huge"), &names[8..], 16), Some((1, 0, 16)));
    }

    #[test]
    fn lookups_stay_inside_the_icon_roots_and_fall_back_to_pixmaps() {
        let themes = Themes::new(vec![(
            "Base",
            "",
            vec![("16x16/apps", 16, 1, vec!["inside"])],
        )]);
        let outside = themes.root.path().join("outside.png");
        std::fs::write(
            &outside,
            encoded(&image::RgbaImage::from_pixel(
                16,
                16,
                image::Rgba([9, 0, 0, 255]),
            )),
        )
        .unwrap();
        let apps = Path::new(&themes.icons).join("Base/16x16/apps");
        std::os::unix::fs::symlink(&outside, apps.join("escaped.png")).unwrap();
        std::os::unix::fs::symlink(apps.join("inside.png"), apps.join("alias.png")).unwrap();
        std::fs::copy(&outside, Path::new(&themes.pixmaps).join("legacy.png")).unwrap();
        let find = |name| themes.find(Some("Base"), &[name], 16);
        assert_eq!(find("inside"), Some((0, 0, 16)));
        assert_eq!(
            find("alias"),
            Some((0, 0, 16)),
            "a link inside the roots is followed"
        );
        assert_eq!(find("escaped"), None, "a link out of the roots is refused");
        assert_eq!(find("legacy"), Some((9, 0, 16)), "pixmaps is searched last");
        assert_eq!(find("../outside"), None);
    }

    fn encoded(image: &image::RgbaImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn artwork_of_the_requested_size_is_returned_pixel_for_pixel() {
        let source = image::RgbaImage::from_fn(24, 24, |x, y| {
            image::Rgba([
                (x * 10) as u8,
                (y * 10) as u8,
                (x ^ y) as u8,
                (x * y % 256) as u8,
            ])
        });
        let icon = decode_png(encoded(&source), 24).unwrap();
        assert_eq!((icon.width, icon.height), (24, 24));
        assert_eq!(&icon.rgba[..], source.as_raw().as_slice());
    }

    #[test]
    fn scaling_ignores_the_color_hidden_in_transparent_pixels() {
        // An opaque red left half beside a transparent right half whose hidden
        // color is green. Straight-alpha filtering would tint the seam green.
        let source = image::RgbaImage::from_fn(24, 24, |x, _| {
            if x < 12 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 255, 0, 0])
            }
        });
        for edge in [20u16, 36] {
            let icon = decode_png(encoded(&source), edge).unwrap();
            assert_eq!(icon.rgba.len(), usize::from(edge) * usize::from(edge) * 4);
            let mut partial = 0;
            for pixel in icon.rgba.as_chunks::<4>().0 {
                if pixel[3] == 0 {
                    assert_eq!(pixel[..3], [0, 0, 0], "transparent pixels carry no color");
                    continue;
                }
                assert!(pixel[0] >= 250 && pixel[1] <= 5, "edge {edge}: {pixel:?}");
                partial += usize::from(pixel[3] < 255);
            }
            assert!(partial > 0, "the seam is antialiased at edge {edge}");
        }
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

    #[test]
    fn recycle_bin_action_uses_the_host_trash_theme_name_without_path_lookup() {
        let key = IconKey::action(ActionIcon::RecycleBin);
        let names =
            icon_names_with_storage(&key, |_| panic!("system action must not inspect a path"));
        assert_eq!(names, ["user-trash"]);
    }
}
