//! Resource extraction only; no executable is loaded as code or started.
//!
//! An icon is drawn from the frame of its icon group that suits the requested edge:
//! the smallest frame at least that large, else the largest, and among frames of one
//! size the one with the most colors. PNG frames, which carry the 256 px artwork of
//! current programs, are decoded by the system. The frame is drawn at its own size
//! and reduced once (`icon_scale::fit`); it is enlarged only when its group has
//! nothing larger. Executables, DLLs and resource-only modules are mapped as data
//! files; icon files are read and parsed with bounds.
use super::*;
use midnight_core::Cancellation;
use midnight_platform_common::{icon_scale, icon_snapshot::SnapshotPixels};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::PathBuf,
};
use windows_sys::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_TYPE_DISK, GetFileType, GetFinalPathNameByHandleW,
        },
        System::LibraryLoader::{
            EnumResourceNamesW, FindResourceW, LOAD_LIBRARY_AS_DATAFILE,
            LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW, LoadResource, LockResource,
            SizeofResource,
        },
        UI::{
            Shell::SHDefExtractIconW,
            WindowsAndMessaging::{
                CreateIconFromResourceEx, LR_DEFAULTCOLOR, RT_GROUP_ICON, RT_ICON,
            },
        },
    },
    core::{BOOL, PCWSTR},
};

/// Frames considered per icon group or icon file.
const MAX_FRAMES: usize = 64;
/// One frame's encoded bytes: a 256 px frame is about 260 KiB uncompressed.
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Frames larger than this are not drawn: a 1024 px frame is 4 MiB of pixels.
const MAX_FRAME_EDGE: u32 = 1024;
/// Icon files larger than this are not read.
const MAX_ICON_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Resource names longer than this are not looked up.
const MAX_NAME_UNITS: usize = 256;
const PATH_UNITS: usize = 32768;

/// Explicitly configured Program artwork, at most 64 px.
pub(super) fn load(path: &Path, size: u16, cancel: &Cancellation) -> Option<IconPixels> {
    if !(1..=64).contains(&size) {
        return None;
    }
    let pixels = load_index(path, 0, size, cancel)?;
    IconPixels::new(pixels.width, pixels.height, pixels.rgba).ok()
}

/// The icon group at `index` of a module or icon file chosen in the icon picker.
pub(super) fn load_index(
    path: &Path,
    index: i32,
    size: u16,
    cancel: &Cancellation,
) -> Option<SnapshotPixels> {
    if !(1..=128).contains(&size) {
        return None;
    }
    let rgba = load_rgba(path, index, size, cancel)?;
    SnapshotPixels::new(u32::from(size), u32::from(size), rgba).ok()
}

/// A listed executable's own artwork at `edge`. The file is opened without
/// following a link, must be an ordinary disk file, and cannot be written, renamed or
/// deleted while its resources are read through the handle's final path.
pub(super) fn load_file(path: &Path, edge: u16, cancel: &Cancellation) -> Option<IconPixels> {
    if cancel.is_cancelled() || !path.is_absolute() {
        return None;
    }
    let (_pinned, resolved) = open_pinned(path).ok()?;
    let rgba = load_rgba(&resolved, 0, edge, cancel)?;
    IconPixels::new(u32::from(edge), u32::from(edge), rgba).ok()
}

/// Opens `path` as an ordinary disk file that others may read but not change, and
/// resolves its final path from that handle, never through a retargetable link.
pub(crate) fn open_pinned(path: &Path) -> io::Result<(File, PathBuf)> {
    let source = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = source.metadata()?;
    // SAFETY: source owns a live handle; this only queries its native file type.
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || unsafe { GetFileType(source.as_raw_handle()) } != FILE_TYPE_DISK
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose an ordinary executable, DLL, or icon file.",
        ));
    }
    let resolved = final_path(&source)?;
    Ok((source, resolved))
}

fn final_path(source: &File) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 256];
    loop {
        // SAFETY: the file handle and initialized writable UTF-16 buffer remain live.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                source.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= PATH_UNITS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The resource's native final path is too large.",
            ));
        }
        if count >= buffer.len() {
            buffer.resize(count + 1, 0);
            continue;
        }
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(
            &buffer[..count],
        )));
    }
}

