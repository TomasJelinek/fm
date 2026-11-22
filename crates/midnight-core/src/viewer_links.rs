//! Where a link in a viewed document leads: one policy for Markdown, HTML and Office
//! previews, decided before anything opens. It is pure: the caller looks at the file
//! system only for what a [`LinkTarget::Local`] names, and passes what it found to
//! [`local_link_action`].
//!
//! - `#name` stays in the document.
//! - A relative path, or a `file:` URL without a host, resolves against the document's
//!   folder, percent-decoded. It never names a device, a file stream or another computer.
//! - `http`, `https` and `mailto` go to the system's default handler ([`ExternalUrl`]).
//! - Anything else is refused, naming its scheme.
//!
//! A link never runs a program: a file that no viewer shows is revealed in its folder.
use crate::visible_name;
use std::path::{Component, Path, PathBuf};

#[cfg(test)]
#[path = "viewer_links_tests.rs"]
mod tests;

/// The longest link target the viewers follow, in UTF-8 bytes (the plugin API's bound).
/// An [`ExternalUrl`] stays within it after encoding, too.
pub const MAX_LINK_TARGET_BYTES: usize = 8192;
/// The status line shows at most this many characters of a link.
const SHOWN_CHARACTERS: usize = 300;
/// A refused scheme is named with at most this many characters.
const SHOWN_SCHEME: usize = 40;

/// Where a link leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A place in the same document: the fragment, percent-decoded. Empty for `#`, which
    /// is the top of the document.
    Fragment(String),
    /// A file or folder on this computer.
    Local(LocalLink),
    /// For the system's default handler.
    External(ExternalUrl),
    /// Not followed; [`LinkRefusal::message`] says why.
    Refused(LinkRefusal),
}

/// A link to a local file or folder, and the place to show in the document it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLink {
    pub path: LocalPath,
    /// The fragment for the document it opens, percent-decoded.
    pub fragment: Option<String>,
}

/// A local path. Each name is decoded, never empty, `.` or `..`, and contains no path
/// separator or NUL (nor, on Windows, a colon, a wildcard or a device name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalPath {
    /// `up` folders above the document's folder, then `names`.
    Relative { up: usize, names: Vec<String> },
    /// From the root of `drive` (Windows), or else of the document's own drive or file
    /// system.
    Rooted {
        drive: Option<char>,
        names: Vec<String>,
    },
}

impl LocalLink {
    /// The path this link names for a document in `folder`, an absolute folder. Going up
    /// stops at the root, as the file system does.
    pub fn resolve(&self, folder: &Path) -> PathBuf {
        let (mut path, names) = match &self.path {
            LocalPath::Relative { up, names } => {
                let mut path = folder.to_path_buf();
                for _ in 0..*up {
                    if !path.pop() {
                        break;
                    }
                }
                (path, names)
            }
            LocalPath::Rooted {
                drive: Some(drive),
                names,
            } => (PathBuf::from(format!("{drive}:\\")), names),
            LocalPath::Rooted { drive: None, names } => (root(folder), names),
        };
        path.extend(names);
        path
    }
}

/// The root of the drive or file system that holds `folder`.
fn root(folder: &Path) -> PathBuf {
    let mut root = PathBuf::new();
    for component in folder.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component),
            _ => break,
        }
    }
    if root.as_os_str().is_empty() {
        root.push(std::path::MAIN_SEPARATOR_STR);
    }
    root
}

/// Why a link is not followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkRefusal {
    Empty,
    TooLong,
    ControlCharacter,
    /// A scheme other than `http`, `https`, `mailto` and `file`, lowercase.
    Scheme(String),
    /// A file URL with a host, a network share, or a protocol-relative `//host` link.
    OtherComputer,
    /// A device or, on Windows, a file stream.
    Device,
    /// Not a valid path or address.
    Unreadable,
    /// A `.invalid` address, such as the HTML viewer's private one, which nothing reaches.
    PrivateAddress,
}

impl LinkRefusal {
    /// The status line's sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "This link has no target.".into(),
            Self::TooLong => "This link's target is longer than viewers follow.".into(),
            Self::ControlCharacter => {
                "This link's target contains control characters, so it is not followed.".into()
            }
            Self::Scheme(scheme) => format!("Links to {scheme}: addresses are not followed."),
            Self::OtherComputer => "Links to files on other computers are not followed.".into(),
            Self::Device => "Links to devices or file streams are not followed.".into(),
            Self::Unreadable => {
                "This link's target is not a valid path or address, so it is not followed.".into()
            }
            Self::PrivateAddress => {
                "This link points to an address that nothing can reach, so it is not followed."
                    .into()
            }
        }
    }
}

