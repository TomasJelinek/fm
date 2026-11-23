use crate::api::{self, Resource, ResourceReply};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Host-only URL interpretation; no native pathname authority is created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourcePathPolicy {
    /// Compatibility behavior for captured HTML directory assets.
    #[default]
    RelativeAssets,
    /// Exact origin-root paths for generated documents. URL aliases are rejected.
    CanonicalRoot,
}

/// Independently observable resource-generation lifetime. Cancellation never waits for the actor.
#[derive(Clone, Debug, Default)]
pub struct ResourceCancellation(Arc<AtomicBool>);

impl ResourceCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Observe through an adapter token, e.g. Core Cancellation::observing. Never clear this flag.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Captured, host-owned directory/mount authority. Segments have already been decoded and
/// validated; implementations must enforce containment and revision checks on actual handles.
/// Neither display paths nor a viewer-generated temporary path can create this capability.
/// Segments start with at most [`Self::ancestor_levels`] `..` segments, each one folder
/// above the scope root, followed by at least one name.
pub trait Resources: Send + Sync + std::fmt::Debug {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String>;

    /// How many folders above its root this scope can capture from (`..` segments). None
    /// unless the scope says so.
    fn ancestor_levels(&self) -> usize {
        0
    }

    fn capture_with_cancellation(
        &self,
        segments: &[String],
        cancellation: &ResourceCancellation,
    ) -> Result<CapturedFile, String> {
        if cancellation.is_cancelled() {
            return Err("HTML resource request cancelled".into());
        }
        self.capture(segments)
    }

    /// Metadata-only policy query. Generated Office resources select CanonicalRoot.
    fn path_policy(&self) -> ResourcePathPolicy {
        ResourcePathPolicy::RelativeAssets
    }

    /// Consume on a backend worker. Errors/unwinds retain unconfirmed ownership.
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        std::mem::forget(self);
        Err("HTML resource scope teardown is unconfirmed".into())
    }
}

struct Entry {
    length: u64,
    segments: Vec<String>,
}

enum Pending {
    Resolve { id: u64, base: u64 },
    Read { id: u64 },
}

pub(crate) struct Store {
    main_length: u64,
    main_mime: &'static str,
    path_policy: ResourcePathPolicy,
    /// Folders above the scope root relative URLs may climb to (`Resources::ancestor_levels`).
    ancestors: usize,
    entries: HashMap<u64, Entry>,
    pending: HashMap<u64, Pending>,
    main: u64,
    next: u64,
    network: bool,
}

impl Store {
    pub fn new(
        main: &CapturedFile,
        path_policy: ResourcePathPolicy,
        ancestors: usize,
    ) -> Result<Self, String> {
        if path_policy == ResourcePathPolicy::CanonicalRoot && main.length() > 1024 * 1024 {
            return Err("Generated HTML exceeds its 1 MiB resource limit".into());
        }
        Ok(Self {
            main_length: main.length(),
            main_mime: if main
                .display_name()
                .rsplit('.')
                .next()
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case("xhtml"))
            {
                "application/xhtml+xml"
            } else {
                "text/html"
            },
            path_policy,
            ancestors: ancestors.min(MAX_ANCESTORS),
            entries: HashMap::new(),
            pending: HashMap::new(),
            main: 0,
            next: 0,
            network: false,
        })
    }

    pub fn reset(&mut self, network: bool) -> Result<Resource, String> {
        self.entries.clear();
        self.network = network;
        let id = self.next_id()?;
        self.main = id;
        self.entries.insert(
            id,
            Entry {
                length: self.main_length,
                segments: Vec::new(),
            },
        );
        Ok(Resource {
            id,
            length: self.main_length,
            mime: self.main_mime.into(),
        })
    }

    fn next_id(&mut self) -> Result<u64, String> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or("HTML resource IDs exhausted")?;
        Ok(self.next)
    }

    pub fn resolve(
        &mut self,
        request: u64,
        base: u64,
        url: &str,
        replacing: Option<u64>,
    ) -> Result<(u64, Vec<String>), String> {
        if self.network {
            return Err("Online mode has no local asset access".into());
        }
        let replacement_credit = replacing.is_some_and(|request| {
            matches!(self.pending.get(&request), Some(Pending::Resolve { .. }))
        }) as usize;
        if self.pending.len() - replacement_credit >= 64 || self.pending.contains_key(&request) {
            return Err("HTML resource request budget exhausted or duplicate request".into());
        }
        let resource_limit = if self.path_policy == ResourcePathPolicy::CanonicalRoot {
            4
        } else {
            2048
        };
        let retained_main = (self.path_policy == ResourcePathPolicy::CanonicalRoot
            && !self.entries.contains_key(&self.main)) as usize;
        if self.entries.len()
            + retained_main
            + self
                .pending
                .values()
                .filter(|pending| matches!(pending, Pending::Resolve { .. }))
                .count()
            - replacement_credit
            >= resource_limit
        {
            return Err("HTML resource budget exhausted".into());
        }
        let entry = self
            .entries
            .get(&base)
            .ok_or("Unknown HTML base resource")?;
        let segments = match self.path_policy {
            ResourcePathPolicy::RelativeAssets => {
                resolve_segments(&entry.segments, url, self.ancestors)?
            }
            ResourcePathPolicy::CanonicalRoot => canonical_segments(url)?,
        };
        let id = self.next_id()?;
        self.pending.insert(request, Pending::Resolve { id, base });
        Ok((id, segments))
    }

    pub fn read(
        &mut self,
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    ) -> Result<(), String> {
        api::validate_read(offset, length).map_err(|error| error.to_string())?;
        if self.pending.len() >= 64 || self.pending.contains_key(&request) {
            return Err("HTML resource request budget exhausted or duplicate request".into());
        }
        let entry = self
            .entries
            .get(&resource)
            .ok_or("Unknown or revoked HTML resource")?;
        if offset > entry.length {
            return Err("HTML read starts beyond EOF".into());
        }
        self.pending.insert(request, Pending::Read { id: resource });
        Ok(())
    }

    pub fn release(&mut self, resource: u64) -> bool {
        self.entries.remove(&resource).is_some()
    }

    pub fn deny(&mut self, request: u64, reason: &str) -> ResourceReply {
        self.pending.remove(&request);
        ResourceReply::Denied {
            request,
            reason: reason.into(),
        }
    }

    pub fn cancel_pending(&mut self) -> Vec<ResourceReply> {
        self.pending
            .drain()
            .map(|(request, _)| ResourceReply::Denied {
                request,
                reason: "HTML resource document was revoked".into(),
            })
            .collect()
    }

    pub fn accept_resolved(
        &mut self,
        request: u64,
        base: u64,
        resource: Resource,
        segments: Vec<String>,
    ) -> Result<ResourceReply, ResourceReply> {
        let pending = self.pending.remove(&request);
        if !matches!(pending, Some(Pending::Resolve { id, base: expected })
            if id == resource.id && expected == base)
            || !self.entries.contains_key(&base)
        {
            return Err(ResourceReply::Denied {
                request,
                reason: "HTML resource base was revoked".into(),
            });
        }
        self.entries.insert(
            resource.id,
            Entry {
                length: resource.length,
                segments,
            },
        );
        Ok(ResourceReply::Resolved { request, resource })
    }

    pub fn accept_read(
        &mut self,
        request: u64,
        resource: u64,
        offset: u64,
        bytes: Vec<u8>,
    ) -> ResourceReply {
        let pending = self.pending.remove(&request);
        if !matches!(pending, Some(Pending::Read { id }) if id == resource)
            || !self.entries.contains_key(&resource)
        {
            return ResourceReply::Denied {
                request,
                reason: "HTML resource was revoked".into(),
            };
        }
        ResourceReply::Data {
            request,
            resource,
            offset,
            bytes,
        }
    }
}

