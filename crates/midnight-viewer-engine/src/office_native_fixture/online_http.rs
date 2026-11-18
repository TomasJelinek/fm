//! Authored native-test HTML and a bounded loopback server. Nothing starts at import.
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::windows::fs::OpenOptionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) const ROOT_TOKEN: &str = "NATIVE_CAPTURED_ROOT_ROUNDTRIP";
pub(super) const NETWORK_TOKEN: &str = "NATIVE_LOCAL_NETWORK_ROUNDTRIP";
const TARGET: &str = "/native-online-return";
const MAX_HEADERS: usize = 16 * 1024;
const MAX_REQUESTS: usize = 8;

#[derive(Debug)]
pub(super) struct MainReads {
    pub completed_reads: AtomicUsize,
    pub first_reads: AtomicUsize,
    pub returned_bytes: AtomicU64,
    requests: AtomicUsize,
    releases: AtomicUsize,
    drops: AtomicUsize,
    pub length: u64,
}

impl MainReads {
    pub(super) fn verify_released(&self, path: &Path) -> Result<(), String> {
        let releases = self.releases.load(Ordering::Acquire);
        let drops = self.drops.load(Ordering::Acquire);
        if releases != 1 || drops != 1 {
            return Err(format!(
                "captured HTML main cleanup: releases={releases}, drops={drops}"
            ));
        }
        // Only the authored D fixture is probed; this is actual native sharing,
        // not a counter pretending to observe CEF's separate process tree.
        let probe = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .map_err(message)?;
        drop(probe);
        eprintln!(
            "Native captured HTML cleanup: requests={} completed={} returned_bytes={} releases={releases} drops={drops}; exclusive sharing probe succeeded",
            self.requests.load(Ordering::Acquire),
            self.completed_reads.load(Ordering::Acquire),
            self.returned_bytes.load(Ordering::Acquire)
        );
        Ok(())
    }
}

#[derive(Debug)]
struct ObservedMain {
    inner: Option<CapturedFile>,
    reads: Arc<MainReads>,
}

impl ObservedMain {
    fn inner(&self) -> &CapturedFile {
        self.inner
            .as_ref()
            .expect("live HTML observer owns its native capture")
    }
}

impl ContentSource for ObservedMain {
    fn length(&self) -> u64 {
        self.inner().length()
    }
    fn display_name(&self) -> String {
        self.inner().display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.inner().stable_version()
    }
    fn validate(&self) -> Result<(), Error> {
        self.inner().revalidate()
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.inner().cleanup_unconfirmed()
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.requests.fetch_add(1, Ordering::AcqRel);
        // This is a genuine checked read of the authored native main file.
        // The observer never supplies a replacement ResourceRead/Data result.
        let bytes = self.inner().read_at(offset, length)?;
        if !bytes.is_empty() {
            self.reads
                .returned_bytes
                .fetch_add(bytes.len() as u64, Ordering::AcqRel);
            if offset == 0 {
                self.reads.first_reads.fetch_add(1, Ordering::AcqRel);
            }
        }
        self.reads.completed_reads.fetch_add(1, Ordering::AcqRel);
        Ok(bytes)
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(source) => {
                let mut retained = super::super::Retained::new(source);
                let inner =
                    retained.get_mut().inner.take().ok_or_else(|| {
                        Error("captured HTML release was already consumed".into())
                    })?;
                inner.release_on_worker()?;
                retained.get().reads.releases.fetch_add(1, Ordering::AcqRel);
                drop(retained.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

impl Drop for ObservedMain {
    fn drop(&mut self) {
        self.reads.drops.fetch_add(1, Ordering::AcqRel);
    }
}

pub(super) fn capture_main(
    path: &Path,
    network_url: &str,
) -> Result<(CapturedFile, Arc<MainReads>), String> {
    if !network_url.starts_with("http://127.0.0.1:")
        || !network_url.ends_with(TARGET)
        || network_url
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '&' | '"' | '\''))
    {
        return Err("fixture network URL is not its owned loopback target".into());
    }
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Captured return root</title><link rel=\"icon\" href=\"data:,\"></head><body style=\"color:#111;background:#fff;font:16px monospace\"><h1>{ROOT_TOKEN}</h1><p>This authored main is delivered only through its captured source.</p><a href=\"{network_url}\">Open the local network page</a></body></html>"
    );
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(message)?;
    file.write_all(html.as_bytes()).map_err(message)?;
    file.sync_all().map_err(message)?;
    drop(file);
    let inner = CapturedFile::capture(path).map_err(message)?;
    let reads = Arc::new(MainReads {
        completed_reads: AtomicUsize::new(0),
        first_reads: AtomicUsize::new(0),
        returned_bytes: AtomicU64::new(0),
        requests: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
        drops: AtomicUsize::new(0),
        length: inner.length(),
    });
    let source = Arc::new(ObservedMain {
        inner: Some(inner),
        reads: reads.clone(),
    });
    match CapturedFile::from_source(source.clone()) {
        Ok(file) => Ok((file, reads)),
        Err(error) => match source.release_on_worker() {
            Ok(()) => Err(error.to_string()),
            Err(cleanup) => Err(format!("{error}; {cleanup}")),
        },
    }
}

#[derive(Debug)]
struct Request {
    target: String,
    host: String,
}

struct ServerState {
    stop: AtomicBool,
    return_root: Mutex<Option<String>>,
    requests: Mutex<Vec<Request>>,
    responses: AtomicUsize,
}

pub(super) struct Server {
    address: SocketAddr,
    state: Arc<ServerState>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

impl Server {
    /// Called only by the ignored, explicitly leased native case.
    pub(super) fn start() -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(message)?;
        listener.set_nonblocking(true).map_err(message)?;
        let address = listener.local_addr().map_err(message)?;
        let state = Arc::new(ServerState {
            stop: AtomicBool::new(false),
            return_root: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
            responses: AtomicUsize::new(0),
        });
        let run = state.clone();
        let worker = thread::Builder::new()
            .name("viewer-online-http-fixture".into())
            .spawn(move || serve(listener, address, &run))
            .map_err(message)?;
        Ok(Self {
            address,
            state,
            worker: Some(worker),
        })
    }

