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
impl DocumentPolicy {
    pub fn new(generation: u64, network: bool, scripts: bool) -> Result<Self, &'static str> {
        if generation == 0 || scripts && !network {
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
    pub fn main_url(&self) -> String {
        format!("{}/", self.origin())
    }
    pub fn access(&self, url: &str) -> Access {
        // CEF provides canonical URLs; compare the full authority boundary, never a prefix host.
        let prefix = format!("{}/", self.origin());
        if url.starts_with(&prefix) {
            return if !self.network
                || url.split(['?', '#']).next() == Some(self.main_url().as_str())
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
