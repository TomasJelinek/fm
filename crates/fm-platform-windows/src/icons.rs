use crate::{WindowsFileSystem, desktop::ComGuard, volume_details::ErrorMode};
use fm_core::{FsError, IconKey, IconKind, IconPixels, IconService};
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
            SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGSI_ICON,
            SHGSI_SMALLICON, SHGetFileInfoW, SHGetStockIconInfo, SHSTOCKICONINFO, SIID_DRIVECD,
            SIID_DRIVEFIXED, SIID_DRIVENET, SIID_DRIVERAM, SIID_DRIVEREMOVE, SIID_DRIVEUNKNOWN,
            SIID_LINK,
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
        Ok(IconPixels::new(
            u32::from(key.size),
            u32::from(key.size),
            straight_rgba(&black, &white),
        )
        .ok())
    }
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
    if key.kind == IconKind::Link {
        return stock_icon(SIID_LINK);
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
    let mut info = SHSTOCKICONINFO {
        cbSize: size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: sized output structure; a stock lookup never probes media or network paths.
    if unsafe { SHGetStockIconInfo(stock, SHGSI_ICON | SHGSI_SMALLICON, &mut info) } < 0 {
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
    use windows_sys::Win32::{
        System::WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE, DRIVE_UNKNOWN,
        },
        UI::Shell::{
            SIID_DRIVECD, SIID_DRIVENET, SIID_DRIVERAM, SIID_DRIVEREMOVE, SIID_DRIVEUNKNOWN,
        },
    };

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
