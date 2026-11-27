//! Shared libraries that the browser cannot load. A browser whose dynamic loader stops it
//! (exit status 127) closes its output before its first message, which the host could only
//! report as a stopped transport ("failed to fill whole buffer"). These functions name the
//! libraries instead, and how to install them.
//!
//! They read the ELF dynamic sections of the browser executable and of every library it
//! needs, and look each needed library up as glibc's loader does: the needing object's
//! DT_RPATH and those of the objects that needed it (unless it has DT_RUNPATH), then its
//! DT_RUNPATH, then `/etc/ld.so.cache`, then the loader's default folders. The browser runs
//! with an empty environment, so `LD_LIBRARY_PATH` plays no part. Nothing is executed (no
//! `ldd`, no `ld.so --list`): only regular files are opened, without blocking on a FIFO,
//! and every read is bounded. They run only after the browser ended with the loader's
//! status, so a lookup that differs from the loader's can at worst word the message of a
//! browser that could not start anyway; it never keeps a working browser from starting.
use super::super::LibrariesMissing;
use rustix::fs::{Mode, OFlags};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    fs::{self, File},
    io,
    os::unix::{ffi::OsStrExt, fs::FileExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    rc::Rc,
};

#[cfg(test)]
#[path = "browser_libraries_linux_tests.rs"]
mod tests;

/// ld.so's exit status when it cannot load a needed shared library.
const LOADER_FAILED: i32 = 127;
/// Objects read at most; Chromium's whole closure is about 150 on a desktop.
const MAX_OBJECTS: usize = 1024;
const MAX_PROGRAM_HEADERS: usize = 256;
/// Bytes of one dynamic section read at most (4,096 entries).
const MAX_DYNAMIC: u64 = 64 * 1024;
/// Longest name or search path read from a string table.
const MAX_STRING: u64 = 4096;
/// Largest `/etc/ld.so.cache` read.
const MAX_CACHE: u64 = 16 * 1024 * 1024;
/// Missing libraries named in the message; the rest are counted.
const MAX_LISTED: usize = 12;
const CACHE: &str = "/etc/ld.so.cache";
const SETUP: &str = "setup-linux.sh";

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
const DT_SONAME: u64 = 14;
const DT_RPATH: u64 = 15;
const DT_RUNPATH: u64 = 29;

/// The Debian and Ubuntu (24.04) package of each shared library that the HTML viewer's
/// Chromium (CEF) needs. `scripts/setup-linux.sh --install` installs every one of them,
/// with their sonames as the package capabilities for dnf and zypper (a test checks this).
pub(super) const DEBIAN_PACKAGES: &[(&str, &str)] = &[
    ("libnss3.so", "libnss3"),
    ("libnssutil3.so", "libnss3"),
    ("libsmime3.so", "libnss3"),
    ("libnspr4.so", "libnspr4"),
    ("libplc4.so", "libnspr4"),
    ("libplds4.so", "libnspr4"),
    ("libasound.so.2", "libasound2t64"),
    ("libatk-1.0.so.0", "libatk1.0-0t64"),
    ("libatk-bridge-2.0.so.0", "libatk-bridge2.0-0t64"),
    ("libatspi.so.0", "libatspi2.0-0t64"),
    ("libcups.so.2", "libcups2t64"),
    ("libdbus-1.so.3", "libdbus-1-3"),
    ("libdrm.so.2", "libdrm2"),
    ("libexpat.so.1", "libexpat1"),
    ("libgbm.so.1", "libgbm1"),
    ("libglib-2.0.so.0", "libglib2.0-0t64"),
    ("libgobject-2.0.so.0", "libglib2.0-0t64"),
    ("libgio-2.0.so.0", "libglib2.0-0t64"),
    ("libpango-1.0.so.0", "libpango-1.0-0"),
    ("libcairo.so.2", "libcairo2"),
    ("libX11.so.6", "libx11-6"),
    ("libxcb.so.1", "libxcb1"),
    ("libxkbcommon.so.0", "libxkbcommon0"),
    ("libXcomposite.so.1", "libxcomposite1"),
    ("libXdamage.so.1", "libxdamage1"),
    ("libXext.so.6", "libxext6"),
    ("libXfixes.so.3", "libxfixes3"),
    ("libXrandr.so.2", "libxrandr2"),
];

