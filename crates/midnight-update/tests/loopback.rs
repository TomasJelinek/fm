//! The real system transport (WinHTTP on Windows, rustls/ureq on Linux) against a local
//! HTTP server: the whole check → download → verify → stage flow over real sockets,
//! through the debug-only loopback endpoint. Release builds have no such endpoint, and
//! the GitHub policy refuses the same server before connecting.
use crate::support::{ReleaseFiles, TestKey, platform, release_tree, write_tree};
use midnight_update::{
    SystemTransport,
    apply::StartupReport,
    client::{Endpoints, FetchError, UpdateClient},
    controller::{Environment, Phase, UpdateController},
    install::Installation,
    release::current_version,
    transport::UrlPolicy,
};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// A minimal HTTP/1.1 server for GET with `Range: bytes=N-`; one thread per connection.
pub struct LoopbackServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

fn answer(mut stream: TcpStream, files: &HashMap<String, Vec<u8>>, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut range = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("range: bytes=") {
            range = value.trim().trim_end_matches('-').parse().unwrap_or(0);
        }
    }
    requests
        .lock()
        .unwrap()
        .push(format!("GET {path} range={range}"));
    let response = match files.get(&path) {
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
        Some(body) if range > 0 && range < body.len() => {
            let mut head = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                body.len() - range,
                range,
                body.len() - 1,
                body.len()
            )
            .into_bytes();
            head.extend_from_slice(&body[range..]);
            head
        }
        Some(body) => {
            let mut head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            head.extend_from_slice(body);
            head
        }
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
    let mut rest = [0u8; 16];
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.read(&mut rest);
}

impl LoopbackServer {
    pub fn start(files: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = files.clone();
                let log = log.clone();
                std::thread::spawn(move || answer(stream, &files, &log));
            }
        });
        Self { base, requests }
    }
}

fn serve(release: &ReleaseFiles) -> LoopbackServer {
    // The listing names the server's own address, so it is made after binding: the
    // server learns its files through a shared map filled before the first request.
    let listener_files = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let listing = serde_json::to_vec(&[release.listing_entry(&base, true)]).unwrap();
    {
        let mut files = listener_files.lock().unwrap();
        files.insert("/releases".to_owned(), listing);
        files.insert(
            format!("/download/{}", release.archive_name),
            release.archive.clone(),
        );
        files.insert("/download/SHA256SUMS".to_owned(), release.sums.clone());
        files.insert(
            "/download/SHA256SUMS.minisig".to_owned(),
            release.signature.clone().into_bytes(),
        );
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = listener_files.lock().unwrap().clone();
            let log = log.clone();
            std::thread::spawn(move || answer(stream, &files, &log));
        }
    });
    LoopbackServer { base, requests }
}

#[test]
fn the_system_transport_runs_the_whole_flow_against_a_local_server() {
    let Some(endpoints) = Endpoints::loopback_test("http://127.0.0.1:1") else {
        // A release build: the loopback endpoint does not exist (see client.rs).
        return;
    };
    drop(endpoints);
    let key = TestKey::generate();
    let mut version = current_version();
    version.patch += 1;
    version.pre = semver::Prerelease::EMPTY;
    let release = ReleaseFiles::new(&key, &version.to_string(), "me.exe");
    let server = serve(&release);
    let endpoints = Endpoints::loopback_test(&server.base).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let installation = folder.path().join("install");
    write_tree(
        &installation,
        &release_tree(&current_version().to_string(), "me.exe", &[]),
    );
    let controller = UpdateController::start(
        Environment {
            client: UpdateClient::new(
                Arc::new(SystemTransport::new(UrlPolicy::LoopbackTest)),
                endpoints,
            ),
            anchor: Some(key.anchor()),
            current: current_version(),
            platform: Some(platform()),
            installation: Installation::detect(&installation.join("me.exe"), &platform()),
            private_folder: folder.path().join("private"),
            startup: StartupReport::default(),
        },
        Box::new(|_| {}),
    );
    controller.check(true, true);
    let wait = |done: &dyn Fn(&Phase) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let snapshot = controller.snapshot();
            if done(&snapshot.phase) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "{snapshot:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let snapshot = wait(&|phase| {
        matches!(
            phase,
            Phase::Available | Phase::Failed { .. } | Phase::UpToDate
        )
    });
    assert_eq!(snapshot.phase, Phase::Available, "{snapshot:?}");
    controller.download();
    let snapshot = wait(&|phase| matches!(phase, Phase::Ready | Phase::Failed { .. }));
    assert_eq!(snapshot.phase, Phase::Ready, "{snapshot:?}");
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests[0], "GET /releases range=0", "{requests:?}");
    assert!(
        requests
            .iter()
            .any(|request| request.contains(&release.archive_name))
    );
}

#[test]
fn the_github_policy_refuses_the_local_server_before_connecting() {
    let server = LoopbackServer::start(HashMap::from([("/releases".to_owned(), b"[]".to_vec())]));
    let client = UpdateClient::new(
        Arc::new(SystemTransport::new(UrlPolicy::GitHub)),
        Endpoints {
            releases_url: format!("{}/releases", server.base),
            policy: UrlPolicy::GitHub,
        },
    );
    let error = client.releases(&AtomicBool::new(false)).unwrap_err();
    assert!(matches!(error, FetchError::Refused(_)), "{error:?}");
    // Even a transport handed the address directly refuses plain HTTP.
    let transport = SystemTransport::default();
    let mut sink = Discard;
    let result = midnight_update::transport::Transport::get(
        &transport,
        &midnight_update::transport::Request {
            url: format!("{}/releases", server.base),
            accept: "application/json",
            range_from: 0,
        },
        &mut sink,
    );
    assert!(result.is_err());
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        server.requests.lock().unwrap().is_empty(),
        "the server was contacted"
    );
}

struct Discard;

impl midnight_update::transport::Sink for Discard {
    fn begin(&mut self, _: &midnight_update::transport::Head) -> std::io::Result<()> {
        Ok(())
    }
    fn write(&mut self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
}

/// GitHub redirects a release download to a signed address of about a thousand
/// characters; the system transport reports a longer one in full, here one of 3,000.
#[test]
fn the_system_transport_reports_a_long_redirect_location() {
    if Endpoints::loopback_test("http://127.0.0.1:1").is_none() {
        return; // A release build: the loopback policy is refused (see client.rs).
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let location = format!("{base}/signed?sig={}", "a".repeat(3000));
    let answer = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 2 {
            line.clear();
        }
        let mut stream = stream;
        stream.write_all(answer.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    let head = midnight_update::transport::Transport::get(
        &SystemTransport::new(UrlPolicy::LoopbackTest),
        &midnight_update::transport::Request {
            url: format!("{base}/asset"),
            accept: "application/octet-stream",
            range_from: 0,
        },
        &mut Discard,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(head.status, 302);
    assert_eq!(head.location.as_deref(), Some(location.as_str()));
}