fn canonical_segments(url: &str) -> Result<Vec<String>, String> {
    let path = url.strip_prefix('/').unwrap_or(url);
    if path.is_empty()
        || path.len() > 64 * 1024
        || path.chars().any(|character| {
            character.is_control() || matches!(character, '\\' | ':' | '%' | '?' | '#')
        })
    {
        return Err("Invalid generated-document URL".into());
    }
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() > 256
        || segments.iter().any(|segment| {
            segment.is_empty()
                || *segment == "."
                || *segment == ".."
                || segment.ends_with(['.', ' '])
        })
    {
        return Err("Invalid generated-document path".into());
    }
    Ok(segments.into_iter().map(str::to_owned).collect())
}

/// Most folders above the document's folder a relative asset may come from: as many as
/// the bundled provider's synthetic levels tell apart.
pub(crate) const MAX_ANCESTORS: usize = 16;

/// The provider removes its own synthetic origin before requesting a relative URL.
/// Reject schemes, authority changes and traversal above the captured scope, even when
/// percent encoded. `..` resolves lexically, like a browser, within the scope and up to
/// `ancestors` folders above its root, which the result names with leading `..` segments.
fn resolve_segments(base: &[String], url: &str, ancestors: usize) -> Result<Vec<String>, String> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return Err("An HTML asset must use a relative URL".into());
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut input = path.bytes();
    while let Some(value) = input.next() {
        if value == b'%' {
            let digit = |value: Option<u8>| value.and_then(|b| char::from(b).to_digit(16));
            let high = digit(input.next()).ok_or("Malformed HTML URL escape")?;
            let low = digit(input.next()).ok_or("Malformed HTML URL escape")?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(value);
        }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| "HTML URL must be UTF-8")?;
    // A decoded `#` or `?` is part of a file name (`C# notes_files`), never a URL part
    // again: the names below are passed on literally. `%` stays refused, so nothing is
    // decoded twice.
    if decoded.starts_with('/')
        || decoded
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '%'))
    {
        return Err("Forbidden HTML resource URL".into());
    }
    let mut result = base
        .get(..base.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec();
    for segment in decoded.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            // `css/site.css` may name `../img/logo.png`. Above the scope root a parent
            // folder is one more leading `..`, as far as the scope reaches.
            if result.last().is_some_and(|last| last != "..") {
                result.pop();
            } else if result.len() < ancestors {
                result.push("..".into());
            } else {
                return Err(
                    "HTML resource traversal above the document folder is forbidden".into(),
                );
            }
            continue;
        }
        if segment.ends_with(['.', ' ']) {
            return Err("HTML resource traversal is forbidden".into());
        }
        result.push(segment.to_owned());
    }
    if result.last().is_none_or(|last| last == "..") {
        return Err("Empty HTML resource path".into());
    }
    Ok(result)
}

/// The MIME type of an asset, by its name's extension. Browsers saving a "complete" page
/// name scripts and other assets `<name>.<ext>.download`; the extension before
/// `.download` counts then.
pub(crate) fn mime_type(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let name = lower
        .strip_suffix(".download")
        .filter(|inner| inner.contains('.'))
        .unwrap_or(&lower);
    match name.rsplit('.').next().unwrap_or("") {
        "html" | "htm" | "shtml" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "css" => "text/css",
        "png" => "image/png",
        "apng" => "image/apng",
        "jpg" | "jpeg" | "jfif" | "pjpeg" | "pjp" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" | "cur" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "js" | "mjs" | "cjs" => "text/javascript",
        "json" | "map" => "application/json",
        "wasm" => "application/wasm",
        "xml" | "xsl" | "xslt" => "application/xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "vtt" => "text/vtt",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests;