/// Once the browser `executable` ended with `status` before its first message: the shared
/// libraries its loader could not find, when it ended with the loader's status 127 and the
/// libraries it needs show which, as a [`LibrariesMissing`]. `None` otherwise. Reads files
/// only; call after the process was waited for, on a backend thread.
pub(super) fn diagnose(status: ExitStatus, executable: &Path) -> Option<io::Error> {
    if status.code() != Some(LOADER_FAILED) {
        return None;
    }
    let missing = missing(executable);
    if missing.is_empty() {
        return None;
    }
    let message = message(&missing, running_family(), checkout(executable));
    Some(io::Error::new(
        io::ErrorKind::NotFound,
        LibrariesMissing::new(missing, message),
    ))
}

/// The libraries that `executable` needs, directly or through other libraries, and that
/// its loader cannot find on this system, in the order the loader meets them.
pub(super) fn missing(executable: &Path) -> Vec<String> {
    match object(executable, None) {
        Some((machine, _)) => missing_with(executable, &System::read(machine)),
        None => Vec::new(),
    }
}

/// Where the loader looks after an object's own search paths.
#[derive(Debug, Default)]
pub(super) struct System {
    /// `/etc/ld.so.cache`: each soname with the paths it lists.
    cache: HashMap<String, Vec<PathBuf>>,
    /// The loader's built-in folders for this machine.
    defaults: Vec<PathBuf>,
}

impl System {
    fn read(machine: u16) -> Self {
        Self {
            cache: read_regular(Path::new(CACHE), MAX_CACHE)
                .map(|bytes| cache_entries(&bytes))
                .unwrap_or_default(),
            defaults: default_folders(machine),
        }
    }
}

/// The folders glibc searches last, as distributions build it: Debian's multiarch folders,
/// the `lib64` folders of Fedora and openSUSE, then `/lib` and `/usr/lib`. A folder that
/// this system's loader does not search can only hide a missing library, never report a
/// present one as missing.
fn default_folders(machine: u16) -> Vec<PathBuf> {
    let multiarch = match machine {
        EM_X86_64 => Some("x86_64-linux-gnu"),
        EM_AARCH64 => Some("aarch64-linux-gnu"),
        _ => None,
    };
    let mut folders = Vec::new();
    if let Some(triplet) = multiarch {
        folders.push(PathBuf::from(format!("/lib/{triplet}")));
        folders.push(PathBuf::from(format!("/usr/lib/{triplet}")));
    }
    folders.extend(["/lib64", "/usr/lib64", "/lib", "/usr/lib"].map(PathBuf::from));
    folders
}

/// What one object's dynamic section declares.
#[derive(Debug, Default)]
struct Dynamic {
    needed: Vec<String>,
    soname: Option<String>,
    rpath: Option<String>,
    runpath: Option<String>,
}

/// A loaded object and where the libraries that it needs are looked for.
struct Loaded {
    needed: Vec<String>,
    /// Its DT_RUNPATH folders: with them, no DT_RPATH is used for its own needs.
    runpath: Option<Vec<PathBuf>>,
    /// Its own DT_RPATH folders (none when it has DT_RUNPATH, which glibc then ignores),
    /// then those of the objects that needed it, up to the executable.
    rpath: Rc<Vec<PathBuf>>,
}

impl Loaded {
    fn new(dynamic: Dynamic, origin: &Path, inherited: &Rc<Vec<PathBuf>>) -> Self {
        let (runpath, rpath) = match (dynamic.runpath, dynamic.rpath) {
            (Some(runpath), _) => (Some(expand(&runpath, origin)), inherited.clone()),
            // Most libraries have neither: they share the chain of the object that needed them.
            (None, None) => (None, inherited.clone()),
            (None, Some(rpath)) => {
                let mut own = expand(&rpath, origin);
                own.extend(inherited.iter().cloned());
                (None, Rc::new(own))
            }
        };
        Self {
            needed: dynamic.needed,
            runpath,
            rpath,
        }
    }
}

/// [`missing`] with the cache and default folders of `system`.
fn missing_with(executable: &Path, system: &System) -> Vec<String> {
    let Some((machine, dynamic)) = object(executable, None) else {
        return Vec::new();
    };
    // `$ORIGIN` of the executable is the folder of its real path, as the kernel reports it;
    // that of a library is the folder of the path it was found at.
    let real = fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    let origin = real.parent().unwrap_or(Path::new("/"));
    let mut satisfied: HashSet<String> = dynamic.soname.iter().cloned().collect();
    let mut missing = Vec::new();
    let mut pending = VecDeque::from([Loaded::new(dynamic, origin, &Rc::default())]);
    let mut objects = 1;
    while let Some(object) = pending.pop_front() {
        for name in &object.needed {
            if satisfied.contains(name) || missing.contains(name) {
                continue;
            }
            let Some((path, dynamic)) = find(name, &object, machine, system) else {
                missing.push(name.clone());
                continue;
            };
            objects += 1;
            if objects > MAX_OBJECTS {
                return missing;
            }
            satisfied.insert(name.clone());
            satisfied.extend(dynamic.soname.clone());
            let origin = path.parent().unwrap_or(Path::new("/"));
            pending.push_back(Loaded::new(dynamic, origin, &object.rpath));
        }
    }
    missing
}