/// Straight-alpha RGBA of `edge` by `edge` pixels for the icon group at `index`.
fn load_rgba(path: &Path, index: i32, edge: u16, cancel: &Cancellation) -> Option<Vec<u8>> {
    if index < 0
        || !path.is_absolute()
        || !(1..=midnight_core::MAX_ICON_EDGE).contains(&edge)
        || cancel.is_cancelled()
    {
        return None;
    }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    let _mode = ErrorMode::new().ok()?;
    let _com = ComGuard::initialize(COINIT_MULTITHREADED as u32)?;
    let wanted = u32::from(edge);
    let drawn = module_frame(&wide, index, wanted, cancel)
        .or_else(|| {
            (index == 0)
                .then(|| icon_file_frame(path, wanted, cancel))
                .flatten()
        })
        .or_else(|| shell_extracted(&wide, index, edge, cancel))?;
    if cancel.is_cancelled() {
        return None;
    }
    let rgba = icon_scale::fit(&drawn.rgba, drawn.edge, drawn.edge, wanted)?;
    rgba.as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] != 0)
        .then_some(rgba)
}

/// A frame drawn at its own size on a square transparent canvas.
struct Drawn {
    rgba: Vec<u8>,
    edge: u32,
}

/// One image of an icon group or icon file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    width: u32,
    height: u32,
    /// Bits per pixel; zero when unknown.
    depth: u16,
    /// The `RT_ICON` identifier of a module's frame, the file offset of an icon file's.
    at: usize,
    /// Encoded bytes, as the directory states them.
    bytes: usize,
}

impl Frame {
    fn edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// `frames` in the order they are tried for an `edge` icon: frames at least that
/// large from the smallest up, then smaller ones from the largest down; among frames
/// of one size the one with the most colors first.
fn ranked(frames: &[Frame], edge: u32) -> Vec<Frame> {
    let mut ranked: Vec<Frame> = frames
        .iter()
        .copied()
        .filter(|frame| (1..=MAX_FRAME_EDGE).contains(&frame.edge()))
        .collect();
    ranked.sort_by_key(|frame| {
        let size = frame.edge();
        let order = if size >= edge {
            (0, size)
        } else {
            (1, u32::MAX - size)
        };
        (order, std::cmp::Reverse(frame.depth))
    });
    ranked
}

/// A module's icon group, drawn from its best frame for `edge`.
fn module_frame(path: &[u16], index: i32, edge: u32, cancel: &Cancellation) -> Option<Drawn> {
    let module = DataModule::load(path)?;
    let name = module.group_name(usize::try_from(index).ok()?)?;
    let directory = module.resource(name.as_pcwstr(), RT_GROUP_ICON)?;
    let frames = group_frames(directory);
    for frame in ranked(&frames, edge) {
        if cancel.is_cancelled() {
            return None;
        }
        let id = u16::try_from(frame.at).ok()?;
        let Some(bits) = module.resource(id as usize as PCWSTR, RT_ICON) else {
            continue;
        };
        if let Some(drawn) = draw_frame(bits) {
            return Some(drawn);
        }
    }
    None
}

/// The entries of a `GRPICONDIR`: a six-byte header and fourteen-byte entries
/// whose last field is the `RT_ICON` identifier of the frame.
fn group_frames(directory: &[u8]) -> Vec<Frame> {
    directory_frames(directory, 14, |entry| {
        usize::from(u16::from_le_bytes([entry[12], entry[13]]))
    })
}

/// The entries of an icon file's `ICONDIR`: sixteen-byte entries whose last field
/// is the frame's offset in the file.
fn file_frames(directory: &[u8]) -> Vec<Frame> {
    directory_frames(directory, 16, |entry| {
        usize::try_from(u32::from_le_bytes([
            entry[12], entry[13], entry[14], entry[15],
        ]))
        .unwrap_or(usize::MAX)
    })
}

fn directory_frames(
    directory: &[u8],
    entry_size: usize,
    at: impl Fn(&[u8]) -> usize,
) -> Vec<Frame> {
    let Some(header) = directory.get(..6) else {
        return Vec::new();
    };
    if header[..4] != [0, 0, 1, 0] {
        return Vec::new();
    }
    let count = usize::from(u16::from_le_bytes([header[4], header[5]])).min(MAX_FRAMES);
    directory[6..]
        .chunks_exact(entry_size)
        .take(count)
        .map(|entry| Frame {
            // Zero stands for 256 in a directory's one-byte sizes.
            width: if entry[0] == 0 {
                256
            } else {
                u32::from(entry[0])
            },
            height: if entry[1] == 0 {
                256
            } else {
                u32::from(entry[1])
            },
            depth: u16::from_le_bytes([entry[6], entry[7]]),
            at: at(entry),
            bytes: usize::try_from(u32::from_le_bytes([
                entry[8], entry[9], entry[10], entry[11],
            ]))
            .unwrap_or(usize::MAX),
        })
        .collect()
}

/// An icon file's best frame for `edge`. Its directory sizes are checked against
/// each frame's own header, which also holds the true size of frames over 256 px.
fn icon_file_frame(path: &Path, edge: u32, cancel: &Cancellation) -> Option<Drawn> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    if !(6..=MAX_ICON_FILE_BYTES).contains(&length) {
        return None;
    }
    let mut bytes = vec![0; 6];
    file.read_exact(&mut bytes).ok()?;
    if bytes[..4] != [0, 0, 1, 0] {
        return None;
    }
    file.take(length - 6).read_to_end(&mut bytes).ok()?;
    let frames: Vec<Frame> = file_frames(&bytes)
        .into_iter()
        .filter_map(|frame| {
            let (width, height, depth) = frame_header(frame_bits(&bytes, &frame)?)?;
            Some(Frame {
                width,
                height,
                depth,
                ..frame
            })
        })
        .collect();
    for frame in ranked(&frames, edge) {
        if cancel.is_cancelled() {
            return None;
        }
        if let Some(drawn) = frame_bits(&bytes, &frame).and_then(draw_frame) {
            return Some(drawn);
        }
    }
    None
}

