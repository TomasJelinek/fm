use crate::{WindowsFileSystem, desktop::ComGuard, volume_details::ErrorMode};
use fm_core::{ActionIcon, FsError, IconKey, IconKind, IconPixels, IconService, LocationKind};
use std::{
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
    },
    Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, GetDriveTypeW, GetLogicalDrives,
    },
    System::{
        Com::COINIT_MULTITHREADED,
        WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
        },
    },
    UI::{
        Shell::{
            Common::ITEMIDLIST, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads,
            FOLDERID_Profile, ILFree, KF_FLAG_DONT_VERIFY, SHFILEINFOW, SHGFI_ICON, SHGFI_PIDL,
            SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGSI_ICON, SHGSI_LINKOVERLAY,
            SHGSI_SMALLICON, SHGetFileInfoW, SHGetKnownFolderIDList, SHGetStockIconInfo,
            SHSTOCKICONINFO, SIID_DELETE, SIID_DOCNOASSOC, SIID_DRIVECD, SIID_DRIVEFIXED,
            SIID_DRIVENET, SIID_DRIVERAM, SIID_DRIVEREMOVE, SIID_DRIVEUNKNOWN, SIID_FOLDER,
            SIID_FOLDEROPEN, SIID_HELP, SIID_INFO, SIID_RENAME, SIID_SETTINGS,
        },
        WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
    },
};

impl IconService for WindowsFileSystem {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
        if !(1..=64).contains(&key.size) {
            return Ok(None);
        }
        let Some(_com) = ComGuard::initialize(COINIT_MULTITHREADED as u32) else {
            return Ok(None);
        };
        let Some(icon) = native_icon(key) else {
            return Ok(None);
        };
        let Some(mut surface) = Dib::new(key.size) else {
            return Ok(None);
        };
        let Some(black) = surface.draw(icon.0, 0) else {
            return Ok(None);
        };
        let Some(white) = surface.draw(icon.0, 255) else {
            return Ok(None);
        };
        let rgba = straight_rgba(&black, &white);
        // Some host versions return a transparent placeholder for a documented
        // stock id. Report it as unavailable so the semantic UI fallback remains.
        let pixels = rgba.as_chunks::<4>().0;
        if !pixels.iter().any(|pixel| pixel[3] != 0) || opaque_black_placeholder(key.kind, pixels) {
            return Ok(None);
        }
        Ok(IconPixels::new(u32::from(key.size), u32::from(key.size), rgba).ok())
    }
}

fn opaque_black_placeholder(kind: IconKind, pixels: &[[u8; 4]]) -> bool {
    // The host can also return a completely opaque black Settings placeholder.
    // Reject only that exact semantic-art artifact; real black glyphs retain
    // their transparent surroundings, and ordinary file/drive art stays untouched.
    matches!(kind, IconKind::Action(_) | IconKind::Location(_))
        && !pixels.is_empty()
        && pixels.iter().all(|pixel| *pixel == [0, 0, 0, 255])
}

fn synthetic_name(key: &IconKey) -> Vec<u16> {
    let mut name: Vec<u16> = "type".encode_utf16().collect();
    if let Some(extension) = &key.extension {
        let extension: Vec<_> = extension.encode_wide().take(65).collect();
        if !extension.is_empty()
            && extension.len() <= 64
            && !extension
                .iter()
                .any(|ch| [0, b'/' as u16, b'\\' as u16, b':' as u16, b'.' as u16].contains(ch))
        {
            name.push(b'.' as u16);
            name.extend(extension);
        }
    }
    name.push(0);
    name
}

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: Shell icon functions return an owned icon, destroyed exactly once.
        unsafe { DestroyIcon(self.0) };
    }
}

