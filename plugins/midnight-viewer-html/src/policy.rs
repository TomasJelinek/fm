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
        format!("https://mewv-{}.invalid", self.generation)
    }
    /// Synthetic document URL; relative captured assets resolve below it.
    pub fn main_url(&self) -> String {
        format!("{}/{DOCUMENT_ROOT}/", self.origin())
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
