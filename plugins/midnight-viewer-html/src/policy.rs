//! Document-private origins and explicit request authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Captured,
    Network,
    Deny,
}
#[derive(Clone, Debug)]
pub struct DocumentPolicy {
    pub generation: u64,
    pub network: bool,
    pub scripts: bool,
}

/// First path segment of the synthetic document directory. Serving the document
/// below the origin root makes a `../` reference that climbs above the document
/// directory lose this prefix, so it is denied instead of being clamped by URL
/// resolution into a different, same-named file of the document directory.
const DOCUMENT_ROOT: &str = "mewv-document";

/// Synthetic folder levels between [`DOCUMENT_ROOT`] and the captured document.
///
/// Chromium resolves a link's `../` segments itself and stops at the origin's root, so
/// below a single folder `../../a.html` and `../../../a.html` would both become the same
/// URL, and the host could not be told which file a link names. Below 16 more folders,
/// up to 16 `../` segments stay apart and the provider reports them relative to the
/// document's folder (`links.rs`). A link with 17 or more `../` cannot be told apart from
/// one with more; it is refused. Assets of folders above the document's (generated
/// documentation keeps its stylesheets there) are requested the same way, as `../`
/// references the host resolves within the folders it reaches.
pub const SYNTHETIC_LEVELS: usize = 16;

/// The captured document's folders below its origin: [`DOCUMENT_ROOT`], then the
/// synthetic levels. Their names are distinct, so a real `../<name>/` path aliases a
/// document-folder path only when it repeats the exact tail of this chain.
const DOCUMENT_FOLDERS: [&str; 1 + SYNTHETIC_LEVELS] = [
    DOCUMENT_ROOT,
    "mewv-level-1",
    "mewv-level-2",
    "mewv-level-3",
    "mewv-level-4",
    "mewv-level-5",
    "mewv-level-6",
    "mewv-level-7",
    "mewv-level-8",
    "mewv-level-9",
    "mewv-level-10",
    "mewv-level-11",
    "mewv-level-12",
    "mewv-level-13",
    "mewv-level-14",
    "mewv-level-15",
    "mewv-level-16",
];

/// Longest captured relative URL (path and query) sent for host resolution; longer
/// URLs are denied locally instead of exceeding the protocol's control budget.
pub const MAX_RESOURCE_URL_BYTES: usize = 8 * 1024;

#[cfg(test)]
#[path = "policy_tests.rs"]
pub(crate) mod tests;