fn native_icon(key: &IconKey) -> Option<OwnedIcon> {
    match key.kind {
        IconKind::Action(role) => return action_stock_id(role).and_then(stock_icon),
        IconKind::Location(LocationKind::Volume) => return stock_icon(SIID_DRIVEFIXED),
        IconKind::Location(kind) => {
            return known_folder_icon(kind).or_else(|| stock_icon(SIID_FOLDER));
        }
        _ => {}
    }
    if key.kind == IconKind::Drive {
        let Some(root) = &key.drive_root else {
            return stock_icon(SIID_DRIVEFIXED);
        };
        // No critical-error/media dialogs may escape this backend thread.
        let _mode = ErrorMode::new().ok()?;
        // SAFETY: the discovery bitmask query has no pointer arguments.
        let discovered = unsafe { GetLogicalDrives() };
        return drive_icon(
            root,
            discovered,
            |root| {
                // SAFETY: drive_icon supplies a validated NUL-terminated drive root.
                unsafe { GetDriveTypeW(root.as_ptr()) }
            },
            |root, flags| shell_icon(root, 0, flags),
            stock_icon,
        );
    }
    if let Some((stock, flags)) = link_stock_request(key.kind) {
        return stock_icon_with_flags(stock, flags);
    }
    let attributes = if key.kind == IconKind::Folder {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    shell_icon(
        &synthetic_name(key),
        attributes,
        SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
    )
}

fn link_stock_request(kind: IconKind) -> Option<(i32, u32)> {
    let stock = match kind {
        IconKind::FolderLink => SIID_FOLDER,
        IconKind::Link => SIID_DOCNOASSOC,
        _ => return None,
    };
    // The Shell renders its own configured overlay onto a trusted stock base.
    // No untrusted link path, target or arbitrary icon location is passed to the Shell.
    Some((stock, SHGSI_ICON | SHGSI_SMALLICON | SHGSI_LINKOVERLAY))
}

fn action_stock_id(role: ActionIcon) -> Option<i32> {
    match role {
        ActionIcon::Options => Some(SIID_SETTINGS),
        ActionIcon::Help => Some(SIID_HELP),
        ActionIcon::Rename => Some(SIID_RENAME),
        ActionIcon::Delete => Some(SIID_DELETE),
        ActionIcon::Properties | ActionIcon::About => Some(SIID_INFO),
        ActionIcon::NewFolder => Some(SIID_FOLDER),
        ActionIcon::OpenPath => Some(SIID_FOLDEROPEN),
        // Windows has no appropriate documented stock icon for these roles.
        ActionIcon::Back
        | ActionIcon::Forward
        | ActionIcon::Parent
        | ActionIcon::Refresh
        | ActionIcon::Bookmarks
        | ActionIcon::Terminal
        | ActionIcon::Run
        | ActionIcon::TabAdd
        | ActionIcon::CloseTab
        | ActionIcon::RemoveBookmark
        | ActionIcon::AddBookmark
        | ActionIcon::View
        | ActionIcon::Edit
        | ActionIcon::Copy
        | ActionIcon::Move
        | ActionIcon::UserMenu
        | ActionIcon::Menu
        | ActionIcon::SaveAgain
        | ActionIcon::Done
        | ActionIcon::Close
        | ActionIcon::FolderSizes
        | ActionIcon::Exit
        | ActionIcon::SortAscending
        | ActionIcon::SortDescending => None,
    }
}

fn location_folder_id(kind: LocationKind) -> Option<windows_sys::core::GUID> {
    Some(match kind {
        LocationKind::Home => FOLDERID_Profile,
        LocationKind::Desktop => FOLDERID_Desktop,
        LocationKind::Documents => FOLDERID_Documents,
        LocationKind::Downloads => FOLDERID_Downloads,
        LocationKind::Volume => return None,
    })
}

struct OwnedPidl(*mut ITEMIDLIST);

impl Drop for OwnedPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: SHGetKnownFolderIDList transfers this PIDL to the caller;
            // its documented allocator pair is ILFree, called exactly once here.
            unsafe { ILFree(self.0) };
        }
    }
}