    pub(super) fn url(&self) -> String {
        format!("http://{}{TARGET}", self.address)
    }

    pub(super) fn set_return_root(&self, root: &str) -> Result<(), String> {
        let document = root
            .strip_prefix("https://mewv-")
            .and_then(|s| s.strip_suffix(".invalid/"))
            .and_then(|s| s.parse::<u64>().ok());
        if document.is_none_or(|document| document == 0) {
            return Err("fixture return target is not the actual captured root".into());
        }
        *self.state.return_root.lock().map_err(message)? = Some(root.into());
        Ok(())
    }

    pub(super) fn responses(&self) -> usize {
        self.state.responses.load(Ordering::Acquire)
    }

    pub(super) fn assert_only_network_page_requests(&self) -> Result<(), String> {
        let requests = self.state.requests.lock().map_err(message)?;
        if requests.len() != 1
            || self.responses() != 1
            || requests
                .iter()
                .any(|r| r.target != TARGET || r.host != self.address.to_string())
        {
            return Err(format!("unexpected local HTTP requests: {requests:?}"));
        }
        Ok(())
    }

    /// Joining confirms that the real worker dropped its owned listener/socket.
    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.state.stop.store(true, Ordering::Release);
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| "local HTTP fixture worker panicked".to_owned())?
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("local HTTP fixture cleanup failed: {error}");
        }
    }
}

fn serve(listener: TcpListener, address: SocketAddr, state: &ServerState) -> Result<(), String> {
    while !state.stop.load(Ordering::Acquire) {
        let (mut stream, peer) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(message(error)),
        };
        if !peer.ip().is_loopback() {
            return Err("non-loopback fixture client".into());
        }
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(message)?;
        stream
            .set_write_timeout(Some(Duration::from_millis(500)))
            .map_err(message)?;
        let Some(request) = read_request(&mut stream, &address.to_string())? else {
            // Chromium may close an unused speculative TCP connection. It sent
            // no HTTP request; do not mistake that for a refused navigation.
            continue;
        };
        {
            let mut requests = state.requests.lock().map_err(message)?;
            if requests.len() >= MAX_REQUESTS {
                return Err("HTTP fixture request budget exceeded".into());
            }
            requests.push(request);
        }
        let root = state
            .return_root
            .lock()
            .map_err(message)?
            .clone()
            .ok_or("HTTP request preceded the actual online root binding")?;
        let body = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Local network page</title><link rel=\"icon\" href=\"data:,\"></head><body style=\"color:#111;background:#fff;font:16px monospace\"><h1>{NETWORK_TOKEN}</h1><p>This distinct page came from the owned HTTP listener.</p><a href=\"{root}\">Return to the captured root</a></body></html>"
        );
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'none'\r\n\r\n",
            body.len()
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        write_response(&mut stream, header.as_bytes(), deadline)?;
        write_response(&mut stream, body.as_bytes(), deadline)?;
        drop(stream);
        state.responses.fetch_add(1, Ordering::AcqRel);
    }
    drop(listener);
    Ok(())
}

fn write_response(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err("HTTP fixture response deadline exceeded".into());
        }
        match stream.write(bytes) {
            Ok(0) => return Err("HTTP fixture response made no progress".into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(message(error)),
        }
    }
    Ok(())
}

fn read_request(stream: &mut TcpStream, host: &str) -> Result<Option<Request>, String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Err("HTTP fixture header deadline exceeded".into());
        }
        let mut chunk = [0_u8; 1024];
        let count = match stream.read(&mut chunk) {
            Ok(0) if bytes.is_empty() => return Ok(None),
            Ok(0) => return Err("HTTP fixture client closed before headers".into()),
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(message(error)),
        };
        if bytes.len() + count > MAX_HEADERS {
            return Err("HTTP fixture header budget exceeded".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|end| end == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).map_err(message)?;
    let mut lines = request.split("\r\n");
    if lines.next() != Some("GET /native-online-return HTTP/1.1") {
        return Err("HTTP fixture received a foreign target/method or proxy request".into());
    }
    let mut received_host = None;
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("invalid HTTP fixture header")?;
        if name.eq_ignore_ascii_case("host") && received_host.replace(value.trim()).is_some() {
            return Err("duplicate HTTP fixture Host".into());
        }
    }
    if received_host != Some(host) {
        return Err("HTTP fixture received a foreign Host".into());
    }
    Ok(Some(Request {
        target: TARGET.into(),
        host: host.into(),
    }))
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
