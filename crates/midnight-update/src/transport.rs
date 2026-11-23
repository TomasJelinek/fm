//! One HTTP GET at a time, behind a trait so tests inject their own. The platform
//! transports (WinHTTP, rustls) never follow redirects themselves: `client` follows
//! them after checking every address against the `UrlPolicy`.
use std::{fmt, io, time::Duration};

/// The User-Agent of every request: the product and version, nothing about the user.
pub const USER_AGENT: &str = concat!("Midnight-Explorer/", env!("CARGO_PKG_VERSION"));

/// Connecting, sending and waiting for each answer chunk give up after these.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const RECEIVE_TIMEOUT: Duration = Duration::from_secs(30);

/// One GET request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub accept: &'static str,
    /// Ask for the body from this byte on (`Range: bytes=N-`); 0 asks for all of it.
    pub range_from: u64,
}

/// The status and the headers the update client reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    pub location: Option<String>,
    pub content_length: Option<u64>,
    /// The first byte of a 206 answer, from `Content-Range`.
    pub range_start: Option<u64>,
    /// `Retry-After` in seconds.
    pub retry_after: Option<u64>,
    /// `x-ratelimit-remaining` and `x-ratelimit-reset` (Unix seconds), GitHub's limit.
    pub rate_remaining: Option<u64>,
    pub rate_reset: Option<u64>,
}

impl Head {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308)
    }
}

/// Receives a 2xx answer: its head first, then the body in chunks. Returning an error
/// stops the transfer: the client uses that for size limits and cancellation.
pub trait Sink {
    fn begin(&mut self, head: &Head) -> io::Result<()>;
    fn write(&mut self, chunk: &[u8]) -> io::Result<()>;
}

/// Why a request got no answer.
#[derive(Debug)]
pub enum TransportError {
    /// No connection: offline, name not resolved or refused.
    Unreachable(String),
    TimedOut,
    /// TLS failed: certificate not trusted, protocol refused.
    Tls(String),
    /// The answer broke off or was not HTTP.
    Protocol(String),
    /// The body sink refused more data (limit, cancellation or disk).
    Sink(io::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(detail) => write!(f, "GitHub cannot be reached ({detail})."),
            Self::TimedOut => write!(f, "GitHub did not answer in time."),
            Self::Tls(detail) => write!(f, "The secure connection to GitHub failed ({detail})."),
            Self::Protocol(detail) => write!(f, "The connection to GitHub broke off ({detail})."),
            Self::Sink(error) => write!(f, "{error}"),
        }
    }
}

/// Sends one GET and streams a 2xx answer into `sink`. Other answers' bodies are
/// discarded. Must not follow redirects.
pub trait Transport: Send + Sync {
    fn get(&self, request: &Request, sink: &mut dyn Sink) -> Result<Head, TransportError>;
}

/// Which addresses requests may go to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlPolicy {
    /// HTTPS to GitHub's API and download hosts on port 443 only.
    GitHub,
    /// Plain HTTP to 127.0.0.1 only: local end-to-end tests in debug builds.
    LoopbackTest,
}

/// The hosts that serve GitHub's release API and downloads.
pub const GITHUB_HOSTS: [&str; 4] = [
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// An absolute URL split for the transports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl UrlPolicy {
    /// Accepts `url` when this policy allows it.
    pub fn check(&self, url: &str) -> Result<Target, String> {
        let target = split_url(url).ok_or_else(|| format!("Not a valid address: {url}"))?;
        match self {
            Self::GitHub => {
                if !target.secure {
                    return Err(format!("Refused an address without HTTPS: {url}"));
                }
                if target.port != 443 || !GITHUB_HOSTS.contains(&target.host.as_str()) {
                    return Err(format!("Refused an address outside GitHub: {url}"));
                }
            }
            Self::LoopbackTest => {
                if target.secure || target.host != "127.0.0.1" {
                    return Err(format!("Refused a test address outside 127.0.0.1: {url}"));
                }
            }
        }
        Ok(target)
    }
}

fn split_url(url: &str) -> Option<Target> {
    let (secure, rest) = match url.strip_prefix("https://") {
        Some(rest) => (true, rest),
        None => (false, url.strip_prefix("http://")?),
    };
    let (authority, path) = match rest.find(['/', '?']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty()
        || authority.contains('@')
        || path.contains('#')
        || path.chars().any(|c| c.is_control() || c == ' ')
    {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let path = if path.starts_with('?') {
        format!("/{path}")
    } else {
        path.to_owned()
    };
    Some(Target {
        secure,
        host: host.to_ascii_lowercase(),
        port,
        path,
    })
}

/// Resolves a redirect's `Location` against the address that answered.
pub fn resolve_location(base: &Target, location: &str) -> Option<String> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_owned());
    }
    if location.starts_with('/') && !location.starts_with("//") {
        let scheme = if base.secure { "https" } else { "http" };
        let default_port = if base.secure { 443 } else { 80 };
        let authority = if base.port == default_port {
            base.host.clone()
        } else {
            format!("{}:{}", base.host, base.port)
        };
        return Some(format!("{scheme}://{authority}{location}"));
    }
    None
}

/// Parses `bytes 100-199/200` from `Content-Range`.
pub fn parse_content_range(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes ")?;
    let (start, _) = range.split_once('-')?;
    start.trim().parse().ok()
}