impl DocumentPolicy {
    /// Any policy of a nonzero generation. Offline scripts (web viewer API 0.4) run in a
    /// sandboxed document without network ([`Self::csp`]).
    pub fn new(generation: u64, network: bool, scripts: bool) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("document generation must be nonzero");
        }
        Ok(Self {
            generation,
            network,
            scripts,
        })
    }
    pub fn origin(&self) -> String {
        Self::origin_of(self.generation)
    }
    /// Synthetic document URL; relative captured assets resolve below it.
    pub fn main_url(&self) -> String {
        Self::main_url_of(self.generation)
    }
    /// [`Self::main_url`] of `generation`, for state created before its policy: the
    /// document's folder, [`SYNTHETIC_LEVELS`] below `DOCUMENT_ROOT`.
    pub fn main_url_of(generation: u64) -> String {
        format!(
            "{}/{}/",
            Self::origin_of(generation),
            DOCUMENT_FOLDERS.join("/")
        )
    }
    /// The URL `State` reports: the document directory without the synthetic levels,
    /// as it was before they existed. Display only; it never grants authority.
    pub fn display_url(&self) -> String {
        format!("{}/{DOCUMENT_ROOT}/", self.origin())
    }
    /// The captured document's folders below its origin, outermost first.
    pub(crate) fn document_folders() -> &'static [&'static str] {
        &DOCUMENT_FOLDERS
    }
    /// The relative URL (path and query) of a captured asset that the host resolves: below
    /// the document's folder, or above it within the synthetic levels as `../` segments
    /// (the host decides how far up it reaches). None when it must be denied locally:
    /// outside the document folders, the main document itself, or a URL beyond the
    /// resolution budget.
    pub fn captured_path(&self, url: &str) -> Option<String> {
        let path = match url.strip_prefix(&self.main_url()) {
            Some(path) => path.to_owned(),
            None => ancestor_path(url.strip_prefix(&format!("{}/", self.origin()))?)?,
        };
        (!path.is_empty() && path.len() <= MAX_RESOURCE_URL_BYTES).then_some(path)
    }
    fn origin_of(generation: u64) -> String {
        format!("https://mewv-{generation}.invalid")
    }
    pub fn access(&self, url: &str) -> Access {
        // CEF provides canonical URLs; compare the full authority boundary, never a prefix host.
        let prefix = format!("{}/", self.origin());
        if url.starts_with(&prefix) {
            let main = self.main_url();
            let path = url.split(['?', '#']).next();
            return if path == Some(main.as_str())
                || (!self.network && self.captured_path(url).is_some())
            {
                Access::Captured
            } else {
                Access::Deny
            };
        }
        if self.network && (url.starts_with("https://") || url.starts_with("http://")) {
            // A previous generation's private origin must never become a network request.
            let authority = url
                .split("//")
                .nth(1)
                .and_then(|s| s.split('/').next())
                .unwrap_or("");
            if !authority.contains(".invalid") && !authority.contains('@') && !authority.is_empty()
            {
                return Access::Network;
            }
        }
        Access::Deny
    }
    /// The document's Content-Security-Policy.
    ///
    /// Offline with scripts (web viewer API 0.4) the document is sandboxed without
    /// `allow-same-origin`: its origin is opaque, so its scripts cannot read any captured
    /// file (each asset is cross-origin to it, like a local file in a browser), while
    /// stylesheets, images, fonts and scripts still load and render. `connect-src 'none'`
    /// refuses fetch, XHR, WebSocket, EventSource, beacons and WebTransport, and no
    /// worker, frame, plugin or form runs. `allow-popups` only lets a `target=_blank` link
    /// reach the browser's popup check, which never opens a window (`links.rs`).
    pub fn csp(&self) -> String {
        let origin = self.origin();
        match (self.network, self.scripts) {
            (false, false) => {
                "default-src 'self' data:; style-src 'self' 'unsafe-inline' data:; script-src 'none'; connect-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'".into()
            }
            (false, true) => format!(
                "sandbox allow-scripts allow-popups; default-src {origin} data: blob:; style-src {origin} 'unsafe-inline' data: blob:; script-src {origin} 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; connect-src 'none'; object-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; manifest-src 'none'; form-action 'none'; base-uri {origin}"
            ),
            (true, false) => {
                "default-src https: http: data: 'unsafe-inline'; script-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'".into()
            }
            (true, true) => {
                "default-src https: http: data: 'unsafe-inline'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'".into()
            }
        }
    }

    /// The document is sandboxed with an opaque origin (offline scripts, [`Self::csp`]):
    /// fonts and module scripts, which a browser requests in CORS mode, need
    /// `Access-Control-Allow-Origin` to load. No other asset gets it, and no script can
    /// read any asset (`connect-src 'none'`).
    pub fn sandboxed(&self) -> bool {
        self.scripts && !self.network
    }
}

/// The `../` reference of an asset above the document's folder, from the URL's path and
/// query after its origin's `/`, or None outside the document folders. Each document
/// folder the URL leaves is one `../`; a URL that leaves `mewv-document` itself lies above
/// the levels the viewer tells apart. A fragment names no other file and is dropped.
fn ancestor_path(rest: &str) -> Option<String> {
    let rest = rest.split('#').next().unwrap_or_default();
    let end = rest.find('?').unwrap_or(rest.len());
    let (path, query) = rest.split_at(end);
    let mut segments: Vec<&str> = path.split('/').collect();
    let file = segments.pop()?;
    let shared = DOCUMENT_FOLDERS
        .iter()
        .zip(&segments)
        .take_while(|(folder, segment)| folder == segment)
        .count();
    if shared == 0 || shared == DOCUMENT_FOLDERS.len() || file.is_empty() {
        return None;
    }
    let mut target = "../".repeat(DOCUMENT_FOLDERS.len() - shared);
    for segment in &segments[shared..] {
        target.push_str(segment);
        target.push('/');
    }
    target.push_str(file);
    target.push_str(query);
    Some(target)
}