/// The bytes of an icon file's frame, as its directory entry places them.
fn frame_bits<'a>(file: &'a [u8], frame: &Frame) -> Option<&'a [u8]> {
    if frame.bytes == 0 || frame.bytes > MAX_FRAME_BYTES {
        return None;
    }
    file.get(frame.at..frame.at.checked_add(frame.bytes)?)
}

/// Width, height and bits per pixel from a frame's own PNG or bitmap header.
fn frame_header(bits: &[u8]) -> Option<(u32, u32, u16)> {
    const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let read_u32 = |at: usize, big: bool| -> Option<u32> {
        let bytes: [u8; 4] = bits.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    let (width, height, depth) = if bits.starts_with(&PNG) {
        if bits.get(12..16)? != b"IHDR" {
            return None;
        }
        let samples = match *bits.get(25)? {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => return None,
        };
        (
            read_u32(16, true)?,
            read_u32(20, true)?,
            u16::from(*bits.get(24)?) * samples,
        )
    } else {
        // BITMAPINFOHEADER; an icon's height covers its color and mask rows.
        if read_u32(0, false)? < 40 {
            return None;
        }
        let width = i32::from_le_bytes(bits.get(4..8)?.try_into().ok()?);
        let height = i32::from_le_bytes(bits.get(8..12)?.try_into().ok()?);
        (
            u32::try_from(width).ok()?,
            u32::try_from(height).ok()? / 2,
            u16::from_le_bytes(bits.get(14..16)?.try_into().ok()?),
        )
    };
    ((1..=MAX_FRAME_EDGE).contains(&width) && (1..=MAX_FRAME_EDGE).contains(&height))
        .then_some((width, height, depth))
}

/// A frame drawn at its own size, centered on a square transparent canvas.
fn draw_frame(bits: &[u8]) -> Option<Drawn> {
    let (width, height, _) = frame_header(bits)?;
    let edge = width.max(height);
    let size = u16::try_from(edge).ok()?;
    // SAFETY: `bits` is one complete, bounded frame (a bitmap or PNG image); the
    // system parses it and returns a new icon of its own size, owned below.
    let icon = unsafe {
        CreateIconFromResourceEx(
            bits.as_ptr(),
            u32::try_from(bits.len()).ok()?,
            1,
            0x0003_0000,
            i32::try_from(width).ok()?,
            i32::try_from(height).ok()?,
            LR_DEFAULTCOLOR,
        )
    };
    if icon.is_null() {
        return None;
    }
    let icon = OwnedIcon(icon);
    let mut surface = Dib::new(size)?;
    let place = (
        u16::try_from((edge - width) / 2).ok()?,
        u16::try_from((edge - height) / 2).ok()?,
        u16::try_from(width).ok()?,
        u16::try_from(height).ok()?,
    );
    let black = surface.draw_at(icon.0, place, 0)?;
    let white = surface.draw_at(icon.0, place, 255)?;
    Some(Drawn {
        rgba: straight_rgba(&black, &white),
        edge,
    })
}

/// The Shell's own extraction at `edge`, for formats read above only in part, such
/// as 16-bit executables and icon libraries. It may enlarge a small frame itself.
fn shell_extracted(path: &[u16], index: i32, edge: u16, cancel: &Cancellation) -> Option<Drawn> {
    let mut handle = ptr::null_mut();
    // SAFETY: the native path is terminated and the output points to a writable HICON.
    // The nonnegative index selects a resource; LOWORD requests this size.
    let result = unsafe {
        SHDefExtractIconW(
            path.as_ptr(),
            index,
            0,
            &mut handle,
            ptr::null_mut(),
            u32::from(edge),
        )
    };
    let icon = (!handle.is_null()).then_some(OwnedIcon(handle))?;
    if result < 0 || cancel.is_cancelled() {
        return None;
    }
    let mut surface = Dib::new(edge)?;
    let black = surface.draw(icon.0, 0)?;
    let white = surface.draw(icon.0, 255)?;
    Some(Drawn {
        rgba: straight_rgba(&black, &white),
        edge: u32::from(edge),
    })
}

/// A module mapped for its resources only: no code runs and no import is resolved.
struct DataModule(HMODULE);

enum ResourceName {
    Id(u16),
    /// NUL-terminated.
    Text(Vec<u16>),
}

impl ResourceName {
    fn as_pcwstr(&self) -> PCWSTR {
        match self {
            Self::Id(id) => *id as usize as PCWSTR,
            Self::Text(units) => units.as_ptr(),
        }
    }
}

impl DataModule {
    fn load(path: &[u16]) -> Option<Self> {
        // SAFETY: NUL-terminated path. As a data file and image resource the system
        // maps the file for resource access only; a non-module file fails here.
        let module = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            )
        };
        (!module.is_null()).then_some(Self(module))
    }

    /// The name of the `wanted`-th icon group in resource order, as icon indexes
    /// count them (`ExtractIconExW`): named groups first, then numbered ones.
    fn group_name(&self, wanted: usize) -> Option<ResourceName> {
        struct Search {
            wanted: usize,
            seen: usize,
            found: Option<ResourceName>,
        }
        unsafe extern "system" fn visit(_: HMODULE, _: PCWSTR, name: PCWSTR, state: isize) -> BOOL {
            // SAFETY: `state` is the exclusive `Search` below, alive for the whole
            // enumeration, which calls back synchronously on this thread.
            let search = unsafe { &mut *(state as *mut Search) };
            if search.seen < search.wanted {
                search.seen += 1;
                return 1;
            }
            search.found = if (name as usize) >> 16 == 0 {
                Some(ResourceName::Id(name as usize as u16))
            } else {
                let mut units = Vec::new();
                for at in 0..=MAX_NAME_UNITS {
                    // SAFETY: a string name is NUL-terminated and valid during the
                    // callback; reading stops at its terminator or at the bound.
                    let unit = unsafe { *name.add(at) };
                    units.push(unit);
                    if unit == 0 {
                        break;
                    }
                }
                (units.last() == Some(&0)).then_some(ResourceName::Text(units))
            };
            0
        }
        let mut search = Search {
            wanted,
            seen: 0,
            found: None,
        };
        // SAFETY: the module is loaded; the callback only touches `search`. A stopped
        // enumeration reports failure, which leaves `found` as decided.
        unsafe {
            EnumResourceNamesW(
                self.0,
                RT_GROUP_ICON,
                Some(visit),
                &mut search as *mut Search as isize,
            )
        };
        search.found
    }

    /// A resource's bytes, valid while the module stays loaded.
    fn resource(&self, name: PCWSTR, kind: PCWSTR) -> Option<&[u8]> {
        // SAFETY: the module is loaded; `name` is a live identifier or string.
        let info = unsafe { FindResourceW(self.0, name, kind) };
        if info.is_null() {
            return None;
        }
        // SAFETY: `info` was found in this module.
        let size = unsafe { SizeofResource(self.0, info) } as usize;
        if size == 0 || size > MAX_FRAME_BYTES {
            return None;
        }
        // SAFETY: as above; a data file's resource is mapped, not copied.
        let data = unsafe { LoadResource(self.0, info) };
        if data.is_null() {
            return None;
        }
        // SAFETY: `data` belongs to this module, which outlives the borrow.
        let bytes = unsafe { LockResource(data) };
        if bytes.is_null() {
            return None;
        }
        // SAFETY: the system maps `size` bytes for the module's lifetime.
        Some(unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), size) })
    }
}

impl Drop for DataModule {
    fn drop(&mut self) {
        // SAFETY: the module was loaded once by `load` and is released once here.
        unsafe { FreeLibrary(self.0) };
    }
}

#[cfg(test)]
#[path = "icons_executable_tests.rs"]
mod tests;
