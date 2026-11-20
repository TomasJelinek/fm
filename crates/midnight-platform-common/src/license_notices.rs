//! Bounded reads of the license notices the About dialog shows.
//!
//! Release builds read only the `licenses` folder and `THIRD-PARTY.json` that a
//! release package keeps beside the executable; debug builds may also find a
//! crate in a developer's Cargo registry. Only real folders and regular files are
//! read, never links, and every read is capped before it starts. Call on a
//! backend worker.
use midnight_core::{LicenseNotice, LicenseNoticeRequest};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Notice text one detail holds at most. The UI lays it out as one text block;
/// Apache plus MIT texts take about 12 KiB.
pub const MAX_NOTICE_TEXT: usize = 128 * 1024;
/// Bytes read from one notice file at most.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Notice files one detail reads at most.
const MAX_FILES: usize = 48;
/// Folder levels searched below one library's notice folder.
const MAX_DEPTH: usize = 4;
/// Folder entries examined per folder.
const MAX_ENTRIES: usize = 256;
const MAX_INVENTORY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_AUTHORS: usize = 24;
const MAX_REGISTRY_INDEXES: usize = 16;

/// Where notices may be found.
#[derive(Clone, Debug, Default)]
pub struct NoticeSources {
    /// The release package's `licenses` folder.
    pub installed: Option<PathBuf>,
    /// The release package's `THIRD-PARTY.json`.
    pub inventory: Option<PathBuf>,
    /// Cargo registry `src` folders. Only debug builds use any.
    pub registries: Vec<PathBuf>,
}

impl NoticeSources {
    /// The release files beside the running executable. Debug builds also look in
    /// the developer's Cargo registries; release builds never do.
    pub fn beside_executable() -> Self {
        let directory = std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf));
        Self::beside(directory.as_deref(), cfg!(debug_assertions))
    }

    fn beside(directory: Option<&Path>, developer: bool) -> Self {
        Self {
            installed: directory.map(|path| path.join("licenses")),
            inventory: directory.map(|path| path.join("THIRD-PARTY.json")),
            registries: if developer {
                developer_registries(directory)
            } else {
                Vec::new()
            },
        }
    }
}

/// `CARGO_HOME`, the user's Cargo home and the project's container Cargo home
/// beside a development build's output folder.
fn developer_registries(executable_directory: Option<&Path>) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        homes.push(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        homes.push(PathBuf::from(home).join(".cargo"));
    }
    homes.extend(
        executable_directory
            .into_iter()
            .flat_map(Path::ancestors)
            .take(4)
            .map(|directory| directory.join(".tools").join("docker").join("cargo")),
    );
    homes
        .into_iter()
        .map(|home| home.join("registry").join("src"))
        .collect()
}

/// Package authors recorded in a release package's `THIRD-PARTY.json`, parsed once.
#[derive(Debug, Default)]
pub struct AuthorIndex(HashMap<(String, String), Vec<String>>);

impl AuthorIndex {
    /// An unreadable, oversized or malformed inventory gives an empty index.
    pub fn load(path: Option<&Path>) -> Self {
        match path.map(|path| read_regular(path, MAX_INVENTORY_BYTES)) {
            Some(Ok((bytes, false))) => Self::parse(&bytes),
            _ => Self::default(),
        }
    }

    fn parse(bytes: &[u8]) -> Self {
        #[derive(serde::Deserialize)]
        struct Record {
            name: String,
            version: String,
            #[serde(default)]
            authors: Option<Vec<String>>,
        }
        let records: Vec<Record> = serde_json::from_slice(bytes).unwrap_or_default();
        Self(
            records
                .into_iter()
                .filter_map(|record| {
                    let authors = record.authors.filter(|authors| !authors.is_empty())?;
                    Some(((record.name, record.version), authors))
                })
                .collect(),
        )
    }

    fn authors(&self, name: &str, version: &str) -> Vec<String> {
        self.0
            .get(&(name.to_owned(), version.to_owned()))
            .map(|authors| bounded_authors(authors.iter().map(String::as_str)))
            .unwrap_or_default()
    }
}

