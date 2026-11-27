//! Whether the Recycle Bin takes items from a folder, asked on a worker before a Trash
//! delete is confirmed (`PreparedMutation::trash_support`), so that F8 where the Recycle
//! Bin would refuse asks for a permanent delete at once instead of starting a job that
//! fails. It reads what the Shell goes by and never the Recycle Bin or its contents:
//!
//! - the "Do not move deleted files to the Recycle Bin" policy (`NoRecycleFiles` under
//!   `Software\Microsoft\Windows\CurrentVersion\Policies\Explorer`, HKLM before HKCU, as
//!   the Shell reads its restrictions), which turns it off for every drive;
//! - the type of the folder's volume (the volume the folder is on, also when it is
//!   mounted in a folder or reached through `subst`): only fixed disks have a Recycle
//!   Bin. Network shares and mapped network drives, removable media (USB sticks, memory
//!   cards), CD and DVD drives and RAM disks have none, except in a known folder
//!   (Documents, say) redirected there, which the Shell gives one;
//! - on a fixed disk, that volume's "Don't move files to the Recycle Bin" setting:
//!   `NukeOnDelete` of `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\
//!   BitBucket\Volume\{volume GUID}`; for a volume without one, the older all-drives
//!   `NukeOnDelete` of `...\Explorer\BitBucket` (HKCU, then HKLM).
//!
//! What it cannot tell is `Available`: the Shell's move still decides, and a refusal is
//! then an ordinary error that keeps the item. Answers are reused for two seconds per
//! volume, so a selection spread over many folders of a volume (Find results) reads the
//! settings once.
use midnight_core::TrashSupport;
use std::{
    ffi::OsString,
    fs::File,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr,
    sync::Mutex,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        Storage::FileSystem::{
            GetDriveTypeW, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
            GetVolumePathNameW, VOLUME_NAME_GUID,
        },
        System::{
            Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD,
                RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
            },
            WindowsProgramming::{
                DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
            },
        },
        UI::Shell::{
            FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
            FOLDERID_Pictures, FOLDERID_Videos,
        },
    },
    core::GUID,
};

#[cfg(test)]
#[path = "mutation_trash_support_tests.rs"]
mod tests;

const POLICIES: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer";
const BIT_BUCKET: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket";
const REUSE: Duration = Duration::from_secs(2);

/// A volume's recent answers, by the folder's path prefix (a drive or a share) and the
/// volume's serial number.
type Answer = ((String, u32), Instant, TrashSupport);
static ANSWERS: Mutex<Vec<Answer>> = Mutex::new(Vec::new());

/// The answer for entries of `folder`, whose open handle is `handle`.
pub(super) fn support(folder: &Path, handle: &File) -> TrashSupport {
    let key = serial(handle).map(|serial| (prefix(folder), serial));
    let now = Instant::now();
    if let Some(key) = &key {
        let answers = ANSWERS.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((_, _, answer)) = answers
            .iter()
            .find(|(known, at, _)| known == key && now.duration_since(*at) < REUSE)
        {
            return answer.clone();
        }
    }
    let answer = volume(folder, handle).map_or(TrashSupport::Available, |volume| {
        decide(&volume, folder, &Registry)
    });
    if let Some(key) = key {
        let mut answers = ANSWERS.lock().unwrap_or_else(|error| error.into_inner());
        answers.retain(|(known, at, _)| *known != key && now.duration_since(*at) < REUSE);
        answers.push((key, now, answer.clone()));
    }
    answer
}

/// What the answer depends on about the folder's volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Volume {
    /// How the answer names it: `C:`, a folder it is mounted in, or `\\server\share`.
    pub(super) name: String,
    pub(super) drive: Drive,
    /// `{…}` of its `\\?\Volume{…}\` name, which names its settings key.
    pub(super) guid: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drive {
    /// A fixed disk: it has a Recycle Bin unless its setting turns it off.
    Fixed,
    /// A network, removable, CD/DVD or RAM drive: no Recycle Bin.
    WithoutBin,
    /// The drive type is unknown.
    Unknown,
}

/// The settings the answer reads; the registry, or a test's own.
pub(super) trait Settings {
    /// `NoRecycleFiles` is set.
    fn policy(&self) -> bool;
    /// The `NukeOnDelete` of the volume `{guid}`, if it has one.
    fn volume_nuke(&self, guid: &str) -> Option<bool>;
    /// The older all-drives `NukeOnDelete`, if set.
    fn global_nuke(&self) -> Option<bool>;
    /// The known folder that holds `folder` and has a Recycle Bin of its own: its path
    /// and whether that Recycle Bin is turned off.
    fn known_folder(&self, folder: &Path) -> Option<(PathBuf, bool)>;
}

