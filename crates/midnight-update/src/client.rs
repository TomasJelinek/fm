//! The GitHub release client: the release list, small files and resumable downloads,
//! over any `Transport`, with every address checked by the `UrlPolicy`.
use crate::{
    release::{self, LISTED_RELEASES, ListingError, Release},
    transport::{Head, Request, Sink, Transport, TransportError, UrlPolicy, resolve_location},
};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// The repository whose releases are offered.
pub const REPOSITORY: &str = "TomasJelinek/fm";
/// The most bytes of the release list that are read.
pub const MAX_LISTING_BYTES: u64 = 4 * 1024 * 1024;
/// No release archive is larger than this.
pub const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
/// A download that breaks off is resumed this many times before it fails.
const DOWNLOAD_ATTEMPTS: usize = 4;
/// Without a reset time, a rate-limited client waits this long.
const DEFAULT_RATE_WAIT_SECONDS: u64 = 60 * 60;

/// Where the client asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub releases_url: String,
    pub policy: UrlPolicy,
}

impl Endpoints {
    /// GitHub's API over HTTPS.
    pub fn github() -> Self {
        Self {
            releases_url: format!(
                "https://api.github.com/repos/{REPOSITORY}/releases?per_page={LISTED_RELEASES}"
            ),
            policy: UrlPolicy::GitHub,
        }
    }

    /// A local test server at `http://127.0.0.1:PORT` serving `/releases`. Debug builds
    /// only: a release build returns `None`, so it can never be pointed at plain HTTP.
    pub fn loopback_test(base: &str) -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        let base = base.trim_end_matches('/');
        let endpoints = Self {
            releases_url: format!("{base}/releases"),
            policy: UrlPolicy::LoopbackTest,
        };
        endpoints
            .policy
            .check(&endpoints.releases_url)
            .ok()
            .map(|_| endpoints)
    }

    /// GitHub, or in debug builds the server that `MIDNIGHT_UPDATE_TEST_SERVER` names.
    pub fn from_environment() -> Self {
        std::env::var("MIDNIGHT_UPDATE_TEST_SERVER")
            .ok()
            .and_then(|base| Self::loopback_test(&base))
            .unwrap_or_else(Self::github)
    }
}

/// Why a request failed.
#[derive(Debug)]
pub enum FetchError {
    Transport(TransportError),
    /// GitHub's request limit: try again at `retry_at` (Unix seconds).
    RateLimited {
        retry_at: u64,
    },
    Status(u16),
    Refused(String),
    TooLarge,
    TooManyRedirects,
    Cancelled,
    Listing(ListingError),
    Disk(String),
    Incomplete {
        expected: u64,
        received: u64,
    },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "{error}"),
            Self::RateLimited { .. } => write!(
                f,
                "GitHub limits how often it is asked; Midnight Explorer tries again later."
            ),
            Self::Status(404) => write!(f, "GitHub has no such release file (HTTP 404)."),
            Self::Status(status) => write!(f, "GitHub answered with HTTP {status}."),
            Self::Refused(detail) => write!(f, "{detail}"),
            Self::TooLarge => write!(f, "The download is larger than allowed."),
            Self::TooManyRedirects => write!(f, "GitHub redirected the download too often."),
            Self::Cancelled => write!(f, "Cancelled."),
            Self::Listing(error) => write!(f, "{error}"),
            Self::Disk(detail) => write!(f, "The download cannot be saved: {detail}"),
            Self::Incomplete { expected, received } => {
                write!(f, "The download stopped at {received} of {expected} bytes.")
            }
        }
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Clone)]
pub struct UpdateClient {
    transport: Arc<dyn Transport>,
    endpoints: Endpoints,
}

