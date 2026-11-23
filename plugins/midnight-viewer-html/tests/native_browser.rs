//! Opt-in real Chromium qualification of the small browser (web viewer API 0.4): offline
//! scripts run, reach no network through any channel, read no captured file and open no
//! window; local pages render with their stylesheets, `@import`, fonts and images (also
//! from folders above the document and with spaces or non-ASCII names); and the page's
//! cursor reaches the host. Run like `native_runtime.rs` with `--ignored`.
#![cfg(feature = "native-cef")]
use midnight_web_viewer_api::*;
use std::{
    collections::HashMap,
    io::Write,
    net::{TcpListener, UdpSocket},
    process::{Command as Process, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

fn send(stdin: &mut impl Write, sequence: &mut u64, kind: CommandKind) {
    *sequence += 1;
    write_message(
        stdin,
        &Message::Command(Command {
            header: Header {
                session: 1,
                document: 1,
                sequence: *sequence,
            },
            kind,
        }),
    )
    .unwrap();
}

fn pointer(stdin: &mut impl Write, sequence: &mut u64, x: f64, y: f64) {
    send(
        stdin,
        sequence,
        CommandKind::Pointer(Pointer {
            kind: PointerKind::Move,
            button: PointerButton::None,
            x,
            y,
            modifiers: Modifiers::default(),
            click_count: 0,
        }),
    );
}

/// Percent-decoded UTF-8, as the host decodes a relative URL.
fn decoded(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let mut bytes = Vec::new();
    let mut input = path.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let hex: String = [input.next().unwrap(), input.next().unwrap()]
                .iter()
                .map(|&b| b as char)
                .collect();
            bytes.push(u8::from_str_radix(&hex, 16).unwrap());
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).unwrap()
}

/// A 2x2 opaque PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a,
    0x73, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x9f, 0x81, 0x81, 0x81, 0xe1, 0x3f, 0x03, 0x03, 0x03, 0x00, 0x1a, 0xf6, 0x04, 0xfd, 0x66, 0x15,
    0x2c, 0x5c, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A TrueType font of the machine, for the web font check; None skips that check.
fn system_font() -> Option<Vec<u8>> {
    [
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok())
}

fn page(tcp: u16, udp: u16) -> String {
    format!(
        r##"<!doctype html><html><head><meta charset=utf-8><title>start</title>
<link rel=stylesheet href="css/site.css">
<link rel=stylesheet href="../shared/up%20one.css">
<link rel=preconnect href="http://127.0.0.1:{tcp}">
<link rel=dns-prefetch href="http://dns-probe.example">
<link rel=prefetch href="http://127.0.0.1:{tcp}/prefetch">
<link rel=preload as=fetch href="http://127.0.0.1:{tcp}/preload">
<script type=speculationrules>{{"prefetch":[{{"source":"list","urls":["http://127.0.0.1:{tcp}/spec"]}}],"prerender":[{{"source":"list","urls":["http://127.0.0.1:{tcp}/prerender"]}}]}}</script>
<style>@font-face{{font-family:Probe;src:url("fonts/probe%20font.ttf")}} body{{margin:0}}
#link{{position:absolute;left:0;top:150px;width:200px;height:40px;display:block;background:#fff}}
#text{{position:absolute;left:0;top:100px;margin:0;font-size:30px;background:#fff}}</style>
</head><body>
<p id=text>Plain text here</p>
<a id=link href="https://example.org/followed">LINK</a>
<div id=imported></div><div id=above></div>
<img id=picture src="images/%C5%BE%20picture.png" style="position:absolute;left:250px;top:0">
<a id=ping href="https://example.org/x" ping="http://127.0.0.1:{tcp}/ping" style="display:none">p</a>
<script>
const r = {{}};
const done = () => {{ document.title = 'R ' + JSON.stringify(r); }};
setTimeout(done, 6000);
document.title = 'script ran';
r.js = document.getElementById('text') ? 1 : 0;
document.body.dataset.ran = 'yes';
const outcome = (key, promise) => promise.then(() => r[key] = 'reached', () => r[key] = 'blocked');
const tried = (key, run) => {{ try {{ run(); }} catch (e) {{ r[key] = 'threw'; }} }};
outcome('fetch', fetch('http://127.0.0.1:{tcp}/fetch', {{mode: 'no-cors'}}));
outcome('local', fetch('css/site.css'));
outcome('file', fetch('file:///C:/Windows/win.ini'));
tried('xhr', () => {{ const x = new XMLHttpRequest(); x.open('GET', 'http://127.0.0.1:{tcp}/xhr'); x.onerror = () => r.xhr = 'blocked'; x.onload = () => r.xhr = 'reached'; x.send(); }});
tried('ws', () => {{ const w = new WebSocket('ws://127.0.0.1:{tcp}/ws'); w.onerror = () => r.ws = 'blocked'; w.onopen = () => r.ws = 'reached'; }});
tried('es', () => {{ const s = new EventSource('http://127.0.0.1:{tcp}/es'); s.onerror = () => {{ r.es = 'blocked'; s.close(); }}; }});
tried('beacon', () => {{ r.beacon = navigator.sendBeacon('http://127.0.0.1:{tcp}/beacon', 'x') ? 'queued' : 'refused'; }});
tried('img', () => {{ const i = new Image(); i.onerror = () => r.img = 'blocked'; i.onload = () => r.img = 'reached'; i.src = 'http://127.0.0.1:{tcp}/img'; }});
tried('imgfile', () => {{ const i = new Image(); i.onerror = () => r.imgfile = 'blocked'; i.onload = () => r.imgfile = 'reached'; i.src = 'file:///C:/Windows/win.ini'; }});
outcome('import', import('http://127.0.0.1:{tcp}/module.js'));
tried('worker', () => {{ new Worker('data:text/javascript,fetch("http://127.0.0.1:{tcp}/worker")'); r.worker = 'created'; }});
tried('sw', () => {{ r.sw = typeof navigator.serviceWorker === 'undefined' ? 'none' : 'present'; if (navigator.serviceWorker) outcome('swreg', navigator.serviceWorker.register('sw.js')); }});
r.rtc = typeof RTCPeerConnection;
r.rtcw = typeof webkitRTCPeerConnection;
tried('rtcdefine', () => {{ window.RTCPeerConnection = function () {{}}; r.rtcdefine = typeof RTCPeerConnection; }});
tried('frame', () => {{ const f = document.createElement('iframe'); document.body.appendChild(f); r.frame = f.contentWindow ? typeof f.contentWindow.RTCPeerConnection : 'none'; }});
tried('stun', () => {{ const P = window.RTCPeerConnection; const c = new P({{iceServers: [{{urls: 'stun:127.0.0.1:{udp}'}}]}}); c.createDataChannel('x'); c.createOffer().then(o => c.setLocalDescription(o)); r.stun = 'created'; }});
tried('popup', () => {{ r.popup = window.open('https://example.org/popup') ? 'opened' : 'none'; }});
tried('storage', () => {{ r.storage = typeof localStorage.getItem; }});
window.addEventListener('load', () => {{
  const css = (id, property) => getComputedStyle(document.getElementById(id))[property];
  r.imported = css('imported', 'width');
  r.above = css('above', 'width');
  r.picture = document.getElementById('picture').naturalWidth;
  tried('canvas', () => {{ const c = document.createElement('canvas'); const g = c.getContext('2d'); g.drawImage(document.getElementById('picture'), 0, 0); g.getImageData(0, 0, 1, 1); r.canvas = 'read'; }});
  tried('dataurl', () => {{ const c = document.createElement('canvas'); c.getContext('2d').drawImage(document.getElementById('picture'), 0, 0); c.toDataURL(); r.dataurl = 'read'; }});
  r.frames = typeof VideoFrame;
  document.fonts.load('20px Probe').then(fonts => {{ r.font = fonts.length; }}, () => {{ r.font = -1; }});
  try {{ location.href = 'http://127.0.0.1:{tcp}/navigate'; }} catch (e) {{}}
  setTimeout(done, 2500);
}});
</script></body></html>"##
    )
}

#[test]
#[ignore = "requires matching packaged CEF runtime and interactive Windows or Linux desktop"]
fn offline_scripts_run_contained_and_local_pages_render_with_their_styles() {
    let executable =
        std::env::var_os("MIDNIGHT_HTML_TEST_EXECUTABLE").expect("set packaged bootstrap path");
    let profile =
        std::env::var_os("MIDNIGHT_WEB_VIEWER_PROFILE").expect("set owned private profile");
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    tcp.set_nonblocking(true).unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_nonblocking(true).unwrap();
    let html = page(
        tcp.local_addr().unwrap().port(),
        udp.local_addr().unwrap().port(),
    )
    .into_bytes();
    let font = system_font();
    let mut files: HashMap<&str, (Vec<u8>, &str)> = HashMap::from([
        (
            "css/site.css",
            (
                b"@import \"base.css\";\nhtml,body{background:#123456}".to_vec(),
                "text/css",
            ),
        ),
        (
            "css/base.css",
            (b"#imported{width:41px}".to_vec(), "text/css"),
        ),
        (
            "../shared/up one.css",
            (b"#above{width:77px}".to_vec(), "text/css"),
        ),
        ("images/ž picture.png", (PNG.to_vec(), "image/png")),
    ]);
    if let Some(font) = &font {
        files.insert("fonts/probe font.ttf", (font.clone(), "font/ttf"));
    }
    let mut child = Process::new(executable)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", profile)
        .env(HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1)
        .env(LINKS_ENV, LINKS_HOST_V1)
        .env(CURSOR_ENV, CURSOR_V1)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut stdout) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let mut sequence = 0;
    send(
        &mut stdin,
        &mut sequence,
        CommandKind::Open {
            source: Resource {
                id: 1,
                length: html.len() as u64,
                mime: "text/html".into(),
            },
            viewport: Viewport {
                revision: 1,
                width: 400,
                height: 300,
                scale: 1.0,
            },
            policy: Policy {
                network: false,
                scripts: true,
            },
        },
    );
    let mut resources: HashMap<u64, Vec<u8>> = HashMap::from([(1, html.clone())]);
    let mut next_id = 2;
    let mut requested = Vec::new();
    let mut report = None;
    let mut styled = false;
    let mut cursors = Vec::new();
    let mut hovers = Vec::new();
    let mut activated = Vec::new();
    let mut notices = Vec::new();
    let mut titles = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline && !(report.is_some() && styled) {
        let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(100)) else {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("Chromium terminated: {status}");
            }
            continue;
        };
        match event.kind {
            EventKind::ResourceResolve { request, url, .. } => {
                let name = decoded(&url);
                requested.push(url.clone());
                let reply = match files.get(name.as_str()) {
                    Some((bytes, mime)) => {
                        resources.insert(next_id, bytes.clone());
                        next_id += 1;
                        ResourceReply::Resolved {
                            request,
                            resource: Resource {
                                id: next_id - 1,
                                length: bytes.len() as u64,
                                mime: (*mime).into(),
                            },
                        }
                    }
                    None => ResourceReply::Denied {
                        request,
                        reason: "not in the fixture".into(),
                    },
                };
                send(&mut stdin, &mut sequence, CommandKind::ResourceReply(reply));
            }
            EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => {
                let data = &resources[&resource];
                let end = (offset as usize + length as usize).min(data.len());
                send(
                    &mut stdin,
                    &mut sequence,
                    CommandKind::ResourceReply(ResourceReply::Data {
                        request,
                        resource,
                        offset,
                        bytes: data[offset as usize..end].to_vec(),
                    }),
                );
            }
            EventKind::State(state) if state.title.starts_with("R ") => {
                report = Some(state.title);
            }
            EventKind::State(state) => titles.push((state.title, state.loading)),
            EventKind::Frame(frame) => {
                let pixel = ((frame.height - 5) * frame.width + 5) as usize * 4;
                styled |= frame.rgba[pixel..pixel + 4] == [0x12, 0x34, 0x56, 255];
            }
            EventKind::Cursor(cursor) => cursors.push(cursor),
            EventKind::LinkHover { target } => hovers.push(target),
            EventKind::LinkActivated { target } => activated.push(target),
            EventKind::Error {
                code: ErrorCode::Denied,
                message,
            } => notices.push(message),
            EventKind::Error { message, .. } => panic!("provider error: {message}"),
            _ => {}
        }
    }
    // Point at the text, the background and the link: each cursor the page asks for.
    for (x, y, expected) in [
        (40.0, 115.0, Cursor::Text),
        (380.0, 280.0, Cursor::Default),
        (30.0, 165.0, Cursor::Pointer),
    ] {
        let until = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while !seen && Instant::now() < until {
            pointer(&mut stdin, &mut sequence, x, y);
            if let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(100)) {
                match event.kind {
                    EventKind::Cursor(cursor) => {
                        cursors.push(cursor);
                        seen = cursor == expected;
                    }
                    EventKind::LinkHover { target } => hovers.push(target),
                    EventKind::LinkActivated { target } => activated.push(target),
                    _ => {}
                }
            }
        }
    }
    // Late connections of anything the page started.
    std::thread::sleep(Duration::from_secs(2));
    while let Ok(Message::Event(event)) = rx.try_recv() {
        match event.kind {
            EventKind::Cursor(cursor) => cursors.push(cursor),
            EventKind::LinkHover { target } => hovers.push(target),
            EventKind::LinkActivated { target } => activated.push(target),
            _ => {}
        }
    }
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    eprintln!(
        "report: {report:?}\nrequested: {requested:?}\ncursors: {cursors:?}\nhovers: {hovers:?}\nnotices: {notices:?}\ntitles: {titles:?}\nstyled: {styled}"
    );
    let report = report.expect("the page's script reported (offline scripts run)");
    let mut reached = Vec::new();
    while let Ok((mut stream, _)) = tcp.accept() {
        use std::io::Read;
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut request = [0; 512];
        let read = stream.read(&mut request).unwrap_or(0);
        reached.push(
            String::from_utf8_lossy(&request[..read])
                .lines()
                .next()
                .unwrap_or("(connection only)")
                .to_owned(),
        );
    }
    assert!(
        reached.is_empty(),
        "an offline page reached the loopback TCP listener: {reached:?}"
    );
    let mut buffer = [0; 1500];
    assert_eq!(
        udp.recv_from(&mut buffer).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "an offline page reached the loopback UDP listener"
    );
    let field = |key: &str| {
        let start = report.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = &report[start..];
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim_matches('"').to_owned())
    };
    assert_eq!(field("js").as_deref(), Some("1"), "offline scripts run");
    for channel in [
        "fetch", "local", "file", "xhr", "es", "img", "imgfile", "import",
    ] {
        assert_eq!(
            field(channel).as_deref(),
            Some("blocked"),
            "{channel} in {report}"
        );
    }
    // A channel the CSP refuses throws or errors, or (a beacon, a worker) is never sent;
    // the listeners above prove that nothing of them reached the network.
    for channel in ["ws", "beacon", "worker"] {
        assert!(field(channel).is_some(), "{channel} in {report}");
        assert_ne!(
            field(channel).as_deref(),
            Some("reached"),
            "{channel} in {report}"
        );
    }
    assert!(
        matches!(field("sw").as_deref(), Some("none" | "threw"))
            || field("swreg").as_deref() == Some("blocked"),
        "{report}"
    );
    for webrtc in ["rtc", "rtcw", "rtcdefine"] {
        assert_eq!(
            field(webrtc).as_deref(),
            Some("undefined"),
            "{webrtc} in {report}"
        );
    }
    // A frame of the sandboxed page is another opaque origin: its window is out of reach.
    assert!(
        matches!(field("frame").as_deref(), Some("undefined" | "threw")),
        "{report}"
    );
    assert_eq!(field("stun").as_deref(), Some("threw"), "{report}");
    assert_eq!(field("popup").as_deref(), Some("none"), "{report}");
    // No local image's pixels can be read back.
    for readback in ["canvas", "dataurl"] {
        assert_eq!(
            field(readback).as_deref(),
            Some("threw"),
            "{readback} in {report}"
        );
    }
    assert_eq!(field("frames").as_deref(), Some("undefined"), "{report}");
    // Local pages render with their styles: linked, @imported, from a folder above, with
    // spaces and non-ASCII names, and their web font.
    assert!(styled, "the linked stylesheet painted the page");
    assert_eq!(field("imported").as_deref(), Some("41px"), "{report}");
    assert_eq!(field("above").as_deref(), Some("77px"), "{report}");
    assert_eq!(field("picture").as_deref(), Some("2"), "{report}");
    if font.is_some() {
        assert_eq!(field("font").as_deref(), Some("1"), "{report}");
    }
    assert!(
        requested.contains(&"../shared/up%20one.css".to_owned()),
        "{requested:?}"
    );
    assert!(
        requested
            .iter()
            .all(|url| !url.contains("127.0.0.1") && !url.starts_with("file:")),
        "{requested:?}"
    );
    // No script-made navigation or window reached the host as a followed link.
    assert!(activated.is_empty(), "{activated:?}");
    // The page's cursor reaches the host: the hand over the link, the I-beam over text,
    // the arrow elsewhere.
    for expected in [Cursor::Pointer, Cursor::Text, Cursor::Default] {
        assert!(cursors.contains(&expected), "{expected:?} in {cursors:?}");
    }
    assert!(
        hovers.contains(&"https://example.org/followed".to_owned()),
        "{hovers:?}"
    );
}

