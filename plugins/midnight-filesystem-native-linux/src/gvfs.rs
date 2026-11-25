//! Locations mounted through GVfs. GVfs serves each location it mounts (an SMB share,
//! an SFTP host, a phone) as a folder of one FUSE mount, the bridge. The folder is
//! named after the location's mount specification: its type, a colon, then
//! comma-separated `key=value` items with percent-escaped values, such as
//! `smb-share:server=nas,share=data`.

#[cfg(test)]
#[path = "gvfs_tests.rs"]
mod tests;

use std::{
    collections::HashSet,
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
                return found;
            }
            let root = [bridge.as_slice(), b"/", name.as_bytes()].concat();
            if !listed.contains(root.as_slice()) {
                found.push(Location {
                    root,
                    label: label(&name),
                });
            }
        }
    }
    found
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

fn readable(name: &str) -> Option<String> {
    let (kind, items) = name.split_once(':').filter(|(kind, _)| !kind.is_empty())?;
    let mut values = Vec::new();
    for item in items.split(',').filter(|item| !item.is_empty()) {
        let (key, value) = item.split_once('=')?;
        values.push((key, unescape(value)?));
    }
    let value = |key: &str| {
        values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    };
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