impl UpdateClient {
    pub fn new(transport: Arc<dyn Transport>, endpoints: Endpoints) -> Self {
        Self {
            transport,
            endpoints,
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// The published releases, drafts left out.
    pub fn releases(&self, cancel: &AtomicBool) -> Result<Vec<Release>, FetchError> {
        let body = self.fetch_small(
            &self.endpoints.releases_url,
            "application/vnd.github+json",
            MAX_LISTING_BYTES,
            cancel,
        )?;
        release::parse_releases(&body).map_err(FetchError::Listing)
    }

    /// A whole small file, refused beyond `limit` bytes.
    pub fn fetch_small(
        &self,
        url: &str,
        accept: &'static str,
        limit: u64,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, FetchError> {
        let mut sink = MemorySink {
            body: Vec::new(),
            limit,
        };
        let head = self.get_following(url, accept, 0, cancel, &mut sink)?;
        check_status(&head)?;
        Ok(sink.body)
    }

    /// Downloads `url` into `destination`, which must end up `expected` bytes long.
    /// The partial file `<destination>.part` is resumed with a range request when the
    /// connection breaks off or an earlier attempt left it.
    pub fn download(
        &self,
        url: &str,
        destination: &Path,
        expected: u64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), FetchError> {
        if expected > MAX_ARCHIVE_BYTES {
            return Err(FetchError::TooLarge);
        }
        let partial = partial_path(destination);
        let mut last_error = None;
        for _ in 0..DOWNLOAD_ATTEMPTS {
            match self.download_attempt(url, &partial, expected, cancel, progress) {
                Ok(()) => {
                    fs::rename(&partial, destination)
                        .map_err(|error| FetchError::Disk(error.to_string()))?;
                    return Ok(());
                }
                Err(error @ (FetchError::Transport(_) | FetchError::Incomplete { .. })) => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or(FetchError::Incomplete {
            expected,
            received: 0,
        }))
    }

    fn download_attempt(
        &self,
        url: &str,
        partial: &Path,
        expected: u64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), FetchError> {
        let mut have = fs::metadata(partial).map_or(0, |metadata| metadata.len());
        if have > expected {
            have = 0;
        }
        if have == expected && expected > 0 {
            return Ok(());
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(partial)
            .map_err(|error| FetchError::Disk(error.to_string()))?;
        progress(have, expected);
        let mut sink = FileSink {
            file,
            requested: have,
            written: have,
            expected,
            progress,
        };
        let result = self.get_following(url, "application/octet-stream", have, cancel, &mut sink);
        let written = sink.written;
        sink.file
            .sync_all()
            .map_err(|error| FetchError::Disk(error.to_string()))?;
        let head = result?;
        if head.status == 416 && have == expected {
            return Ok(());
        }
        check_status(&head)?;
        if written != expected {
            return Err(FetchError::Incomplete {
                expected,
                received: written,
            });
        }
        Ok(())
    }

    /// Follows redirects, checking every address against the policy.
    fn get_following(
        &self,
        url: &str,
        accept: &'static str,
        range_from: u64,
        cancel: &AtomicBool,
        sink: &mut dyn Sink,
    ) -> Result<Head, FetchError> {
        let mut url = url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            if cancel.load(Ordering::Relaxed) {
                return Err(FetchError::Cancelled);
            }
            let target = self
                .endpoints
                .policy
                .check(&url)
                .map_err(FetchError::Refused)?;
            let request = Request {
                url: url.clone(),
                accept,
                range_from,
            };
            let mut guarded = CancelSink {
                inner: &mut *sink,
                cancel,
            };
            let head = self
                .transport
                .get(&request, &mut guarded)
                .map_err(|error| match error {
                    TransportError::Sink(error) => sink_error(error),
                    other => FetchError::Transport(other),
                })?;
            if !head.is_redirect() {
                return Ok(head);
            }
            let location = head
                .location
                .as_deref()
                .and_then(|location| resolve_location(&target, location))
                .ok_or_else(|| FetchError::Refused("GitHub sent an invalid redirect.".into()))?;
            url = location;
        }
        Err(FetchError::TooManyRedirects)
    }
}

const CANCELLED: &str = "cancelled";
const LIMIT_REACHED: &str = "limit reached";
const WRONG_RANGE: &str = "GitHub resumed the download at the wrong byte.";

fn sink_error(error: io::Error) -> FetchError {
    match error.to_string().as_str() {
        CANCELLED => FetchError::Cancelled,
        LIMIT_REACHED => FetchError::TooLarge,
        WRONG_RANGE => FetchError::Refused(WRONG_RANGE.into()),
        _ => FetchError::Disk(error.to_string()),
    }
}

fn check_status(head: &Head) -> Result<(), FetchError> {
    if head.is_success() {
        return Ok(());
    }
    let status = head.status;
    let limited = status == 429 || (status == 403 && head.rate_remaining == Some(0));
    if limited {
        let now = unix_now();
        let retry_at = head
            .retry_after
            .map(|seconds| now + seconds)
            .or(head.rate_reset.filter(|reset| *reset > now))
            .unwrap_or(now + DEFAULT_RATE_WAIT_SECONDS);
        return Err(FetchError::RateLimited { retry_at });
    }
    Err(FetchError::Status(status))
}

pub(crate) fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_owned();
    name.push(".part");
    destination.with_file_name(name)
}

struct CancelSink<'a> {
    inner: &'a mut dyn Sink,
    cancel: &'a AtomicBool,
}

impl Sink for CancelSink<'_> {
    fn begin(&mut self, head: &Head) -> io::Result<()> {
        self.inner.begin(head)
    }

    fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other(CANCELLED));
        }
        self.inner.write(chunk)
    }
}

struct MemorySink {
    body: Vec<u8>,
    limit: u64,
}

impl Sink for MemorySink {
    fn begin(&mut self, head: &Head) -> io::Result<()> {
        self.body.clear();
        if head
            .content_length
            .is_some_and(|length| length > self.limit)
        {
            return Err(io::Error::other(LIMIT_REACHED));
        }
        Ok(())
    }

    fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        if self.body.len() as u64 + chunk.len() as u64 > self.limit {
            return Err(io::Error::other(LIMIT_REACHED));
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }
}

/// Appends a 206 answer that starts at the requested byte; a 200 answer starts over.
struct FileSink<'a> {
    file: File,
    requested: u64,
    written: u64,
    expected: u64,
    progress: &'a mut dyn FnMut(u64, u64),
}

impl Sink for FileSink<'_> {
    fn begin(&mut self, head: &Head) -> io::Result<()> {
        let start = if head.status == 206 {
            if head.range_start != Some(self.requested) {
                return Err(io::Error::other(WRONG_RANGE));
            }
            self.requested
        } else {
            0
        };
        if head
            .content_length
            .is_some_and(|length| start + length > self.expected)
        {
            return Err(io::Error::other(LIMIT_REACHED));
        }
        self.file.set_len(start)?;
        self.file.seek(SeekFrom::Start(start))?;
        self.written = start;
        (self.progress)(start, self.expected);
        Ok(())
    }

    fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        if self.written + chunk.len() as u64 > self.expected {
            return Err(io::Error::other(LIMIT_REACHED));
        }
        self.file.write_all(chunk)?;
        self.written += chunk.len() as u64;
        (self.progress)(self.written, self.expected);
        Ok(())
    }
}