fn known_folder_icon(kind: LocationKind) -> Option<OwnedIcon> {
    let id = location_folder_id(kind)?;
    let _mode = ErrorMode::new().ok()?;
    let mut raw = ptr::null_mut();
    // SAFETY: only fixed host-known GUIDs enter here. The output pointer is valid,
    // the current user's token is implicit, and flags never create/verify folders.
    let result = unsafe {
        SHGetKnownFolderIDList(&id, KF_FLAG_DONT_VERIFY as u32, ptr::null_mut(), &mut raw)
    };
    let pidl = OwnedPidl(raw);
    if result < 0 || pidl.0.is_null() {
        return None;
    }
    let mut info = SHFILEINFOW::default();
    // SAFETY: the owned absolute PIDL remains alive for the call. SHGFI_PIDL
    // explicitly interprets this pointer as ITEMIDLIST, never as a path string.
    // Existing load_icon COM/worker guards keep installed host handlers off UI.
    let result = unsafe {
        SHGetFileInfoW(
            pidl.0.cast(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_ICON | SHGFI_SMALLICON,
        )
    };
    (result != 0 && !info.hIcon.is_null()).then_some(OwnedIcon(info.hIcon))
}

/// Only an enumerated drive-letter root can reach an actual Shell path query.
/// Components plus raw length reject normalized-away dot components and device paths.
fn validated_drive_root(root: &Path) -> Option<(u8, Vec<u16>)> {
    let mut components = root.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let (letter, expected_length) = match prefix.kind() {
        Prefix::Disk(letter) => (letter, 3),
        Prefix::VerbatimDisk(letter) => (letter, 7),
        _ => return None,
    };
    if !letter.is_ascii_alphabetic()
        || components.next() != Some(Component::RootDir)
        || components.next().is_some()
    {
        return None;
    }
    let raw: Vec<_> = root
        .as_os_str()
        .encode_wide()
        .take(expected_length + 1)
        .collect();
    if raw.len() != expected_length || raw.contains(&0) {
        return None;
    }
    let letter = letter.to_ascii_uppercase();
    Some((letter - b'A', vec![u16::from(letter), 58, 92, 0]))
}

fn drive_icon<T>(
    root: &Path,
    discovered: u32,
    classify: impl FnOnce(&[u16]) -> u32,
    query: impl FnOnce(&[u16], u32) -> Option<T>,
    stock: impl FnOnce(i32) -> Option<T>,
) -> Option<T> {
    let (index, root) = validated_drive_root(root)?;
    if discovered & (1 << index) == 0 {
        return None;
    }
    let stock_id = match classify(&root) {
        DRIVE_FIXED => {
            // Host Shell may provide the system-volume decoration. Only roots
            // from discovery enter here; we never parse autorun/custom icon paths.
            return query(&root, SHGFI_ICON | SHGFI_SMALLICON).or_else(|| stock(SIID_DRIVEFIXED));
        }
        DRIVE_REMOTE => SIID_DRIVENET,
        DRIVE_CDROM => SIID_DRIVECD,
        DRIVE_REMOVABLE => SIID_DRIVEREMOVE,
        DRIVE_RAMDISK => SIID_DRIVERAM,
        _ => SIID_DRIVEUNKNOWN,
    };
    stock(stock_id)
}

fn stock_icon(stock: i32) -> Option<OwnedIcon> {
    stock_icon_with_flags(stock, SHGSI_ICON | SHGSI_SMALLICON)
}

fn stock_icon_with_flags(stock: i32, flags: u32) -> Option<OwnedIcon> {
    let mut info = SHSTOCKICONINFO {
        cbSize: size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: sized output structure; a stock lookup never probes media or network paths.
    if unsafe { SHGetStockIconInfo(stock, flags, &mut info) } < 0 {
        return None;
    }
    (!info.hIcon.is_null()).then_some(OwnedIcon(info.hIcon))
}

fn shell_icon(path: &[u16], attributes: u32, flags: u32) -> Option<OwnedIcon> {
    let mut info = SHFILEINFOW::default();
    // SAFETY: callers supply a NUL-terminated synthetic type name with USEFILEATTRIBUTES,
    // or a validated enumerated local fixed root. Actual listing paths never enter here.
    // Installed Shell handlers are trusted host integration, executed only on this worker.
    if unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            attributes,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    } == 0
    {
        return None;
    }
    (!info.hIcon.is_null()).then_some(OwnedIcon(info.hIcon))
}

struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    size: u16,
}

impl Dib {
    fn new(size: u16) -> Option<Self> {
        // SAFETY: null source asks for a memory DC; ownership is immediately guarded.
        let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
        if dc.is_null() {
            return None;
        }
        let mut surface = Self {
            dc,
            bitmap: ptr::null_mut(),
            previous: ptr::null_mut(),
            bits: ptr::null_mut(),
            size,
        };
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: i32::from(size),
                biHeight: -i32::from(size),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = ptr::null_mut();
        // SAFETY: bounded top-down BGRA DIB; native allocation lives until surface drop.
        surface.bitmap =
            unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0) };
        if surface.bitmap.is_null() || bits.is_null() {
            return None;
        }
        surface.bits = bits.cast();
        // SAFETY: both objects are live and exclusively owned by this worker.
        surface.previous = unsafe { SelectObject(dc, surface.bitmap) };
        if surface.previous.is_null() || surface.previous == (-1isize as HGDIOBJ) {
            return None;
        }
        Some(surface)
    }

    fn draw(&mut self, icon: HICON, background: u8) -> Option<Vec<u8>> {
        let length = usize::from(self.size) * usize::from(self.size) * 4;
        // SAFETY: DIB is exactly size*size*4 writable bytes, with no pending drawing.
        unsafe { std::slice::from_raw_parts_mut(self.bits, length) }
            .as_chunks_mut::<4>()
            .0
            .fill([background, background, background, 255]);
        // SAFETY: live icon/DC; dimensions are bounded. Flush synchronizes all GDI writes
        // before accessing the DIB's bytes, including before the next CPU-side fill.
        let success = unsafe {
            DrawIconEx(
                self.dc,
                0,
                0,
                icon,
                i32::from(self.size),
                i32::from(self.size),
                0,
                ptr::null_mut(),
                DI_NORMAL,
            )
        };
        let flushed = unsafe { GdiFlush() };
        if success == 0 || flushed == 0 {
            return None;
        }
        // SAFETY: GDI has completed and allocation remains owned by self.
        Some(unsafe { std::slice::from_raw_parts(self.bits, length) }.to_vec())
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: release only owned handles, restoring the original bitmap first.
        unsafe {
            if !self.previous.is_null() && self.previous != (-1isize as HGDIOBJ) {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            DeleteDC(self.dc);
        }
    }
}