/// The notices of one library. Folder names are validated as single path
/// components; nothing outside the sources is ever opened.
pub fn read_notice(
    sources: &NoticeSources,
    index: &AuthorIndex,
    request: &LicenseNoticeRequest,
) -> LicenseNotice {
    let mut notice = LicenseNotice {
        id: request.id,
        ..LicenseNotice::default()
    };
    let Some((folder, packaged)) = locate(sources, request) else {
        return notice;
    };
    let mut files = Vec::new();
    if packaged {
        // A release package's folder holds only the copied notices.
        collect(&folder, 0, &mut files, &|_| true);
    } else {
        collect(&folder, 0, &mut files, &is_notice);
    }
    files.sort();
    notice.truncated = files.len() > MAX_FILES;
    for path in files.into_iter().take(MAX_FILES) {
        let remaining = MAX_NOTICE_TEXT.saturating_sub(notice.text.len());
        if remaining == 0 {
            notice.truncated = true;
            break;
        }
        let Ok((bytes, cut)) = read_regular(&path, (remaining as u64).min(MAX_FILE_BYTES)) else {
            continue;
        };
        notice.truncated |= cut;
        let content = String::from_utf8_lossy(&bytes);
        let html = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("html"));
        let content = if html {
            html_text(&content)
        } else {
            content.into_owned()
        };
        if !notice.text.is_empty() {
            notice.text.push_str("\n\n");
        }
        let relative = path.strip_prefix(&folder).unwrap_or(&path);
        notice
            .text
            .push_str(&format!("──── {} ────\n\n", relative.display()));
        notice.text.push_str(content.trim_end());
    }
    let manifest = if packaged {
        index.authors(&request.name, &request.version)
    } else {
        manifest_authors(&folder.join("Cargo.toml"))
    };
    notice.authors = if manifest.is_empty() {
        copyright_lines(&notice.text)
    } else {
        manifest
    };
    notice
}

/// One path component of a crate name or version: no separators, `.` or `..`.
fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

/// The requested folder relative to a source root, if every component is valid.
fn relative_folder(request: &LicenseNoticeRequest) -> Option<PathBuf> {
    match &request.folder {
        None => (component(&request.name) && component(&request.version))
            .then(|| PathBuf::from(format!("{}-{}", request.name, request.version))),
        Some(folder) => {
            let parts: Vec<&str> = folder.split('/').collect();
            (parts.len() <= 2 && parts.iter().all(|part| component(part)))
                .then(|| parts.iter().collect())
        }
    }
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// The library's folder and whether it is a release package's notice folder.
fn locate(sources: &NoticeSources, request: &LicenseNoticeRequest) -> Option<(PathBuf, bool)> {
    let relative = relative_folder(request)?;
    if let Some(installed) = sources
        .installed
        .as_deref()
        .filter(|path| real_directory(path))
    {
        let folder = installed.join(&relative);
        if real_directory(&folder) {
            return Some((folder, true));
        }
    }
    if request.folder.is_some() {
        return None;
    }
    for registry in &sources.registries {
        let Ok(entries) = fs::read_dir(registry) else {
            continue;
        };
        for entry in entries.flatten().take(MAX_REGISTRY_INDEXES) {
            let folder = entry.path().join(&relative);
            if real_directory(&folder) {
                return Some((folder, false));
            }
        }
    }
    None
}

/// Regular files below `folder` whose top-level entry `accept`s, without following links.
fn collect(folder: &Path, depth: usize, files: &mut Vec<PathBuf>, accept: &dyn Fn(&str) -> bool) {
    if depth > MAX_DEPTH || files.len() > MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten().take(MAX_ENTRIES) {
        if files.len() > MAX_FILES {
            return;
        }
        if depth == 0 && !accept(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            files.push(entry.path());
        } else if kind.is_dir() {
            collect(&entry.path(), depth + 1, files, &|_| true);
        }
    }
}

/// Notice names in a crate's sources, as the release notice collector copies them.
fn is_notice(name: &str) -> bool {
    let name = name.to_lowercase();
    ["license", "licence", "copying", "copyright", "notice"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// At most `limit` bytes of a regular file, and whether more followed.
fn read_regular(path: &Path, limit: u64) -> io::Result<(Vec<u8>, bool)> {
    let not_regular = || io::Error::new(io::ErrorKind::InvalidInput, "not a regular file");
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(not_regular());
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(not_regular());
    }
    let mut bytes = Vec::with_capacity(metadata.len().min(limit) as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    let cut = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    Ok((bytes, cut))
}

/// `authors = ["A <a@b>", ...]` of a crate's `[package]` table, without a TOML parser.
fn manifest_authors(path: &Path) -> Vec<String> {
    let Ok((bytes, false)) = read_regular(path, MAX_MANIFEST_BYTES) else {
        return Vec::new();
    };
    let manifest = String::from_utf8_lossy(&bytes);
    let mut package = false;
    let mut list = None;
    for (offset, line) in line_offsets(&manifest) {
        let line = line.trim();
        if line.starts_with('[') {
            package = line == "[package]";
        } else if package
            && let Some(value) = line.strip_prefix("authors")
            && value.trim_start().starts_with('=')
        {
            list = manifest[offset..].find('[').map(|open| offset + open + 1);
            break;
        }
    }
    let Some(start) = list else {
        return Vec::new();
    };
    let mut authors = Vec::new();
    let mut chars = manifest[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            ']' => break,
            '"' => {
                let mut author = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => author.extend(chars.next()),
                        c => author.push(c),
                    }
                }
                authors.push(author);
            }
            _ => {}
        }
    }
    bounded_authors(authors.iter().map(String::as_str))
}

