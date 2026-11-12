use crate::api::{self, Resource, ResourceReply};
use midnight_plugin_host::CapturedFile;
use std::{collections::HashMap, sync::Arc};

/// Captured, host-owned directory/mount authority. Segments have already been decoded and
/// validated; implementations must enforce containment and revision checks on actual handles.
/// Neither display paths nor a viewer-generated temporary path can create this capability.
pub trait Resources: Send + Sync + std::fmt::Debug {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String>;
}

struct Entry {
    file: CapturedFile,
    segments: Vec<String>,
}

pub(crate) struct Store {
    main: CapturedFile,
    scope: Option<Arc<dyn Resources>>,
    entries: HashMap<u64, Entry>,
    next: u64,
    network: bool,
}

impl Store {
    pub fn new(main: CapturedFile, scope: Option<Arc<dyn Resources>>) -> Self {
        Self {
            main,
            scope,
            entries: HashMap::new(),
            next: 0,
            network: false,
        }
    }

    pub fn reset(&mut self, network: bool) -> Result<Resource, String> {
        self.entries.clear();
        self.network = network;
        let mime = if self
            .main
            .display_name()
            .rsplit('.')
            .next()
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("xhtml"))
        {
            "application/xhtml+xml"
        } else {
            "text/html"
        };
        self.insert(self.main.clone(), Vec::new(), mime)
    }

    fn insert(
        &mut self,
        file: CapturedFile,
        segments: Vec<String>,
        mime: &str,
    ) -> Result<Resource, String> {
        if self.entries.len() >= 2048 {
            return Err("HTML resource budget exhausted".into());
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("HTML resource IDs exhausted")?;
        let resource = Resource {
            id: self.next,
            length: file.length(),
            mime: mime.into(),
        };
        self.entries.insert(self.next, Entry { file, segments });
        Ok(resource)
    }

    pub fn resolve(&mut self, request: u64, base: u64, url: &str) -> ResourceReply {
        let result = (|| {
            if self.network {
                return Err("Online mode has no local asset access".into());
            }
            let base = self
                .entries
                .get(&base)
                .ok_or("Unknown HTML base resource")?;
            base.file.revalidate().map_err(|e| e.to_string())?;
            let segments = resolve_segments(&base.segments, url)?;
            let scope = self
                .scope
                .as_ref()
                .ok_or("This source has no relative resource scope")?;
            let file = scope.capture(&segments)?;
            let mime = mime_type(segments.last().map(String::as_str).unwrap_or(""));
            self.insert(file, segments, mime)
        })();
        match result {
            Ok(resource) => ResourceReply::Resolved { request, resource },
            Err(reason) => ResourceReply::Denied { request, reason },
        }
    }

    pub fn read(&self, request: u64, resource: u64, offset: u64, length: u32) -> ResourceReply {
        let result = (|| {
            api::validate_read(offset, length).map_err(|e| e.to_string())?;
            let entry = self
                .entries
                .get(&resource)
                .ok_or("Unknown or revoked HTML resource")?;
            if offset > entry.file.length() {
                return Err("HTML read starts beyond EOF".into());
            }
            entry
                .file
                .read_at(offset, length)
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(bytes) => ResourceReply::Data {
                request,
                resource,
                offset,
                bytes,
            },
            Err(reason) => ResourceReply::Denied { request, reason },
        }
    }

    pub fn release(&mut self, resource: u64) -> ResourceReply {
        self.entries.remove(&resource);
        ResourceReply::Released { resource }
    }
}

/// The provider removes its own synthetic origin before requesting a relative URL.
/// Reject schemes, authority changes and traversal even when percent encoded.
fn resolve_segments(base: &[String], url: &str) -> Result<Vec<String>, String> {
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
    if decoded.starts_with('/')
        || decoded
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '%' | '?' | '#'))
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
        if segment == ".." || segment.ends_with(['.', ' ']) {
            return Err("HTML resource traversal is forbidden".into());
        }
        result.push(segment.to_owned());
    }
    if result.is_empty() {
        return Err("Empty HTML resource path".into());
    }
    Ok(result)
}

fn mime_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "css" => "text/css",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "js" | "mjs" => "text/javascript",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests;
