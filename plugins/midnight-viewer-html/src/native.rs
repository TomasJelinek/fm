//! Audited CEF boundary; no browser objects or native buffers cross the public protocol.
use cef::*;
use midnight_web_viewer_api as api;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

mod handlers;
mod input;
mod resources;

use crate::output::Output;

pub(super) struct Document {
    header: api::Header,
    source: api::Resource,
    policy: crate::policy::DocumentPolicy,
    viewport: Mutex<api::Viewport>,
    surface: Mutex<crate::surface::Surface>,
    layers: Mutex<crate::surface::Layers>,
    output: Arc<Output>,
    active: AtomicBool,
    closed: AtomicBool,
    finding: Mutex<crate::find::FindQueue>,
    request_sequence: AtomicU64,
    pending: Mutex<HashMap<u64, Arc<Mutex<resources::Stream>>>>,
    selection: Mutex<crate::selection::Selection>,
    mouse_buttons: AtomicU64,
    host_shortcuts: bool,
}

impl Document {
    fn emit(&self, kind: api::EventKind) {
        if self.active.load(Ordering::Acquire) {
            self.output.emit(self.header, kind);
        }
    }
    fn error(&self, message: &str) {
        self.emit(api::EventKind::Error {
            code: api::ErrorCode::Runtime,
            message: message.into(),
        });
    }
    fn request(
        &self,
        stream: Arc<Mutex<resources::Stream>>,
        event: impl FnOnce(u64) -> api::EventKind,
    ) -> bool {
        let id = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        if pending.len() >= 128 || id == u64::MAX {
            return false;
        }
        pending.insert(id, stream);
        drop(pending);
        self.emit(event(id));
        true
    }
    fn retire(&self) {
        self.active.store(false, Ordering::Release);
        let streams: Vec<_> = self
            .pending
            .lock()
            .map(|mut p| p.drain().map(|(_, s)| s).collect())
            .unwrap_or_default();
        for stream in streams {
            let callback = if let Ok(mut stream) = stream.lock() {
                stream.failed = true;
                stream.callback.take()
            } else {
                None
            };
            // Cancel can call back into this document; release both locks first.
            if let Some(callback) = callback {
                callback.cancel();
            }
        }
        if let Ok(mut frame) = self.output.frame.lock() {
            *frame = None;
        }
    }
}

pub(super) struct BrowserInstance {
    browser: Browser,
    _context: RequestContext,
    document: Arc<Document>,
}

fn open(command: api::Command, output: Arc<Output>) -> Result<BrowserInstance, String> {
    let api::CommandKind::Open {
        source,
        viewport,
        policy,
    } = command.kind
    else {
        return Err("first command must be Open".into());
    };
    let document = Arc::new(Document {
        header: command.header,
        source,
        policy: crate::policy::DocumentPolicy::new(
            command.header.document,
            policy.network,
            policy.scripts,
        )?,
        viewport: Mutex::new(viewport),
        surface: Mutex::new(crate::surface::Surface::new(
            viewport.revision,
            viewport.width,
            viewport.height,
        )?),
        output,
        active: AtomicBool::new(true),
        closed: AtomicBool::new(false),
        finding: Mutex::new(crate::find::FindQueue::default()),
        request_sequence: AtomicU64::new(1),
        pending: Mutex::new(HashMap::new()),
        selection: Mutex::new(crate::selection::Selection::default()),
        layers: Mutex::new(crate::surface::Layers::default()),
        mouse_buttons: AtomicU64::new(0),
        host_shortcuts: crate::host_keys::enabled(
            std::env::var(api::HOST_SHORTCUTS_ENV).ok().as_deref(),
        ),
    });
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::record(
        crate::trace::Event::ContextStart,
        command.header.document,
        0,
    );
    let mut context = request_context_create_context(
        Some(&RequestContextSettings {
            cookieable_schemes_exclude_defaults: 1,
            ..Default::default()
        }),
        None,
    )
    .ok_or("cannot create isolated request context")?;
    let settings = BrowserSettings {
        javascript: if policy.scripts {
            State::ENABLED
        } else {
            State::DISABLED
        },
        javascript_close_windows: State::DISABLED,
        javascript_access_clipboard: State::DISABLED,
        javascript_dom_paste: State::DISABLED,
        local_storage: State::DISABLED,
        databases_deprecated: State::DISABLED,
        webgl: State::DISABLED,
        background_color: 0xffffffff,
        // Permit a display frame every ~16 ms; the output retains only the newest paint.
        windowless_frame_rate: 60,
        ..Default::default()
    };
    let info = crate::platform::window_info();
    let mut client = handlers::ViewerClient::new(document.clone());
    let browser = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(document.policy.main_url().as_str())),
        Some(&settings),
        None,
        Some(&mut context),
    )
    .ok_or("sandboxed Chromium browser creation failed")?;
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::record(
        crate::trace::Event::ContextReady,
        command.header.document,
        0,
    );
    document.emit(api::EventKind::Ready);
    Ok(BrowserInstance {
        browser,
        _context: context,
        document,
    })
}