pub(super) fn decide(volume: &Volume, folder: &Path, settings: &dyn Settings) -> TrashSupport {
    if settings.policy() {
        return TrashSupport::Unavailable("A policy turns off the Recycle Bin".into());
    }
    match volume.drive {
        Drive::Fixed => {
            let off = volume
                .guid
                .as_deref()
                .and_then(|guid| settings.volume_nuke(guid))
                .or_else(|| settings.global_nuke())
                .unwrap_or(false);
            if off {
                TrashSupport::Unavailable(format!(
                    "The Recycle Bin is turned off for {}",
                    volume.name
                ))
            } else {
                TrashSupport::Available
            }
        }
        Drive::WithoutBin => match settings.known_folder(folder) {
            Some((known, true)) => TrashSupport::Unavailable(format!(
                "The Recycle Bin is turned off for {}",
                known.display()
            )),
            Some((_, false)) => TrashSupport::Available,
            None => {
                TrashSupport::Unavailable(format!("There is no Recycle Bin on {}", volume.name))
            }
        },
        Drive::Unknown => TrashSupport::Available,
    }
}

pub(super) fn drive(drive_type: u32) -> Drive {
    match drive_type {
        DRIVE_FIXED => Drive::Fixed,
        DRIVE_REMOVABLE | DRIVE_REMOTE | DRIVE_CDROM | DRIVE_RAMDISK => Drive::WithoutBin,
        _ => Drive::Unknown,
    }
}

/// The volume of `folder`. A share is told by its path, without touching the network.
fn volume(folder: &Path, handle: &File) -> Option<Volume> {
    let Some(Component::Prefix(prefix)) = folder.components().next() else {
        return None;
    };
    let letter = match prefix.kind() {
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            return Some(Volume {
                name: format!(
                    "\\\\{}\\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                ),
                drive: Drive::WithoutBin,
                guid: None,
            });
        }
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => char::from(letter),
        _ => return None,
    };
    // The volume the folder is on: a volume mounted in a folder or reached through subst
    // has its own type and settings. A network drive has no volume name.
    let root = guid_root(handle);
    let drive_type = match &root {
        // SAFETY: a NUL-terminated `\\?\Volume{…}\` root; this only reads its type.
        Some(root) => unsafe { GetDriveTypeW(root.as_ptr()) },
        None => {
            let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain([0]).collect();
            // SAFETY: a NUL-terminated drive root; this only reads its type.
            unsafe { GetDriveTypeW(root.as_ptr()) }
        }
    };
    let guid = root.as_deref().and_then(guid_of);
    let name = mount_name(folder).unwrap_or_else(|| format!("{letter}:"));
    Some(Volume {
        name,
        drive: drive(drive_type),
        guid,
    })
}

/// The volume's serial number, to recognize its recent answer.
fn serial(handle: &File) -> Option<u32> {
    let mut serial = 0u32;
    // SAFETY: live handle; only the serial number is requested, other outputs are null.
    let done = unsafe {
        GetVolumeInformationByHandleW(
            handle.as_raw_handle(),
            ptr::null_mut(),
            0,
            &mut serial,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        )
    };
    (done != 0).then_some(serial)
}