/// The file that the loader would load for `name`, needed by `requester`, and its dynamic
/// section. A candidate of another class or machine is skipped, as the loader skips it.
fn find(
    name: &str,
    requester: &Loaded,
    machine: u16,
    system: &System,
) -> Option<(PathBuf, Dynamic)> {
    let load = |path: PathBuf| object(&path, Some(machine)).map(|(_, dynamic)| (path, dynamic));
    if name.contains('/') {
        // A path, relative to the browser's working folder, which is the root.
        return load(Path::new("/").join(name));
    }
    let own = requester.runpath.as_deref().unwrap_or(&requester.rpath);
    own.iter()
        .map(|folder| folder.join(name))
        .chain(system.cache.get(name).into_iter().flatten().cloned())
        .chain(system.defaults.iter().map(|folder| folder.join(name)))
        .find_map(load)
}

/// The folders of a DT_RPATH or DT_RUNPATH list, with `$ORIGIN` replaced by `origin`.
/// A folder with another substitution (`$LIB`, `$PLATFORM`) is left out; a relative one
/// is relative to the browser's working folder, the root.
fn expand(list: &str, origin: &Path) -> Vec<PathBuf> {
    list.split(':')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let mut path = Vec::new();
            let mut rest = entry.as_bytes();
            while let Some(at) = rest.iter().position(|byte| *byte == b'$') {
                path.extend_from_slice(&rest[..at]);
                rest = &rest[at..];
                let after = match rest.strip_prefix(b"${ORIGIN}") {
                    Some(after) => after,
                    None => rest
                        .strip_prefix(b"$ORIGIN")
                        .filter(|after| !after.first().is_some_and(identifier))?,
                };
                path.extend_from_slice(origin.as_os_str().as_bytes());
                rest = after;
            }
            path.extend_from_slice(rest);
            Some(Path::new("/").join(OsStr::from_bytes(&path)))
        })
        .collect()
}

fn identifier(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

/// Open `path` for reading if it is a regular file, never blocking on a FIFO or device.
fn open_regular(path: &Path) -> Option<File> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY;
    let file = File::from(rustix::fs::open(path, flags, Mode::empty()).ok()?);
    file.metadata()
        .ok()
        .filter(fs::Metadata::is_file)
        .map(|_| file)
}

/// A regular file's contents, if it holds at most `limit` bytes.
fn read_regular(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = open_regular(path)?;
    let length = file.metadata().ok()?.len();
    if length > limit {
        return None;
    }
    let mut bytes = vec![0; usize::try_from(length).ok()?];
    let read = read_at_most(&file, &mut bytes, 0).ok()?;
    bytes.truncate(read);
    Some(bytes)
}

