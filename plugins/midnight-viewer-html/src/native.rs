//! Audited CEF boundary; no browser objects or native buffers cross the public protocol.
use cef::*;
use midnight_web_viewer_api as api;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[path = "navigation.rs"]
mod admission;
mod handlers;
mod input;
mod navigation;
mod resources;

use crate::output::Output;

/// Longest wait between CEF message-loop pumps; a command wakes the loop at once.
const PUMP_INTERVAL: Duration = Duration::from_millis(2);
/// Commands handled before the next pump, so a burst of resource replies or input
/// costs one turn instead of one pump plus a fixed sleep each.
const MAX_COMMANDS_PER_TURN: usize = 64;

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
    requests: Mutex<resources::Requests>,
    selection: Mutex<crate::selection::Selection>,
    mouse_buttons: AtomicU64,
    wheel_remainder: Mutex<[f64; 2]>,
    host_shortcuts: bool,
    navigation: Mutex<admission::Navigation>,
    bootstrap: AtomicBool,
}

impl Document {
    fn new(
        header: api::Header,
        source: api::Resource,
        policy: crate::policy::DocumentPolicy,
        viewport: api::Viewport,
        output: Arc<Output>,
        host_shortcuts: bool,
    ) -> Result<Self, &'static str> {
        let navigation =
            admission::Navigation::new(header.session, header.document, source.clone());
        Ok(Self {
            header,
            source,
            policy,
            viewport: Mutex::new(viewport),
            surface: Mutex::new(crate::surface::Surface::new(
                viewport.revision,
                viewport.width,
                viewport.height,
            )?),
            layers: Mutex::new(crate::surface::Layers::default()),
            output,
            active: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            finding: Mutex::new(crate::find::FindQueue::default()),
            request_sequence: AtomicU64::new(1),
            requests: Mutex::new(resources::Requests::default()),
            selection: Mutex::new(crate::selection::Selection::default()),
            mouse_buttons: AtomicU64::new(0),
            wheel_remainder: Mutex::new([0.0; 2]),
            host_shortcuts,
            navigation: Mutex::new(navigation),
            bootstrap: AtomicBool::new(true),
        })
    }
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
    fn next_request_id(&self) -> Option<u64> {
        self.request_sequence
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()
            .filter(|id| *id != 0)
    }
    fn retire(&self) {
        self.active.store(false, Ordering::Release);
        if let Ok(mut navigation) = self.navigation.lock() {
            navigation.close();
            if let Some(attempt) = navigation.unclaimed_allowed_attempt() {
                // Claimed response owners retain their bytes until their actual release.
                navigation.response_finished(attempt);
            }
        }
        let streams = self
            .requests
            .lock()
            .map(|mut requests| requests.drain())
            .unwrap_or_default();
        for stream in streams {
            let (callback, response) = if let Ok(mut stream) = stream.lock() {
                stream.failed = true;
                (stream.callback.take(), stream.admitted.take())
            } else {
                (None, None)
            };
            // Cancel can call back into this document; release both locks first.
            if let Some(callback) = callback {
                callback.cancel();
            }
            drop(response);
        }
        self.output.discard_presentation();
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
    let document = Arc::new(Document::new(
        command.header,
        source,
        crate::policy::DocumentPolicy::new(
            command.header.document,
            policy.network,
            policy.scripts,
        )?,
        viewport,
        output,
        crate::host_keys::enabled(std::env::var(api::HOST_SHORTCUTS_ENV).ok().as_deref()),
    )?);
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

/// Live browser and the command identity accepted so far.
#[derive(Default)]
struct Session {
    instance: Option<BrowserInstance>,
    last_sequence: u64,
    last_header: Option<api::Header>,
}

enum Flow {
    Continue,
    /// Stop accepting commands; the common exit path closes the live browser.
    Stop,
    /// Exit at once without shutting CEF down (a browser could not be closed).
    Exit(i32),
}

fn dispatch(session: &mut Session, command: api::Command, output: &Arc<Output>) -> Flow {
    let header = command.header;
    #[cfg(feature = "diagnostic-timing")]
    crate::trace::record(
        crate::trace::Event::CommandDispatch,
        header.document,
        header.sequence,
    );
    if matches!(command.kind, api::CommandKind::Open { .. }) {
        // Reject a stale or foreign Open before taking the live browser, so the common
        // exit path still retires it and waits for OnBeforeClose before cef::shutdown.
        if let Some(old) = session.instance.as_ref()
            && (header.session != old.document.header.session
                || header.document <= old.document.header.document)
        {
            return Flow::Stop;
        }
        if let Some(old) = session.instance.take()
            && let Err(message) = close(old)
        {
            eprintln!("{message}");
            // Do not initialize a replacement or call cef_shutdown with a live
            // browser. Returning exits the broker; the host reaps its entire tree.
            return Flow::Exit(70);
        }
        session.last_sequence = header.sequence;
        session.last_header = Some(header);
        match open(command, output.clone()) {
            Ok(new) => session.instance = Some(new),
            Err(message) => {
                output.emit(
                    header,
                    api::EventKind::Error {
                        code: api::ErrorCode::Runtime,
                        message,
                    },
                );
                return Flow::Stop;
            }
        }
        Flow::Continue
    } else if let Some(current) = session.instance.as_ref() {
        if header.session != current.document.header.session
            || header.document != current.document.header.document
            || header.sequence <= session.last_sequence
        {
            return Flow::Stop;
        }
        session.last_sequence = header.sequence;
        if matches!(command.kind, api::CommandKind::Close) {
            return Flow::Stop;
        }
        input::command(current, command);
        Flow::Continue
    } else {
        Flow::Stop
    }
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
    // Browser process only, before CEF can start children that inherit stdout.
    let protocol = match crate::platform::protocol_output() {
        Ok(protocol) => protocol,
        Err(error) => {
            eprintln!("HTML viewer cannot reserve its protocol output: {error}");
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
        let mut stdout = protocol;
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
            // Events were validated when queued, so an error here is a broken transport.
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
            #[cfg(feature = "diagnostic-timing")]
            crate::trace::record(
                crate::trace::Event::CommandReceived,
                command.header.document,
                command.header.sequence,
            );
            if tx.send(command).is_err() {
                break;
            }
        }
    });
    let mut session = Session::default();
    'run: loop {
        if output.failed.load(Ordering::Acquire) {
            break;
        }
        let mut next = match rx.recv_timeout(PUMP_INTERVAL) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        #[cfg(feature = "diagnostic-timing")]
        let mut last_command = None;
        let mut handled = 0;
        while let Some(command) = next.take() {
            #[cfg(feature = "diagnostic-timing")]
            {
                last_command = Some(command.header);
            }
            match dispatch(&mut session, command, &output) {
                Flow::Continue => {}
                Flow::Stop => break 'run,
                Flow::Exit(code) => return code,
            }
            handled += 1;
            if handled < MAX_COMMANDS_PER_TURN {
                next = match rx.try_recv() {
                    Ok(command) => Some(command),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => break 'run,
                };
            }
        }
        if let Some(current) = session.instance.as_ref() {
            navigation::drive(current);
        }
        do_message_loop_work();
        #[cfg(feature = "diagnostic-timing")]
        if let Some(header) = last_command {
            crate::trace::record(
                crate::trace::Event::CommandTurnComplete,
                header.document,
                header.sequence,
            );
        }
    }
    if let Some(current) = session.instance.take()
        && let Err(message) = close(current)
    {
        eprintln!("{message}");
        return 70;
    }
    if let Some(header) = session.last_header {
        output.emit(header, api::EventKind::Closed);
    }
    // The writer owns an Output (and therefore the senders), so it would otherwise
    // wait forever when no Closed is queued: stdin ended, or a non-Open came first.
    output.finish();
    if !output.failed.load(Ordering::Acquire) {
        let _ = writer.join();
    }
    shutdown();
    0
}