/// An `http`, `https` or `mailto` address for the system's default handler, as plain ASCII:
/// characters that could end a quoted argument, and those that are not ASCII, are
/// percent-encoded, and a mail link never carries file attachments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalUrl {
    text: String,
    mail: bool,
    attachments_removed: bool,
}

impl ExternalUrl {
    /// Checks the scheme, the length and the characters of `target`, and for a web address
    /// that it names a host. Mail header fields named `attach…` are removed, because some
    /// mail programs attach the local files named there.
    pub fn parse(target: &str) -> Result<Self, LinkRefusal> {
        check_text(target)?;
        let colon = target.find(':').ok_or(LinkRefusal::Unreadable)?;
        let scheme = target[..colon].to_ascii_lowercase();
        let rest = &target[colon + 1..];
        let (text, mail, attachments_removed) = match scheme.as_str() {
            "http" | "https" => {
                let host = web_host(rest).ok_or(LinkRefusal::Unreadable)?;
                if host == "invalid" || host.ends_with(".invalid") {
                    return Err(LinkRefusal::PrivateAddress);
                }
                (format!("{scheme}:{}", encode(rest)), false, false)
            }
            "mailto" => {
                let (to, fields) = rest.split_once('?').unwrap_or((rest, ""));
                let mut removed = false;
                let kept: Vec<&str> = fields
                    .split('&')
                    .filter(|field| !field.is_empty())
                    .filter(|field| {
                        let name = field.split('=').next().unwrap_or_default();
                        let name = String::from_utf8_lossy(&percent_decode(name)).to_lowercase();
                        let attachment = name.trim().starts_with("attach");
                        removed |= attachment;
                        !attachment
                    })
                    .collect();
                let mut text = format!("mailto:{}", encode(to));
                if !kept.is_empty() {
                    text.push('?');
                    text.push_str(&encode(&kept.join("&")));
                }
                (text, true, removed)
            }
            _ => return Err(LinkRefusal::Scheme(shown_scheme(&scheme))),
        };
        if text.len() > MAX_LINK_TARGET_BYTES {
            return Err(LinkRefusal::TooLong);
        }
        Ok(Self {
            text,
            mail,
            attachments_removed,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// A `mailto` address, for the mail program; otherwise a web page, for the browser.
    pub fn is_mail(&self) -> bool {
        self.mail
    }

    /// Whether file attachment fields were removed from a mail link.
    pub fn attachments_removed(&self) -> bool {
        self.attachments_removed
    }
}

/// The host of `//authority/…`, lowercase, without user information and port.
fn web_host(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix("//")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if host_port.starts_with('[') {
        &host_port[..=host_port.find(']')?]
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// `text` with every character outside RFC 3986's unreserved and reserved ASCII sets
/// percent-encoded as UTF-8, and each `%` that does not start an escape as `%25`.
fn encode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut encoded = String::with_capacity(text.len());
    for (index, &byte) in bytes.iter().enumerate() {
        let escape = byte == b'%'
            && bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit);
        if escape || (byte != b'%' && allowed(byte)) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn allowed(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=".contains(&byte)
}

/// The scheme as a refusal names it.
fn shown_scheme(scheme: &str) -> String {
    if scheme.chars().count() <= SHOWN_SCHEME {
        return scheme.to_owned();
    }
    let mut shown: String = scheme.chars().take(SHOWN_SCHEME - 1).collect();
    shown.push('…');
    shown
}

/// The refusal for an empty, oversized or control-character target.
fn check_text(target: &str) -> Result<(), LinkRefusal> {
    if target.is_empty() {
        Err(LinkRefusal::Empty)
    } else if target.len() > MAX_LINK_TARGET_BYTES {
        Err(LinkRefusal::TooLong)
    } else if target.chars().any(char::is_control) {
        Err(LinkRefusal::ControlCharacter)
    } else {
        Ok(())
    }
}

/// Where `target`, a link's target as the document states it, leads.
pub fn classify_link(target: &str) -> LinkTarget {
    if let Err(refusal) = check_text(target) {
        return LinkTarget::Refused(refusal);
    }
    if let Some(fragment) = target.strip_prefix('#') {
        return LinkTarget::Fragment(decode_text(fragment));
    }
    match scheme(target) {
        Some(scheme) => with_scheme(target, scheme),
        None => local(target),
    }
}

/// The scheme of `target`: the RFC 3986 name before its first colon.
fn scheme(target: &str) -> Option<&str> {
    let name = &target[..target.find(':')?];
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then_some(name)
}

fn with_scheme(target: &str, scheme: &str) -> LinkTarget {
    let rest = &target[scheme.len() + 1..];
    if cfg!(windows) && scheme.len() == 1 {
        // A drive: `C:\x` or `C:/x`. `C:x` is relative to that drive's current folder,
        // which a document cannot know.
        if !rest.starts_with(['/', '\\']) {
            return LinkTarget::Refused(LinkRefusal::Unreadable);
        }
        let drive = scheme.chars().next().map(|c| c.to_ascii_uppercase());
        let (path, fragment) = split_reference(rest);
        return rooted_link(drive, path, fragment);
    }
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "mailto" => match ExternalUrl::parse(target) {
            Ok(url) => LinkTarget::External(url),
            Err(refusal) => LinkTarget::Refused(refusal),
        },
        "file" => file_url(rest),
        other => LinkTarget::Refused(LinkRefusal::Scheme(shown_scheme(other))),
    }
}

/// `file:` URLs name a local path: `file:///path` or `file:/path`. One with a host, even
/// `localhost`, is refused.
fn file_url(rest: &str) -> LinkTarget {
    let (rest, fragment) = split_reference(rest);
    let path = match rest.strip_prefix("//") {
        Some(after) => {
            let (host, path) = after.split_at(after.find('/').unwrap_or(after.len()));
            if !host.is_empty() || path.starts_with("//") {
                return LinkTarget::Refused(LinkRefusal::OtherComputer);
            }
            path
        }
        None => rest,
    };
    let Some(path) = path.strip_prefix('/') else {
        return LinkTarget::Refused(LinkRefusal::Unreadable);
    };
    // `file:///C:/x` names drive C.
    let bytes = path.as_bytes();
    if cfg!(windows)
        && bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes.get(2).is_none_or(|separator| *separator == b'/')
    {
        let drive = Some(char::from(bytes[0].to_ascii_uppercase()));
        return rooted_link(drive, &path[2..], fragment);
    }
    rooted_link(None, path, fragment)
}

/// A link without a scheme: a relative path, or one from the root.
fn local(target: &str) -> LinkTarget {
    // Before the query is split off: on Windows `\\?\` opens the device namespace.
    let mut chars = target.chars();
    let (first, second) = (chars.next(), chars.next());
    if first.is_some_and(separator) && second.is_some_and(separator) {
        // `//host/x` and `\\server\share` name another computer; on Windows `\\.\` and
        // `\\?\` open the device namespace.
        let device = cfg!(windows)
            && matches!(chars.next(), Some('.' | '?'))
            && chars.next().is_none_or(separator);
        let refusal = if device {
            LinkRefusal::Device
        } else {
            LinkRefusal::OtherComputer
        };
        return LinkTarget::Refused(refusal);
    }
    let (path, fragment) = split_reference(target);
    if path.starts_with(separator) {
        return rooted_link(None, &path[1..], fragment);
    }
    match parse_names(path, true) {
        Ok((up, names)) => LinkTarget::Local(LocalLink {
            path: LocalPath::Relative { up, names },
            fragment,
        }),
        Err(refusal) => LinkTarget::Refused(refusal),
    }
}

/// A path separator: `/`, and on Windows also `\`.
fn separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// A link to `path` from the root of `drive`, or of the document's own drive.
fn rooted_link(drive: Option<char>, path: &str, fragment: Option<String>) -> LinkTarget {
    match parse_names(path, false) {
        Ok((_, names)) => LinkTarget::Local(LocalLink {
            path: LocalPath::Rooted { drive, names },
            fragment,
        }),
        Err(refusal) => LinkTarget::Refused(refusal),
    }
}

/// The decoded names of `path`, and for a `relative` one how many folders it first goes
/// up. Empty and `.` segments are skipped; `..` removes the name before it, and above a
/// root there is nothing to remove.
fn parse_names(path: &str, relative: bool) -> Result<(usize, Vec<String>), LinkRefusal> {
    let mut up = 0;
    let mut names: Vec<String> = Vec::new();
    for segment in path.split(separator) {
        let name = decode_name(segment).ok_or(LinkRefusal::Unreadable)?;
        match name.as_str() {
            "" | "." => {}
            ".." => {
                if names.pop().is_none() && relative {
                    up += 1;
                }
            }
            _ => {
                check_name(&name)?;
                names.push(name);
            }
        }
    }
    Ok((up, names))
}

/// One percent-decoded path segment, or None when it is not UTF-8 or contains a path
/// separator or NUL once decoded.
fn decode_name(segment: &str) -> Option<String> {
    let name = String::from_utf8(percent_decode(segment)).ok()?;
    (!name.contains(separator) && !name.contains('\0')).then_some(name)
}

/// On Windows, names that open a device or a file stream, or that no file can have.
fn check_name(name: &str) -> Result<(), LinkRefusal> {
    if !cfg!(windows) {
        return Ok(());
    }
    if name.contains(':') {
        return Err(LinkRefusal::Device);
    }
    if name.contains(['<', '>', '"', '|', '?', '*']) {
        return Err(LinkRefusal::Unreadable);
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            matches!(
                rest,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    };
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT");
    if device {
        Err(LinkRefusal::Device)
    } else {
        Ok(())
    }
}

/// The path of a reference, and its fragment. The query is dropped: a local file has
/// none.
fn split_reference(target: &str) -> (&str, Option<String>) {
    let (rest, fragment) = match target.split_once('#') {
        Some((rest, fragment)) => (rest, Some(decode_text(fragment))),
        None => (target, None),
    };
    let path = rest.split_once('?').map_or(rest, |(path, _)| path);
    (path, fragment)
}

/// Text with its valid `%XX` escapes decoded, or as it is when the result is not UTF-8.
fn decode_text(text: &str) -> String {
    String::from_utf8(percent_decode(text)).unwrap_or_else(|_| text.to_owned())
}

/// The bytes of `text` with each `%XX` escape decoded; a `%` that does not start one
/// stays.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |offset: usize| {
            bytes
                .get(index + offset)
                .and_then(|byte| char::from(*byte).to_digit(16))
        };
        match (bytes[index], hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                // Two hex digits make one byte.
                decoded.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    decoded
}

/// What the file system says about a local target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalTargetKind {
    Missing,
    Folder,
    /// A regular file, and whether a viewer can show it by its own plugin or provider,
    /// not only as text or bytes.
    File {
        viewable: bool,
    },
    /// A device, pipe, socket or anything else that is neither a file nor a folder.
    Other,
}

/// What following a link does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Scroll to the fragment in the same document.
    ScrollTo(String),
    /// Open the file in the same viewer window, at the fragment.
    OpenInViewer {
        path: PathBuf,
        fragment: Option<String>,
    },
    /// Open the folder in the active panel.
    OpenFolder(PathBuf),
    /// Show the file in the active panel, with the cursor on it.
    Reveal(PathBuf),
    /// Hand the address to the system's default handler.
    OpenExternal(ExternalUrl),
    /// Do nothing but say why.
    Refuse(String),
}

/// What following a link to the local `path` does, given what the file system says about
/// it. A file no viewer shows is revealed in its folder, never opened by its program.
pub fn local_link_action(
    path: PathBuf,
    fragment: Option<String>,
    kind: LocalTargetKind,
) -> LinkAction {
    match kind {
        LocalTargetKind::Folder => LinkAction::OpenFolder(path),
        LocalTargetKind::File { viewable: true } => LinkAction::OpenInViewer { path, fragment },
        LocalTargetKind::File { viewable: false } => LinkAction::Reveal(path),
        LocalTargetKind::Missing => LinkAction::Refuse(format!(
            "{} does not exist.",
            link_status_text(&path.to_string_lossy())
        )),
        LocalTargetKind::Other => LinkAction::Refuse(format!(
            "{} is not a file or folder, so the link is not followed.",
            link_status_text(&path.to_string_lossy())
        )),
    }
}

/// What the status line says while the pointer or the keyboard focus is on a link to
/// `target` in a document in `folder` (None when the document has no folder on disk):
/// where it really leads and, for a refused one, why it is not followed.
pub fn describe_link(target: &str, folder: Option<&Path>) -> String {
    let described = match classify_link(target) {
        LinkTarget::Fragment(name) if name.is_empty() => "Link to the top of this document".into(),
        LinkTarget::Fragment(name) => {
            format!("Link to #{} in this document", link_status_text(&name))
        }
        LinkTarget::Local(link) => match folder {
            Some(folder) => format!(
                "Link to {}",
                link_status_text(&link.resolve(folder).to_string_lossy())
            ),
            None => format!("Link to {}", link_status_text(target)),
        },
        LinkTarget::External(url) => format!("Link to {}", url.as_str()),
        LinkTarget::Refused(refusal) => {
            format!(
                "Link to {} · {}",
                link_status_text(target),
                refusal.message()
            )
        }
    };
    bounded(described)
}

/// At most [`SHOWN_CHARACTERS`], ending in an ellipsis when cut.
fn bounded(text: String) -> String {
    if text.chars().count() <= SHOWN_CHARACTERS {
        return text;
    }
    let mut cut: String = text.chars().take(SHOWN_CHARACTERS - 1).collect();
    cut.push('…');
    cut
}

/// Text of a link for the status line: bidirectional controls escaped, other controls,
/// such as percent-decoded ones, replaced.
pub fn link_status_text(text: &str) -> String {
    visible_name(text)
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}