/// Drawing against two opaque backgrounds also recovers old mask-based transparency.
fn straight_rgba(black: &[u8], white: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(black.len());
    for (black, white) in black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0)
    {
        let difference = (0..3)
            .map(|index| white[index].saturating_sub(black[index]))
            .max()
            .unwrap_or(255);
        let alpha = 255 - difference;
        let straight = |channel: u8| {
            if alpha == 0 {
                0
            } else {
                ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255)
                    as u8
            }
        };
        rgba.extend([
            straight(black[2]),
            straight(black[1]),
            straight(black[0]),
            alpha,
        ]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_core::IconKind;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::{Barrier, Mutex, MutexGuard};
    use windows_sys::Win32::{
        System::WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE, DRIVE_UNKNOWN,
        },
        UI::Shell::{
            SIID_DRIVECD, SIID_DRIVENET, SIID_DRIVERAM, SIID_DRIVEREMOVE, SIID_DRIVEUNKNOWN,
        },
    };

    // Qualify native artwork independently from its concurrent availability. Cold
    // parallel stock-icon calls can return E_OUTOFMEMORY on a healthy desktop.
    // The concurrent contract test below exercises those optional results itself.
    static NATIVE_QUALIFICATION: Mutex<()> = Mutex::new(());

    fn native_qualification() -> MutexGuard<'static, ()> {
        NATIVE_QUALIFICATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn folder_and_file_links_request_system_composites_without_any_path_lookup() {
        use windows_sys::Win32::UI::Shell::{SHGSI_LINKOVERLAY, SIID_DOCNOASSOC};
        for (kind, stock) in [
            (IconKind::FolderLink, SIID_FOLDER),
            (IconKind::Link, SIID_DOCNOASSOC),
        ] {
            assert_eq!(
                link_stock_request(kind),
                Some((stock, SHGSI_ICON | SHGSI_SMALLICON | SHGSI_LINKOVERLAY))
            );
        }
        assert_eq!(link_stock_request(IconKind::Folder), None);
        assert_eq!(link_stock_request(IconKind::File), None);
    }

    #[test]
    fn native_link_composite_ignores_arbitrary_root_and_extension_data() {
        let _qualification = native_qualification();
        let services = WindowsFileSystem::new(None);
        for kind in [IconKind::FolderLink, IconKind::Link] {
            let key = IconKey {
                kind,
                extension: Some("../../arbitrary.dll".into()),
                drive_root: Some("\\\\server\\untrusted\\target".into()),
                ..IconKey::file()
            };
            let icon = services
                .load_icon(&key)
                .unwrap()
                .expect("system link composite");
            assert!(
                icon.rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0)
            );
            let plain = services
                .load_icon(&IconKey {
                    kind: if kind == IconKind::FolderLink {
                        IconKind::Folder
                    } else {
                        IconKind::File
                    },
                    ..IconKey::file()
                })
                .unwrap()
                .unwrap();
            assert_ne!(
                icon.rgba, plain.rgba,
                "the system overlay must remain visible"
            );
        }
    }

    #[test]
    fn semantic_actions_only_use_appropriate_documented_stock_icons() {
        assert_eq!(
            action_stock_id(fm_core::ActionIcon::Options),
            Some(SIID_SETTINGS)
        );
        assert_eq!(
            action_stock_id(fm_core::ActionIcon::Delete),
            Some(SIID_DELETE)
        );
        for role in [
            fm_core::ActionIcon::Back,
            fm_core::ActionIcon::Copy,
            fm_core::ActionIcon::Move,
            fm_core::ActionIcon::Terminal,
            fm_core::ActionIcon::SortAscending,
            fm_core::ActionIcon::SortDescending,
        ] {
            assert_eq!(action_stock_id(role), None, "unsupported role {role:?}");
        }
    }

    #[test]
    fn opaque_black_placeholders_only_reject_semantic_art_not_native_file_or_drive_icons() {
        let mut pixels = vec![[0, 0, 0, 255]; 32 * 32];
        for kind in [
            IconKind::Action(ActionIcon::Options),
            IconKind::Location(LocationKind::Home),
        ] {
            assert!(opaque_black_placeholder(kind, &pixels));
        }
        for kind in [
            IconKind::File,
            IconKind::Folder,
            IconKind::Link,
            IconKind::Drive,
        ] {
            assert!(!opaque_black_placeholder(kind, &pixels));
        }
        // A real monochrome glyph has transparent surrounding pixels.
        pixels[0] = [0, 0, 0, 0];
        assert!(!opaque_black_placeholder(
            IconKind::Action(ActionIcon::Options),
            &pixels
        ));
        // Preserve even subtle genuine color variation rather than judging its style.
        pixels[0] = [1, 0, 0, 255];
        assert!(!opaque_black_placeholder(
            IconKind::Action(ActionIcon::Options),
            &pixels
        ));
        assert!(!opaque_black_placeholder(
            IconKind::Action(ActionIcon::Options),
            &[]
        ));
    }

    #[test]
    fn special_locations_resolve_only_the_trusted_known_folder_allowlist() {
        for (kind, expected) in [
            (fm_core::LocationKind::Home, FOLDERID_Profile),
            (fm_core::LocationKind::Desktop, FOLDERID_Desktop),
            (fm_core::LocationKind::Documents, FOLDERID_Documents),
            (fm_core::LocationKind::Downloads, FOLDERID_Downloads),
        ] {
            let actual = location_folder_id(kind).unwrap();
            assert_eq!(
                (actual.data1, actual.data2, actual.data3, actual.data4),
                (
                    expected.data1,
                    expected.data2,
                    expected.data3,
                    expected.data4
                )
            );
        }
        assert!(location_folder_id(fm_core::LocationKind::Volume).is_none());
    }

    #[test]
    fn native_action_and_known_folder_render_without_reading_a_supplied_path() {
        let _qualification = native_qualification();
        let services = WindowsFileSystem::new(None);
        for kind in [
            IconKind::Action(fm_core::ActionIcon::Options),
            IconKind::Location(fm_core::LocationKind::Home),
        ] {
            let key = IconKey {
                kind,
                extension: Some("../../arbitrary.dll".into()),
                drive_root: Some("\\\\server\\untrusted\\icon.dll".into()),
                size: 24,
            };
            let pixels = services.load_icon(&key).unwrap();
            if kind == IconKind::Action(ActionIcon::Options) && pixels.is_none() {
                continue;
            }
            let pixels = pixels.unwrap();
            assert_eq!(
                (pixels.width, pixels.height, pixels.rgba.len()),
                (24, 24, 2304)
            );
            assert!(
                pixels
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0),
                "host returned a transparent {kind:?} icon"
            );
        }
    }

    #[test]
    fn fixed_volume_icons_query_the_discovered_root_without_synthetic_attributes() {
        for root in ["C:\\", "\\\\?\\C:\\"] {
            let result = drive_icon(
                Path::new(root),
                1 << 2,
                |_| DRIVE_FIXED,
                |path, flags| {
                    assert_eq!(path, [67, 58, 92, 0]);
                    assert_eq!(flags, SHGFI_ICON | SHGFI_SMALLICON);
                    Some("native system decoration")
                },
                |_| panic!("fixed volume unexpectedly used the stock icon"),
            );
            assert_eq!(result, Some("native system decoration"));
        }
    }

    #[test]
    fn network_and_unavailable_media_use_stock_icons_without_shell_path_queries() {
        for (kind, expected) in [
            (DRIVE_REMOTE, SIID_DRIVENET),
            (DRIVE_CDROM, SIID_DRIVECD),
            (DRIVE_REMOVABLE, SIID_DRIVEREMOVE),
            (DRIVE_RAMDISK, SIID_DRIVERAM),
            (DRIVE_UNKNOWN, SIID_DRIVEUNKNOWN),
        ] {
            assert_eq!(
                drive_icon(
                    Path::new("C:\\"),
                    1 << 2,
                    |_| kind,
                    |_, _| panic!("non-fixed volume reached a Shell filesystem query"),
                    Some,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn fixed_volume_shell_failure_falls_back_to_its_system_stock_icon() {
        assert_eq!(
            drive_icon(
                Path::new("C:\\"),
                1 << 2,
                |_| DRIVE_FIXED,
                |_, _| None,
                Some
            ),
            Some(SIID_DRIVEFIXED)
        );
    }

    #[test]
    fn arbitrary_paths_devices_nul_and_undiscovered_roots_never_reach_native_queries() {
        for root in [
            "",
            "C:",
            "C:\\folder",
            "C:\\file.exe",
            "C:\\.",
            "C:\\..\\",
            "relative",
            "\\\\server\\share\\",
            "\\\\.\\C:\\",
            "\\\\?\\GLOBALROOT\\Device\\x",
            "C:\\\0",
            "Z:\\",
        ] {
            assert_eq!(
                drive_icon::<()>(
                    Path::new(root),
                    1 << 2,
                    |_| panic!("invalid root reached drive classification: {root:?}"),
                    |_, _| panic!("invalid root reached Shell lookup: {root:?}"),
                    |_| panic!("invalid root reached stock lookup: {root:?}"),
                ),
                None
            );
        }
    }

    #[test]
    fn current_native_volume_renders_bounded_nonempty_system_icon_pixels() {
        let _qualification = native_qualification();
        let current = std::env::current_dir().unwrap();
        let root = current.ancestors().last().unwrap();
        let pixels = WindowsFileSystem::new(None)
            .load_icon(&IconKey::for_drive(root))
            .unwrap()
            .unwrap();
        assert_eq!(
            (pixels.width, pixels.height, pixels.rgba.len()),
            (32, 32, 4096)
        );
        assert!(
            pixels
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0)
        );
    }

    #[test]
    fn concurrent_native_icon_requests_preserve_optional_availability_and_pixel_bounds() {
        let _qualification = native_qualification();
        let current = std::env::current_dir().unwrap();
        let root = current.ancestors().last().unwrap();
        let keys = [
            IconKey::for_drive(root),
            IconKey::folder_link(),
            IconKey {
                kind: IconKind::Link,
                ..IconKey::file()
            },
            IconKey::folder(),
            IconKey::file(),
            IconKey {
                kind: IconKind::Location(LocationKind::Home),
                ..IconKey::file()
            },
            IconKey::action(ActionIcon::Options),
        ];
        let start = Barrier::new(16);
        std::thread::scope(|scope| {
            for worker in 0..16 {
                let key = &keys[worker % keys.len()];
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    let Some(pixels) = WindowsFileSystem::new(None).load_icon(key).unwrap() else {
                        // Native art is optional, including native resource failures.
                        return;
                    };
                    assert_eq!(
                        (pixels.width, pixels.height, pixels.rgba.len()),
                        (32, 32, 4096)
                    );
                    assert!(
                        pixels
                            .rgba
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|pixel| pixel[3] != 0),
                        "native art must remain visible when available: {key:?}"
                    );
                });
            }
        });
    }

    #[test]
    fn composited_colors_recover_straight_alpha_including_legacy_transparency() {
        let black = [0, 0, 0, 255, 32, 64, 96, 255, 11, 22, 33, 255];
        let white = [255, 255, 255, 255, 159, 191, 223, 255, 11, 22, 33, 255];
        assert_eq!(
            straight_rgba(&black, &white),
            [0, 0, 0, 0, 191, 128, 64, 128, 33, 22, 11, 255]
        );
    }

    #[test]
    fn shell_icon_lookup_receives_only_bounded_synthetic_type_names() {
        let key = IconKey {
            kind: IconKind::File,
            extension: Some(OsString::from("文档")),
            drive_root: None,
            size: 32,
        };
        assert_eq!(
            String::from_utf16(&synthetic_name(&key)[..7]).unwrap(),
            "type.文档"
        );
        for extension in ["../../evil.exe", "x\\evil", "x\0exe", &"x".repeat(300)] {
            let key = IconKey {
                extension: Some(extension.into()),
                ..key.clone()
            };
            assert_eq!(
                synthetic_name(&key),
                "type".encode_utf16().chain(Some(0)).collect::<Vec<_>>()
            );
        }
    }
}