fn close(instance: BrowserInstance) -> Result<(), &'static str> {
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::record(
        crate::trace::Event::Closing,
        instance.document.header.document,
        0,
    );
    instance.document.retire();
    if let Some(host) = instance.browser.host() {
        host.close_browser(1);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !instance.document.closed.load(Ordering::Acquire) && Instant::now() < deadline {
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(2));
    }
    if !instance.document.closed.load(Ordering::Acquire) {
        return Err("Chromium did not close; terminating this provider before changing authority");
    }
    Ok(())
}

pub(crate) fn run(args: &MainArgs, sandbox: *mut u8) -> i32 {
    // Select the exact versioned C API before constructing any ref-counted CEF wrapper.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let mut app = handlers::ViewerApp::new();
    let result = execute_process(Some(args), Some(&mut app), sandbox);
    if result >= 0 {
        return result;
    }
    let profile = match std::env::var("MIDNIGHT_WEB_VIEWER_PROFILE") {
        Ok(profile) if std::path::Path::new(&profile).is_absolute() => profile,
        _ => {
            eprintln!("HTML viewer requires a private MIDNIGHT_WEB_VIEWER_PROFILE directory");
            return 70;
        }
    };
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::start(std::path::Path::new(&profile));
    let settings = Settings {
        no_sandbox: 0,
        windowless_rendering_enabled: 1,
        command_line_args_disabled: 1,
        root_cache_path: CefString::from(profile.as_str()),
        log_file: CefString::from(
            std::path::Path::new(&profile)
                .join("chromium.log")
                .to_string_lossy()
                .as_ref(),
        ),
        log_severity: LogSeverity::ERROR,
        cookieable_schemes_exclude_defaults: 1,
        ..Default::default()
    };
    if initialize(Some(args), Some(&settings), Some(&mut app), sandbox) == 0 {
        eprintln!(
            "Sandboxed CEF initialization failed (CEF exit {})",
            get_exit_code()
        );
        return 70;
    }
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::record(crate::trace::Event::Initialized, 0, 0);
    let (tx, rx) = mpsc::sync_channel::<api::Command>(64);
    let (output, mut events) = crate::output::channel();
    let writer_output = output.clone();
    let writer = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        let mut sequence = 0u64;
        while let Some(mut event) = events.next(&writer_output) {
            sequence += 1;
            event.header.sequence = sequence;
            let closed = matches!(event.kind, api::EventKind::Closed);
            #[cfg(feature = "diagnostic-timing")]
            let frame_bytes = if let api::EventKind::Frame(frame) = &event.kind {
                crate::trace::record(
                    crate::trace::Event::WireBegin,
                    sequence,
                    frame.rgba.len() as u64,
                );
                Some(frame.rgba.len() as u64)
            } else {
                None
            };
            if api::write_message(&mut stdout, &api::Message::Event(event)).is_err() {
                writer_output.failed.store(true, Ordering::Release);
                break;
            }
            #[cfg(feature = "diagnostic-timing")]
            if let Some(bytes) = frame_bytes {
                crate::trace::record(crate::trace::Event::WireEnd, sequence, bytes);
            }
            if closed {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        while let Ok(api::Message::Command(command)) = api::read_message(&mut stdin) {
            if tx.send(command).is_err() {
                break;
            }
        }
    });
    let mut instance: Option<BrowserInstance> = None;
    let mut last_sequence = 0;
    let mut last_header = None;
    'run: loop {
        if output.failed.load(Ordering::Acquire) {
            break;
        }
        match rx.try_recv() {
            Ok(command) => {
                let header = command.header;
                if matches!(command.kind, api::CommandKind::Open { .. }) {
                    if let Some(old) = instance.take() {
                        if header.session != old.document.header.session
                            || header.document <= old.document.header.document
                        {
                            break;
                        }
                        if let Err(message) = close(old) {
                            eprintln!("{message}");
                            // Do not initialize a replacement or call cef_shutdown with a live
                            // browser. Returning exits the broker; the host reaps its entire tree.
                            return 70;
                        }
                    }
                    last_sequence = header.sequence;
                    last_header = Some(header);
                    match open(command, output.clone()) {
                        Ok(new) => instance = Some(new),
                        Err(message) => {
                            output.emit(
                                header,
                                api::EventKind::Error {
                                    code: api::ErrorCode::Runtime,
                                    message,
                                },
                            );
                            break;
                        }
                    }
                } else if let Some(current) = instance.as_ref() {
                    if header.session != current.document.header.session
                        || header.document != current.document.header.document
                        || header.sequence <= last_sequence
                    {
                        break;
                    }
                    last_sequence = header.sequence;
                    if matches!(command.kind, api::CommandKind::Close) {
                        break 'run;
                    }
                    input::command(current, command);
                } else {
                    break;
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(2));
    }
    if let Some(current) = instance
        && let Err(message) = close(current)
    {
        eprintln!("{message}");
        return 70;
    }
    if let Some(header) = last_header {
        output.emit(header, api::EventKind::Closed);
    }
    if !output.failed.load(Ordering::Acquire) {
        let _ = writer.join();
    }
    shutdown();
    0
}