/// A frame as a 24-bit BMP (premultiplied RGBA over white).
fn bmp(frame: &Frame) -> Vec<u8> {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut out = Vec::with_capacity(size);
    out.extend(b"BM");
    out.extend((size as u32).to_le_bytes());
    out.extend([0; 4]);
    out.extend(54u32.to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend((width as i32).to_le_bytes());
    out.extend((height as i32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend([0; 24]);
    for y in (0..height).rev() {
        for x in 0..width {
            let p = &frame.rgba[(y * width + x) * 4..][..4];
            let over = |c: u8| c.saturating_add(255 - p[3]);
            out.extend([over(p[2]), over(p[1]), over(p[0])]);
        }
        out.extend(std::iter::repeat_n(0, row - width * 3));
    }
    out
}

/// Render `html` with `files` (keyed by the provider's decoded relative URL) under
/// `policy` and return the page's last frame once it has been still for a while.
fn render(
    executable: &std::ffi::OsStr,
    policy: Policy,
    html: &[u8],
    files: &HashMap<String, (Vec<u8>, &'static str)>,
    viewport: (u32, u32),
) -> Frame {
    let profile =
        std::path::PathBuf::from(std::env::var_os("MIDNIGHT_WEB_VIEWER_PROFILE").unwrap())
            .join(format!("shot-{}", std::process::id()))
            .join(format!(
                "{}",
                Instant::now().elapsed().as_nanos() ^ html.len() as u128
            ));
    std::fs::create_dir_all(&profile).unwrap();
    let mut child = Process::new(executable)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", &profile)
        .env(HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1)
        .env(LINKS_ENV, LINKS_HOST_V1)
        .env(CURSOR_ENV, CURSOR_V1)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut stdout) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let mut sequence = 0;
    send(
        &mut stdin,
        &mut sequence,
        CommandKind::Open {
            source: Resource {
                id: 1,
                length: html.len() as u64,
                mime: "text/html".into(),
            },
            viewport: Viewport {
                revision: 1,
                width: viewport.0,
                height: viewport.1,
                scale: 1.0,
            },
            policy,
        },
    );
    let mut resources: HashMap<u64, Vec<u8>> = HashMap::from([(1, html.to_vec())]);
    let mut last = None;
    let mut painted = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline
        && !(last.is_some() && painted.elapsed() > Duration::from_millis(1500))
    {
        let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        match event.kind {
            EventKind::ResourceResolve { request, url, .. } => {
                let reply = match files.get(&decoded(&url)) {
                    Some((bytes, mime)) => {
                        let id = resources.len() as u64 + 1;
                        resources.insert(id, bytes.clone());
                        ResourceReply::Resolved {
                            request,
                            resource: Resource {
                                id,
                                length: bytes.len() as u64,
                                mime: (*mime).into(),
                            },
                        }
                    }
                    None => ResourceReply::Denied {
                        request,
                        reason: "not in the fixture".into(),
                    },
                };
                send(&mut stdin, &mut sequence, CommandKind::ResourceReply(reply));
            }
            EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => {
                let data = &resources[&resource];
                let end = (offset as usize + length as usize).min(data.len());
                send(
                    &mut stdin,
                    &mut sequence,
                    CommandKind::ResourceReply(ResourceReply::Data {
                        request,
                        resource,
                        offset,
                        bytes: data[offset as usize..end].to_vec(),
                    }),
                );
            }
            EventKind::Frame(frame) => {
                last = Some(frame);
                painted = Instant::now();
            }
            _ => {}
        }
    }
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    last.expect("the page painted")
}

/// Typical local pages for before/after screenshots: generated documentation whose
/// stylesheet and script lie one folder above the page, and a page saved "complete" with
/// its `_files` folder. `MIDNIGHT_HTML_SHOT_DIR` names the output folder and
/// `MIDNIGHT_HTML_SHOT_LABEL` the file prefix; `MIDNIGHT_HTML_SHOT_SCRIPTS=0` opens them
/// offline without scripts, as API 0.3 providers must.
#[test]
#[ignore = "writes screenshots; requires the packaged CEF runtime"]
fn screenshots_of_generated_documentation_and_a_saved_page() {
    let executable =
        std::env::var_os("MIDNIGHT_HTML_TEST_EXECUTABLE").expect("set packaged bootstrap path");
    let Some(folder) = std::env::var_os("MIDNIGHT_HTML_SHOT_DIR") else {
        return;
    };
    let label = std::env::var("MIDNIGHT_HTML_SHOT_LABEL").unwrap_or_else(|_| "shot".into());
    let policy = Policy {
        network: false,
        scripts: std::env::var("MIDNIGHT_HTML_SHOT_SCRIPTS").as_deref() != Ok("0"),
    };
    let docs = r##"<!doctype html><html><head><meta charset=utf-8><title>Widget in widgets - Rust</title>
<link rel=stylesheet href="../static.files/rustdoc.css"><script src="../static.files/main.js" defer></script></head>
<body><nav class=sidebar><h2>widgets</h2><ul><li><a href="struct.Widget.html">Widget</a></li><li><a href="fn.build.html">build</a></li><li><a href="index.html">All items</a></li></ul></nav>
<main><h1>Struct <span class=struct>Widget</span></h1><pre class=rust>pub struct Widget {
    pub name: String,
    pub size: u32,
}</pre><p>A widget with a name and a size.</p><h2>Implementations</h2><pre class=rust>impl Widget { pub fn new(name: &amp;str) -&gt; Self }</pre></main></body></html>"##;
    let css = "body{margin:0;font:16px sans-serif;background:#f5f5f5;color:#222;display:flex}\
        .sidebar{width:180px;min-height:100vh;background:#2a3542;color:#fff;padding:12px}\
        .sidebar a{color:#9cd2ff}.sidebar ul{list-style:none;padding:0}\
        main{padding:16px 24px;flex:1}h1{border-bottom:2px solid #c8a2ff;color:#3b2b6b}\
        .struct{color:#ad378a}pre.rust{background:#fff;border-left:4px solid #5fa8ff;padding:8px}\
        #search{display:block;margin:0 0 12px;padding:6px;border:1px solid #aaa;background:#fffbe0}";
    let js = "document.querySelector('main').insertAdjacentHTML('afterbegin', \
        '<div id=search>Search (added by main.js)</div>');";
    let docs_files: HashMap<String, (Vec<u8>, &'static str)> = HashMap::from([
        (
            "../static.files/rustdoc.css".to_owned(),
            (css.as_bytes().to_vec(), "text/css"),
        ),
        (
            "../static.files/main.js".to_owned(),
            (js.as_bytes().to_vec(), "text/javascript"),
        ),
    ]);
    let saved = r##"<!doctype html><html><head><meta charset=utf-8><title>Trip report – Saved</title>
<link rel=stylesheet href="./Trip report – Saved_files/style.css">
<script src="./Trip report – Saved_files/banner.js.download"></script></head>
<body><header>Trip report</header><article><img src="./Trip report – Saved_files/mapa ž.png" alt=map>
<p>We walked along the river and stopped at the old mill.</p></article></body></html>"##;
    let saved_files: HashMap<String, (Vec<u8>, &'static str)> = HashMap::from([
        (
            "Trip report – Saved_files/style.css".to_owned(),
            (
                b"@import \"layout.css\";\nheader{background:#1f6f4a;color:#fff;font:bold 28px serif;padding:16px}".to_vec(),
                "text/css",
            ),
        ),
        (
            "Trip report – Saved_files/layout.css".to_owned(),
            (
                b"body{margin:0;font:18px Georgia,serif;background:#fbf7ef}article{padding:16px}img{width:160px;height:160px;float:left;margin-right:16px;image-rendering:pixelated}".to_vec(),
                "text/css",
            ),
        ),
        (
            "Trip report – Saved_files/banner.js.download".to_owned(),
            (
                b"document.addEventListener('DOMContentLoaded',()=>{const b=document.createElement('div');b.textContent='Banner added by the saved page script';b.style.cssText='background:#ffcf6e;padding:10px;font:16px sans-serif';document.body.prepend(b);});".to_vec(),
                "text/javascript",
            ),
        ),
        ("Trip report – Saved_files/mapa ž.png".to_owned(), (PNG.to_vec(), "image/png")),
    ]);
    std::fs::create_dir_all(&folder).unwrap();
    for (name, html, files) in [
        ("generated-docs", docs, &docs_files),
        ("saved-page", saved, &saved_files),
    ] {
        let frame = render(&executable, policy, html.as_bytes(), files, (900, 520));
        std::fs::write(
            std::path::Path::new(&folder).join(format!("{label}-{name}.bmp")),
            bmp(&frame),
        )
        .unwrap();
    }
}