/// The first component of `folder`, lowercased: `c:` or `\\server\share`.
fn prefix(folder: &Path) -> String {
    folder
        .components()
        .next()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `\\?\Volume{…}\`, NUL-terminated, of the volume the handle's file is on.
fn guid_root(handle: &File) -> Option<Vec<u16>> {
    let mut buffer = vec![0u16; 512];
    for _ in 0..3 {
        // SAFETY: live handle and a writable buffer of the stated length.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_GUID,
            )
        } as usize;
        if length == 0 {
            return None;
        }
        if length >= buffer.len() {
            buffer.resize(length + 1, 0);
            continue;
        }
        let name = &buffer[..length];
        let start: Vec<u16> = "\\\\?\\Volume{".encode_utf16().collect();
        if !name.starts_with(&start) {
            return None;
        }
        let end = name.iter().position(|unit| *unit == u16::from(b'}'))?;
        if name.get(end + 1) != Some(&u16::from(b'\\')) {
            return None;
        }
        let mut root = name[..end + 2].to_vec();
        root.push(0);
        return Some(root);
    }
    None
}

/// `{…}` of a NUL-terminated `\\?\Volume{…}\`.
fn guid_of(root: &[u16]) -> Option<String> {
    let text = String::from_utf16(root.strip_suffix(&[0]).unwrap_or(root)).ok()?;
    let guid = text.strip_prefix("\\\\?\\Volume")?.strip_suffix('\\')?;
    (guid.starts_with('{') && guid.ends_with('}')).then(|| guid.to_owned())
}

/// Where the folder's volume is mounted, as the user knows it: `C:` for a drive's root,
/// or the folder it is mounted in.
fn mount_name(folder: &Path) -> Option<String> {
    let input: Vec<u16> = plain(folder).encode_wide().chain([0]).collect();
    let mut root = vec![0u16; input.len() + 2];
    // SAFETY: a NUL-terminated input and a writable output of the stated capacity.
    if unsafe { GetVolumePathNameW(input.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let length = root.iter().position(|unit| *unit == 0)?;
    let root = String::from_utf16(&root[..length]).ok()?;
    let root = root.strip_prefix("\\\\?\\").unwrap_or(&root);
    let root = root.strip_suffix('\\').unwrap_or(root);
    (!root.is_empty()).then(|| root.to_owned())
}

/// `path` without a `\\?\` or `\\?\UNC\` prefix, as the Shell and the user spell it.
pub(super) fn plain(path: &Path) -> OsString {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let verbatim_unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let verbatim: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    if units.starts_with(&verbatim_unc) {
        let mut plain: Vec<u16> = "\\\\".encode_utf16().collect();
        plain.extend_from_slice(&units[verbatim_unc.len()..]);
        OsString::from_wide(&plain)
    } else if units.starts_with(&verbatim) {
        OsString::from_wide(&units[verbatim.len()..])
    } else {
        path.as_os_str().to_owned()
    }
}

/// Whether `folder` is `base` or inside it, ignoring case and a `\\?\` prefix.
pub(super) fn inside(folder: &Path, base: &Path) -> bool {
    let folder = plain(folder).to_string_lossy().to_lowercase();
    let base = plain(base).to_string_lossy().to_lowercase();
    let base = base.trim_end_matches('\\');
    !base.is_empty()
        && (folder == base
            || folder
                .strip_prefix(base)
                .is_some_and(|rest| rest.starts_with('\\')))
}

/// `{8-4-4-4-12}` as a GUID.
pub(super) fn parse_guid(text: &str) -> Option<GUID> {
    let hex = text.strip_prefix('{')?.strip_suffix('}')?;
    let groups: Vec<&str> = hex.split('-').collect();
    if groups.iter().map(|group| group.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12]
        || !groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return None;
    }
    u128::from_str_radix(&groups.concat(), 16)
        .ok()
        .map(GUID::from_u128)
}

pub(super) fn same(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

/// The settings in this user's registry, read only.
struct Registry;

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: this guard owns one successfully opened registry handle.
        unsafe { RegCloseKey(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A DWORD value, `name` of `root\subkey`.
fn dword(root: HKEY, subkey: &str, name: &str) -> Option<u32> {
    let (subkey, name) = (wide(subkey), wide(name));
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: NUL-terminated names and a writable DWORD of the stated size.
    let result = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&raw mut value).cast(),
            &mut size,
        )
    };
    (result == ERROR_SUCCESS).then_some(value)
}

impl Settings for Registry {
    fn policy(&self) -> bool {
        dword(HKEY_LOCAL_MACHINE, POLICIES, "NoRecycleFiles")
            .or_else(|| dword(HKEY_CURRENT_USER, POLICIES, "NoRecycleFiles"))
            .is_some_and(|value| value != 0)
    }

    fn volume_nuke(&self, guid: &str) -> Option<bool> {
        dword(
            HKEY_CURRENT_USER,
            &format!("{BIT_BUCKET}\\Volume\\{guid}"),
            "NukeOnDelete",
        )
        .map(|value| value != 0)
    }

    fn global_nuke(&self) -> Option<bool> {
        dword(HKEY_CURRENT_USER, BIT_BUCKET, "NukeOnDelete")
            .or_else(|| dword(HKEY_LOCAL_MACHINE, BIT_BUCKET, "NukeOnDelete"))
            .map(|value| value != 0)
    }

    fn known_folder(&self, folder: &Path) -> Option<(PathBuf, bool)> {
        // The known folders whose Recycle Bin the Shell keeps settings for, then the
        // user's folders that get one when they are redirected.
        let mut known = known_folder_bins();
        for id in [
            FOLDERID_Desktop,
            FOLDERID_Documents,
            FOLDERID_Downloads,
            FOLDERID_Music,
            FOLDERID_Pictures,
            FOLDERID_Videos,
        ] {
            if !known.iter().any(|(other, _)| same(other, &id)) {
                known.push((id, false));
            }
        }
        known.into_iter().find_map(|(id, off)| {
            crate::desktop::known_folder(&id)
                .filter(|path| inside(folder, path))
                .map(|path| (path, off))
        })
    }
}

/// `BitBucket\KnownFolder\{id}` keys and whether their `NukeOnDelete` is set.
fn known_folder_bins() -> Vec<(GUID, bool)> {
    let subkey = wide(&format!("{BIT_BUCKET}\\KnownFolder"));
    let mut raw = ptr::null_mut();
    // SAFETY: a NUL-terminated subkey and a valid output; read-only access.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut raw) }
        != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let key = Key(raw);
    let mut found = Vec::new();
    for index in 0..64 {
        // A registry key name has at most 255 characters.
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        // SAFETY: live key; the writable name buffer's length matches its allocation.
        let result = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if result != ERROR_SUCCESS {
            break;
        }
        let Ok(text) = String::from_utf16(&name[..length as usize]) else {
            continue;
        };
        let Some(id) = parse_guid(&text) else {
            continue;
        };
        let off = dword(
            HKEY_CURRENT_USER,
            &format!("{BIT_BUCKET}\\KnownFolder\\{text}"),
            "NukeOnDelete",
        )
        .is_some_and(|value| value != 0);
        found.push((id, off));
    }
    found
}
