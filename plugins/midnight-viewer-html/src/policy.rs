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
/// one with more; it is refused. Assets still resolve only below the document's own
/// folder: their relative URLs and refusals are the same as without these levels.
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
    pub fn new(generation: u64, network: bool, scripts: bool) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("document generation must be nonzero");
        }
        if scripts && !network {
            return Err("scripts require explicit network permission");
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
    #[cfg(any(feature = "native-cef", test))]
    pub(crate) fn document_folders() -> &'static [&'static str] {
        &DOCUMENT_FOLDERS
    }
    /// The relative URL (path and query) of a captured asset below the document's
    /// folder that the host resolves, or None when it must be denied locally: another
    /// folder than the document's, the main document itself, or a URL beyond the
    /// resolution budget.
    pub fn captured_path(&self, url: &str) -> Option<String> {
        let path = url.strip_prefix(&self.main_url())?;
        (!path.is_empty() && path.len() <= MAX_RESOURCE_URL_BYTES).then(|| path.to_owned())
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
            return if path == Some(main.as_str()) || (!self.network && url.starts_with(&main)) {
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
    pub fn csp(&self) -> &'static str {
        match (self.network, self.scripts) {
            (false, _) => {
                "default-src 'self' data:; style-src 'self' 'unsafe-inline' data:; script-src 'none'; connect-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'"
            }
            (true, false) => {
                "default-src https: http: data: 'unsafe-inline'; script-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'"
            }
            (true, true) => {
                "default-src https: http: data: 'unsafe-inline'; object-src 'none'; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'self'"
            }
        }
    }
}