/// Fill `buffer` from `offset` until it is full or the file ends; the bytes read.
fn read_at_most(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut total = 0;
    while total < buffer.len() {
        let position = offset
            .checked_add(total as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        match file.read_at(&mut buffer[total..], position) {
            Ok(0) => break,
            Ok(count) => total += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(total)
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

/// A program header's type, file offset, address and size in the file.
struct Segment {
    kind: u32,
    offset: u64,
    address: u64,
    size: u64,
}

/// `path` as an ELF64 little-endian program or shared object that the loader would load
/// for `machine` (any machine when `None`): its machine and its dynamic section. `None`
/// for anything else: no regular file, unreadable, another class, byte order or machine,
/// or malformed.
fn object(path: &Path, machine: Option<u16>) -> Option<(u16, Dynamic)> {
    let file = open_regular(path)?;
    let mut header = [0; 64];
    file.read_exact_at(&mut header, 0).ok()?;
    // Magic, 64-bit class, little-endian data, current version.
    if header[..7] != *b"\x7fELF\x02\x01\x01" {
        return None;
    }
    let found = u16_at(&header, 18);
    if !matches!(u16_at(&header, 16), ET_EXEC | ET_DYN) || machine.is_some_and(|m| m != found) {
        return None;
    }
    let count = usize::from(u16_at(&header, 56));
    if u16_at(&header, 54) != 56 || count > MAX_PROGRAM_HEADERS {
        return None;
    }
    let mut table = vec![0; count * 56];
    file.read_exact_at(&mut table, u64_at(&header, 32)).ok()?;
    let segments: Vec<Segment> = table
        .as_chunks::<56>()
        .0
        .iter()
        .map(|entry| Segment {
            kind: u32_at(entry, 0),
            offset: u64_at(entry, 8),
            address: u64_at(entry, 16),
            size: u64_at(entry, 32),
        })
        .collect();
    let Some(section) = segments.iter().find(|segment| segment.kind == PT_DYNAMIC) else {
        // Statically linked: it needs nothing.
        return Some((found, Dynamic::default()));
    };
    let mut entries = vec![0; usize::try_from(section.size.min(MAX_DYNAMIC) / 16 * 16).ok()?];
    file.read_exact_at(&mut entries, section.offset).ok()?;
    let entries: Vec<(u64, u64)> = entries
        .as_chunks::<16>()
        .0
        .iter()
        .map(|entry| (u64_at(entry, 0), u64_at(entry, 8)))
        .take_while(|(tag, _)| *tag != DT_NULL)
        .collect();
    let value = |wanted| entries.iter().find(|(tag, _)| *tag == wanted).map(|e| e.1);
    let mut dynamic = Dynamic::default();
    let strings = entries
        .iter()
        .any(|(tag, _)| matches!(*tag, DT_NEEDED | DT_SONAME | DT_RPATH | DT_RUNPATH));
    if !strings {
        return Some((found, dynamic));
    }
    // DT_STRTAB is an address: the loaded segment that holds it gives its file offset.
    let (address, size) = (value(DT_STRTAB)?, value(DT_STRSZ)?);
    let table = segments.iter().find_map(|segment| {
        let within = address.checked_sub(segment.address)?;
        (segment.kind == PT_LOAD && within < segment.size)
            .then(|| segment.offset.checked_add(within))
            .flatten()
    })?;
    let text = |offset: u64| string(&file, table, size, offset);
    for (tag, offset) in &entries {
        match *tag {
            DT_NEEDED => dynamic.needed.push(text(*offset)?),
            DT_SONAME => dynamic.soname = Some(text(*offset)?),
            DT_RPATH => dynamic.rpath = Some(text(*offset)?),
            DT_RUNPATH => dynamic.runpath = Some(text(*offset)?),
            _ => {}
        }
    }
    Some((found, dynamic))
}

/// The NUL-terminated string at `offset` of a string table of `size` bytes at `table`.
fn string(file: &File, table: u64, size: u64, offset: u64) -> Option<String> {
    let length = size.checked_sub(offset).filter(|length| *length > 0)?;
    let mut bytes = vec![0; usize::try_from(length.min(MAX_STRING)).ok()?];
    let read = read_at_most(file, &mut bytes, table.checked_add(offset)?).ok()?;
    let end = bytes[..read].iter().position(|byte| *byte == 0)?;
    (end > 0).then(|| String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// The entries of an `ld.so.cache`: glibc's current format, alone or after the old one.
/// A cache of the other byte order, of an unknown format or damaged lists nothing.
fn cache_entries(bytes: &[u8]) -> HashMap<String, Vec<PathBuf>> {
    const NEW: &[u8] = b"glibc-ld.so.cache1.1";
    const OLD: &[u8] = b"ld.so-1.7.0";
    const HEADER: usize = 48;
    const ENTRY: usize = 24;
    let start = if bytes.starts_with(NEW) {
        Some(0)
    } else if bytes.starts_with(OLD) && bytes.len() >= 16 {
        // The old header (magic, then the entry count at 12) and its 12-byte entries, then
        // the current format aligned to 8 bytes.
        (u32_ne(bytes, 12) as usize)
            .checked_mul(12)
            .and_then(|entries| entries.checked_add(16 + 7))
            .map(|end| end & !7)
            .filter(|start| {
                bytes
                    .get(*start..)
                    .is_some_and(|rest| rest.starts_with(NEW))
            })
    } else {
        None
    };
    let mut entries: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let Some(cache) = start.and_then(|start| bytes.get(start..)) else {
        return entries;
    };
    if cache.len() < HEADER {
        return entries;
    }
    // Byte order: 0 unrecorded, 1 invalid, 2 little-endian, 3 big-endian.
    let order = cache[28] & 3;
    if order != 0 && order != if cfg!(target_endian = "little") { 2 } else { 3 } {
        return entries;
    }
    let count = (u32_ne(cache, 20) as usize).min((cache.len() - HEADER) / ENTRY);
    for index in 0..count {
        let at = HEADER + index * ENTRY;
        let text = |offset: u32| {
            let rest = cache.get(offset as usize..)?;
            let end = rest.iter().position(|byte| *byte == 0)?;
            (end > 0).then(|| &rest[..end])
        };
        if let (Some(name), Some(path)) = (text(u32_ne(cache, at + 4)), text(u32_ne(cache, at + 8)))
        {
            entries
                .entry(String::from_utf8_lossy(name).into_owned())
                .or_default()
                .push(PathBuf::from(OsStr::from_bytes(path)));
        }
    }
    entries
}

fn u32_ne(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_ne_bytes)
}

/// What the running distribution installs packages with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// apt: Debian, Ubuntu and derivatives such as Linux Mint.
    Debian,
    /// dnf: Fedora and the RHEL family.
    Fedora,
    /// zypper: openSUSE and SUSE Linux Enterprise.
    Suse,
    Other,
}

/// The family from os-release's `ID` and `ID_LIKE`.
fn family(os_release: &str) -> Family {
    let mut names = Vec::new();
    for line in os_release.lines() {
        if let Some((key, value)) = line.split_once('=')
            && matches!(key.trim(), "ID" | "ID_LIKE")
        {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            names.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let has = |wanted: &[&str]| names.iter().any(|name| wanted.contains(&name.as_str()));
    if has(&["debian", "ubuntu", "linuxmint"]) {
        Family::Debian
    } else if has(&["fedora", "rhel", "centos", "rocky", "almalinux"]) {
        Family::Fedora
    } else if names.iter().any(|name| {
        name.starts_with("opensuse") || matches!(name.as_str(), "suse" | "sles" | "sled")
    }) {
        Family::Suse
    } else {
        Family::Other
    }
}

fn running_family() -> Family {
    ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|path| read_regular(Path::new(path), 64 * 1024))
        .map_or(Family::Other, |text| {
            family(&String::from_utf8_lossy(&text))
        })
}

/// A development build in a source checkout
/// (`<checkout>/_build/<profile>/viewers/<package>/<executable>`), whose
/// `scripts/setup-linux.sh --install` installs every package that the application needs.
fn checkout(executable: &Path) -> bool {
    executable.ancestors().nth(4).is_some_and(|build| {
        build.file_name() == Some(OsStr::new("_build"))
            && build
                .parent()
                .is_some_and(|checkout| checkout.join("scripts").join(SETUP).is_file())
    })
}

/// A library name as the message shows it: letters, digits and `._+-`, at most 64 of
/// them; anything else (never seen in a real soname) as `?`.
fn shown(name: &str) -> String {
    name.chars()
        .take(64)
        .map(|c| if plain(c) { c } else { '?' })
        .collect()
}

fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._+-".contains(c)
}

/// The message for the `missing` libraries: what is missing, and the command that installs
/// it on this distribution family; in a source checkout, its setup script too.
fn message(missing: &[String], family: Family, checkout: bool) -> String {
    let mut text = format!(
        "The HTML viewer needs system libraries that are not installed: {}.",
        listed(missing)
    );
    // The packages that the command installs, and the libraries it cannot name.
    let mut packages: Vec<String> = Vec::new();
    let mut unnamed: Vec<String> = Vec::new();
    for name in missing {
        let package = match family {
            Family::Debian => DEBIAN_PACKAGES
                .iter()
                .find(|(soname, _)| soname == name)
                .map(|(_, package)| (*package).to_owned()),
            // RPM packages provide each library's soname as a capability.
            Family::Fedora | Family::Suse => name
                .chars()
                .all(plain)
                .then(|| format!("'{name}()(64bit)'")),
            Family::Other => None,
        };
        match package {
            Some(package) if !packages.contains(&package) => packages.push(package),
            Some(_) => {}
            None => unnamed.push(name.clone()),
        }
    }
    let tool = match family {
        Family::Debian => "apt install",
        Family::Fedora => "dnf install",
        Family::Suse => "zypper install",
        Family::Other => "",
    };
    if packages.is_empty() {
        text.push_str(
            " Install the packages of your distribution that provide them \
             (see system-requirements.md).",
        );
        return text;
    }
    text.push_str(" Install them with");
    if checkout {
        text.push_str(" scripts/setup-linux.sh --install in the source checkout, or with");
    }
    text.push_str(&format!(": sudo {tool} {}", packages.join(" ")));
    if !unnamed.is_empty() {
        text.push_str(&format!(
            ", and the packages that provide {}",
            listed(&unnamed)
        ));
    }
    text
}

/// Library names as the message lists them: at most `MAX_LISTED`, then how many more.
fn listed(names: &[String]) -> String {
    let mut items: Vec<String> = names.iter().take(MAX_LISTED).map(|n| shown(n)).collect();
    if names.len() > MAX_LISTED {
        items.push(format!("and {} more", names.len() - MAX_LISTED));
    }
    items.join(", ")
}
