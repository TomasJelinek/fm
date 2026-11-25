//! Locations mounted through GVfs. GVfs serves each location it mounts (an SMB share,
//! an SFTP host, a phone) as a folder of one FUSE mount, the bridge. The folder is
//! named after the location's mount specification: its type, a colon, then
//! comma-separated `key=value` items with percent-escaped values, such as
//! `smb-share:server=nas,share=data`.

#[cfg(test)]
#[path = "gvfs_tests.rs"]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::OsStrExt,
    path::Path,
};

/// At most this many GVfs locations join the drive list.
const MAX_LOCATIONS: usize = 256;

/// A location mounted through GVfs, offered as a drive.
#[derive(Debug, PartialEq, Eq)]
pub struct Location {
    /// The location's folder in the bridge, as raw path bytes.
    pub root: Vec<u8>,
    /// A readable name for the drive list; never an identity.
    pub label: String,
}

/// The folders of the GVfs `bridges` as drives, at most `MAX_LOCATIONS` in all and
/// without roots already in `listed`. Only the bridges' entry names are read: a
/// location's attributes come from its backend, which may be slow or offline, so no
/// location is opened or examined. A bridge that cannot be read adds nothing.
pub fn locations(
    bridges: &[Vec<u8>],
    listed: &[Vec<u8>],
    cancelled: &dyn Fn() -> bool,
) -> Vec<Location> {
    let listed: HashSet<&[u8]> = listed.iter().map(Vec::as_slice).collect();
    let mut found = Vec::new();
    for bridge in bridges {
        if cancelled() {
            break;
        }
        let Some(names) = names(Path::new(OsStr::from_bytes(bridge))) else {
            continue;
        };
        for name in names {
            if found.len() == MAX_LOCATIONS || cancelled() {
                return labeled(found);
            }
            let root = [bridge.as_slice(), b"/", name.as_bytes()].concat();
            if !listed.contains(root.as_slice()) {
                found.push((root, name));
            }
        }
    }
    labeled(found)
}

/// The locations with their labels. Locations that would share a label add what tells
/// them apart, when their folder names have it (see `detailed`).
fn labeled(found: Vec<(Vec<u8>, OsString)>) -> Vec<Location> {
    let labels: Vec<String> = found.iter().map(|(_, name)| label(name)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for label in &labels {
        *counts.entry(label).or_default() += 1;
    }
    let repeated: Vec<bool> = labels
        .iter()
        .map(|label| counts[label.as_str()] > 1)
        .collect();
    found
        .into_iter()
        .zip(labels)
        .zip(repeated)
        .map(|(((root, name), label), repeated)| Location {
            root,
            label: repeated
                .then(|| detailed(&name, &label))
                .flatten()
                .unwrap_or(label),
        })
        .collect()
}

/// The full name of the GVfs location whose folder in a bridge is `path`, from the
/// folder's name alone: its label with the user, domain and port the name has (see
/// `detailed`). `None` for any other path and for a name that does not parse. The drive
/// list may show the name shortened; this is the volume name that spells it out,
/// without asking the location's backend.
pub fn location_name(path: &Path) -> Option<String> {
    let bridge = path.parent()?;
    if !crate::mounts::gvfs_bridge(bridge.as_os_str().as_bytes()) {
        return None;
    }
    let name = path.file_name()?;
    let label = readable(name.to_str()?)?;
    Some(detailed(name, &label).unwrap_or(label))
}

/// The entry names of the directory `path` and nothing else about its entries: the
/// bridge reports no entry types, so `DirEntry::file_type` or `metadata` would examine
/// the location behind each name. A read error ends the names.
fn names(path: &Path) -> Option<impl Iterator<Item = OsString>> {
    let entries = fs::read_dir(path).ok()?;
    Some(entries.map_while(|entry| entry.ok().map(|entry| entry.file_name())))
}

/// A readable drive label for the GVfs folder `name`: "share on server" for an SMB
/// share, the host for SFTP, FTP or WebDAV, the device for a phone or camera and the
/// account for an online drive. A name this cannot read keeps its raw text.
fn label(name: &OsStr) -> String {
    name.to_str()
        .and_then(readable)
        .unwrap_or_else(|| name.to_string_lossy().into_owned())
}

/// `label` followed by what tells locations of one name apart: the user, the domain and
/// the port in the folder `name`, such as `example.com (alice, port 2222)`. `None` when
/// the name has none of them.
fn detailed(name: &OsStr, label: &str) -> Option<String> {
    let (_, values) = specification(name.to_str()?)?;
    let details: Vec<String> = [("user", ""), ("domain", ""), ("port", "port ")]
        .into_iter()
        .filter_map(|(key, prefix)| item(&values, key).map(|value| format!("{prefix}{value}")))
        .collect();
    (!details.is_empty()
        && !details
            .iter()
            .any(|detail| detail.contains(char::is_control)))
    .then(|| format!("{label} ({})", details.join(", ")))
}

/// The type and the decoded `key=value` items of the GVfs folder `name`; `None` when
/// it is not a mount specification.
fn specification(name: &str) -> Option<(&str, Vec<(&str, String)>)> {
    let (kind, items) = name.split_once(':').filter(|(kind, _)| !kind.is_empty())?;
    let mut values = Vec::new();
    for item in items.split(',').filter(|item| !item.is_empty()) {
        let (key, value) = item.split_once('=')?;
        values.push((key, unescape(value)?));
    }
    Some((kind, values))
}

/// The value of `key` among `values`, if it is not empty.
fn item<'a>(values: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    values
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
}

fn readable(name: &str) -> Option<String> {
    let (kind, values) = specification(name)?;
    let value = |key: &str| item(&values, key);
    let label = match kind {
        "smb-share" => format!("{} on {}", value("share")?, value("server")?),
        "afp-volume" => format!("{} on {}", value("volume")?, value("host")?),
        // An NFS location is one export of its host, kept as the prefix.
        "nfs" => match value("prefix")
            .and_then(|prefix| prefix.rsplit('/').find(|part| !part.is_empty()))
        {
            Some(export) => format!("{export} on {}", value("host")?),
            None => value("host")?.to_owned(),
        },
        // Phones and cameras are named by their device serial: vendor_model_serial.
        "mtp" | "gphoto2" => value("host")?.replace('_', " "),
        "google-drive" | "onedrive" => format!("{}@{}", value("user")?, value("host")?),
        // An archive's host is its file's escaped URI; the file's name names it.
        "archive" => unescape(unescape(value("host")?)?.rsplit('/').next()?)?,
        _ => value("host").or_else(|| value("server"))?.to_owned(),
    };
    (!label.is_empty() && !label.chars().any(char::is_control)).then_some(label)
}

/// `value` with its percent escapes decoded, when they are complete and the result is
/// UTF-8 text.
fn unescape(value: &str) -> Option<String> {
    let mut bytes = value.bytes();
    let mut decoded = Vec::with_capacity(value.len());
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            (hex(bytes.next()?)? << 4) | hex(bytes.next()?)?
        } else {
            byte
        });
    }
    String::from_utf8(decoded).ok()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}