fn line_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    })
}

/// At most `MAX_AUTHORS` authors with single spaces between words. An author
/// that differs from an earlier one only in spacing or case, as one statement
/// copied into several notice files can, is listed once.
fn bounded_authors<S: AsRef<str>>(authors: impl IntoIterator<Item = S>) -> Vec<String> {
    let mut bounded: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for author in authors {
        let words: Vec<&str> = author.as_ref().split_whitespace().collect();
        let author: String = words.join(" ").chars().take(200).collect();
        let key = author.to_lowercase();
        if !author.is_empty() && !seen.contains(&key) {
            bounded.push(author);
            seen.push(key);
        }
        if bounded.len() == MAX_AUTHORS {
            break;
        }
    }
    bounded
}

/// The copyright statements of notice text, in order and without repeats.
fn copyright_lines(text: &str) -> Vec<String> {
    bounded_authors(text.lines().filter_map(copyright_line))
}

/// A concrete copyright statement: it starts with "Copyright" or © and names a
/// year or carries a © or (c) mark, or it starts with "(c)" and names a year (so
/// that Apache's list item "(c) You must retain" is no author). It is no
/// template such as "Copyright [yyyy] [name of copyright owner]" or the Open
/// Font License's "Copyright (c) <dates>, <Copyright Holder>", and no license
/// prose such as Apache's "copyright license to reproduce". The GNU licenses'
/// own notice names the license's author, not the library's.
fn copyright_line(line: &str) -> Option<&str> {
    let line = line
        .trim()
        .trim_start_matches(['/', '*', '#', ';', '!', '-'])
        .trim();
    let lower = line.to_lowercase();
    let statement = lower.starts_with("copyright")
        || line.starts_with('©')
        || (lower.starts_with("(c)") && has_year(line));
    if !statement {
        return None;
    }
    let marked = line.contains('©') || lower.contains("(c)") || has_year(line);
    let template = [
        "yyyy",
        "<year>",
        "[year]",
        "{year}",
        "<dates>",
        "name of copyright owner",
        "holder>",
        "holders>",
        "<name of author>",
        "[fullname]",
        "<owner>",
        "<author>",
    ]
    .iter()
    .any(|placeholder| lower.contains(placeholder));
    let license_author = lower.contains("free software foundation");
    (marked && !template && !license_author).then_some(line)
}

fn has_year(line: &str) -> bool {
    line.as_bytes().windows(4).any(|window| {
        window.iter().all(u8::is_ascii_digit) && matches!(&window[..2], b"19" | b"20")
    })
}

/// Readable text of a notice page: tags removed, blocks on their own lines and
/// the common character references decoded.
fn html_text(html: &str) -> String {
    const BLOCKS: [&str; 15] = [
        "p", "br", "li", "ul", "ol", "tr", "div", "pre", "table", "title", "h1", "h2", "h3", "h4",
        "h5",
    ];
    let mut text = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close]
            .trim_start_matches('/')
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if BLOCKS.contains(&tag.as_str()) {
            text.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    let mut readable = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        blank = if line.trim().is_empty() { blank + 1 } else { 0 };
        if blank <= 1 {
            readable.push_str(line);
            readable.push('\n');
        }
    }
    readable
}

#[cfg(test)]
#[path = "license_notices_tests.rs"]
mod tests;
