use super::*;
use midnight_core::*;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, CreateWindowAux, PropMode, WindowClass},
};

/// Accepted requests, a saturated queue switch, the hover state and refusal reports.
#[derive(Default)]
struct Sink(
    Mutex<Vec<FileDropRequest>>,
    AtomicBool,
    AtomicBool,
    Mutex<Vec<String>>,
);
impl FileDropSink for Sink {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        if self.1.load(Ordering::Acquire) {
            return Err(failure("queue full"));
        }
        self.0.lock().unwrap().push(request);
        Ok(())
    }
    fn hover(&self, pane: Option<PaneId>) {
        self.2.store(pane.is_some(), Ordering::Release);
    }
    fn reject(&self, error: FsError) {
        self.3.lock().unwrap().push(error.message);
    }
}

/// The one fixed report for a dropped list that cannot be decoded.
const REFUSED_LIST: &str = "The dropped file list is empty, malformed, names non-local files, or is longer than 4096 items.";
/// The Windows target's fixed reports, and the one for an over-budget list.
const CHANGED: &str = "The drop destination changed.";
const MISSING: &str = "The drag source did not provide its file list.";
const TOO_LARGE: &str = "The dropped file list exceeds the 4 MiB limit.";

/// Elapses only when the input or wake that should end a wait does not.
const FAILURE_WAIT: Duration = Duration::from_secs(30);

/// Whether a wait that began at `started` ended well within `FAILURE_WAIT`.
fn prompt(started: Instant) -> bool {
    started.elapsed() < FAILURE_WAIT / 2
}

use x11rb::{
    protocol::{
        Event,
        xproto::{self, ClientMessageEvent, EventMask, SelectionNotifyEvent},
    },
    rust_connection::RustConnection,
};

struct Fixture {
    connection: RustConnection,
    host: u32,
    source: u32,
    proxy: u32,
    session: Session,
    targets: SharedDropTargets,
    sink: Arc<Sink>,
    lifetime: Cancellation,
}
impl Fixture {
    fn new() -> Self {
        Self::build(None)
    }
    /// A session whose incoming file-list reads time out after `timeout`.
    fn with_read_timeout(timeout: Duration) -> Self {
        Self::build(Some(timeout))
    }
    fn build(read_timeout: Option<Duration>) -> Self {
        assert_eq!(
            std::env::var("MIDNIGHT_DND_TEST_ISOLATED").as_deref(),
            Ok("1")
        );
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        let host = connection.generate_id().unwrap();
        let source = connection.generate_id().unwrap();
        for window in [host, source] {
            connection
                .create_window(
                    0,
                    window,
                    root,
                    100,
                    200,
                    300,
                    200,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .unwrap()
                .check()
                .unwrap();
        }
        connection.map_window(host).unwrap().check().unwrap();
        let targets = Arc::new(Mutex::new(DropTargets {
            host_generation: 7,
            revision: 9,
            targets: vec![FileDropTarget {
                pane: PaneId::Right,
                navigation: NavigationId(11),
                directory: Arc::new("/tmp/destination 文档".into()),
                rect: DropRect {
                    x: 30,
                    y: 40,
                    width: 100,
                    height: 80,
                },
            }],
        }));
        let sink = Arc::new(Sink::default());
        let lifetime = Cancellation::default();
        let drag_host = DragHost {
            generation: 7,
            native: NativeDragWindow::X11 { window: host },
            lifetime: lifetime.clone(),
        };
        // `x11_registers_verified_proxy_and_restores_previous_host_properties` opens
        // sessions through `open_session`; tests reach this one's connection directly.
        let mut session = Session::new(drag_host, host, targets.clone(), sink.clone()).unwrap();
        if let Some(timeout) = read_timeout {
            session.read_timeout = timeout;
        }
        let proxy_atom = connection
            .intern_atom(false, b"XdndProxy")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let proxy = connection
            .get_property(false, host, proxy_atom, AtomEnum::WINDOW, 0, 1)
            .unwrap()
            .reply()
            .unwrap()
            .value32()
            .unwrap()
            .next()
            .unwrap();
        Self {
            connection,
            host,
            source,
            proxy,
            session,
            targets,
            sink,
            lifetime,
        }
    }
    fn atom(&self, name: &str) -> u32 {
        self.connection
            .intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    }
    fn send(&self, name: &str, data: [u32; 5]) {
        self.connection
            .send_event(
                false,
                self.proxy,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, self.host, self.atom(name), data),
            )
            .unwrap()
            .check()
            .unwrap();
    }
    fn next(&mut self, predicate: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(
                Instant::now() < deadline,
                "native protocol response timed out"
            );
            self.session.service_pending().unwrap();
            while let Some(event) = self.connection.poll_for_event().unwrap() {
                if predicate(&event) {
                    return event;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn message(&mut self, name: &str) -> [u32; 5] {
        let atom = self.atom(name);
        let Event::ClientMessage(event) =
            self.next(|e| matches!(e, Event::ClientMessage(e) if e.type_ == atom))
        else {
            unreachable!()
        };
        event.data.as_data32()
    }
    fn offer(&mut self, action: &str) -> [u32; 5] {
        // Host root origin (100,200), client point (35,45) inside the right panel.
        self.offer_at(action, (135 << 16) | 245)
    }
    /// Enters and moves to the packed root position `position`; returns the status.
    fn offer_at(&mut self, action: &str, position: u32) -> [u32; 5] {
        self.connection
            .set_selection_owner(self.source, self.atom("XdndSelection"), x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        self.send(
            "XdndEnter",
            [self.source, 5 << 24, self.atom("text/uri-list"), 0, 0],
        );
        self.send(
            "XdndPosition",
            [self.source, 0, position, 0, self.atom(action)],
        );
        self.message("XdndStatus")
    }
    fn request_drop(&mut self) -> xproto::SelectionRequestEvent {
        self.send("XdndDrop", [self.source, 0, 0, 0, 0]);
        let Event::SelectionRequest(request) =
            self.next(|e| matches!(e, Event::SelectionRequest(_)))
        else {
            unreachable!()
        };
        request
    }
    /// Answers `request` with `bytes`, as the drag source does.
    fn answer(&self, request: &xproto::SelectionRequestEvent, bytes: &[u8]) {
        self.connection
            .change_property8(
                PropMode::REPLACE,
                request.requestor,
                request.property,
                request.target,
                bytes,
            )
            .unwrap()
            .check()
            .unwrap();
        self.notify(request, request.property);
    }
    /// Sends the SelectionNotify that ends `request`, naming `property` (NONE refuses).
    fn notify(&self, request: &xproto::SelectionRequestEvent, property: u32) {
        self.connection
            .send_event(
                false,
                request.requestor,
                EventMask::NO_EVENT,
                SelectionNotifyEvent {
                    response_type: xproto::SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property,
                },
            )
            .unwrap()
            .check()
            .unwrap();
    }
    fn payload(&mut self, bytes: &[u8]) -> [u32; 5] {
        let request = self.request_drop();
        self.answer(&request, bytes);
        self.message("XdndFinished")
    }
    fn reports(&self) -> Vec<String> {
        self.sink.3.lock().unwrap().clone()
    }
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_preserves_raw_names_and_uses_physical_client_target() {
    use std::os::unix::ffi::OsStrExt;
    let mut f = Fixture::new();
    let clipboard = f.atom("CLIPBOARD");
    f.connection
        .set_selection_owner(f.source, clipboard, x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(f.offer("XdndActionCopy")[1] & 1, 1);
    let finish = f.payload(b"file:///tmp/raw-%FF-%E6%96%87.txt\r\n");
    assert_eq!(finish[1] & 1, 1);
    assert_eq!(finish[2], f.atom("XdndActionCopy"));
    let requests = f.sink.0.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].files.paths[0].as_os_str().as_bytes(),
        b"/tmp/raw-\xff-\xe6\x96\x87.txt"
    );
    assert_eq!(requests[0].target.pane, PaneId::Right);
    assert_eq!(requests[0].targets_revision, 9);
    assert_eq!(
        f.connection
            .get_selection_owner(clipboard)
            .unwrap()
            .reply()
            .unwrap()
            .owner,
        f.source
    );
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_rejects_move_queue_saturation_and_malformed_or_oversized_payloads() {
    let mut f = Fixture::new();
    assert_eq!(f.offer("XdndActionMove")[1] & 1, 0);
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    // The Move-only offer never showed a Copy status: nothing to explain.
    assert!(f.sink.3.lock().unwrap().is_empty());
    f.sink.1.store(true, Ordering::Release);
    assert_eq!(f.offer("XdndActionCopy")[1] & 1, 1);
    assert_eq!(f.payload(b"file:///tmp/good\r\n")[1] & 1, 0);
    f.sink.1.store(false, Ordering::Release);
    for bytes in [
        b"file:///tmp/good\nfile://evil/share\n".as_slice(),
        b"file:///tmp/%ZZ\n",
        b"file:///tmp/%00\n",
    ] {
        f.offer("XdndActionCopy");
        assert_eq!(f.payload(bytes)[1] & 1, 0);
    }
    assert!(f.sink.0.lock().unwrap().is_empty());
    // One report per refused Copy drop: the admission refusal, then the fixed list text.
    assert_eq!(
        *f.sink.3.lock().unwrap(),
        ["queue full", REFUSED_LIST, REFUSED_LIST, REFUSED_LIST]
    );
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_rejects_target_navigation_between_position_and_selection() {
    let mut f = Fixture::new();
    // A republish of the same destination while the list is read (a refresh, say)
    // keeps the drop.
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.targets.lock().unwrap().revision += 1;
    f.answer(&request, b"file:///tmp/good\r\n");
    assert_eq!(f.message("XdndFinished")[1] & 1, 1);
    assert_eq!(f.sink.0.lock().unwrap()[0].targets_revision, 10);
    // The pane navigated elsewhere: the user released over a folder no longer shown.
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    {
        let mut targets = f.targets.lock().unwrap();
        targets.revision += 1;
        targets.targets[0].navigation = NavigationId(12);
    }
    f.answer(&request, b"file:///tmp/good\r\n");
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    assert_eq!(f.reports(), [CHANGED]);
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_goes_to_the_republished_destination_and_reports_one_that_is_gone() {
    let mut f = Fixture::new();
    // Republished since the Copy status, unchanged at the drop position: admitted to
    // the current revision, as the Windows target re-resolves at Drop.
    assert_eq!(f.offer("XdndActionCopy")[1] & 1, 1);
    f.targets.lock().unwrap().revision += 1;
    assert_eq!(f.payload(b"file:///tmp/good\r\n")[1] & 1, 1);
    assert_eq!(f.sink.0.lock().unwrap()[0].targets_revision, 10);
    assert!(f.reports().is_empty());
    // The pane withdrew its target since the Copy status (it is loading); the session
    // notices that before the drop arrives.
    assert_eq!(f.offer("XdndActionCopy")[1] & 1, 1);
    {
        let mut targets = f.targets.lock().unwrap();
        targets.revision += 1;
        targets.targets.clear();
    }
    f.session.service_pending().unwrap();
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    assert_eq!(f.reports(), [CHANGED], "reported once");
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_reports_a_source_that_does_not_deliver_its_file_list() {
    let mut f = Fixture::new();
    // The drag source lost the selection before the drop.
    f.offer("XdndActionCopy");
    f.connection
        .set_selection_owner(x11rb::NONE, f.atom("XdndSelection"), x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    // The source refuses to convert its selection.
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.notify(&request, x11rb::NONE);
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    // A new drag enters before the dropped list arrived.
    f.offer("XdndActionCopy");
    f.request_drop();
    f.send(
        "XdndEnter",
        [f.source, 5 << 24, f.atom("text/uri-list"), 0, 0],
    );
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    assert!(f.sink.0.lock().unwrap().is_empty());
    assert_eq!(f.reports(), [MISSING, MISSING, MISSING]);
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_reports_a_list_read_that_times_out_once() {
    // Every read times out at the session's next pass.
    let mut f = Fixture::with_read_timeout(Duration::ZERO);
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    assert_eq!(f.reports(), [MISSING]);
    // The late answer finds no read. The next offer's status follows it, so the
    // session has handled the answer by then.
    f.answer(&request, b"file:///tmp/late\r\n");
    f.offer("XdndActionCopy");
    assert!(f.sink.0.lock().unwrap().is_empty());
    assert_eq!(f.reports(), [MISSING], "no second report");
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_without_a_copy_status_or_after_the_window_closed_is_refused_silently() {
    let mut f = Fixture::new();
    // Outside both panels: the source never saw the Copy status.
    assert_eq!(
        f.offer_at("XdndActionCopy", (10 << 16) | 210)[1] & 1,
        0,
        "no target there"
    );
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    // The window closes while the dropped list is read: nobody is left to tell.
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.lifetime.cancel();
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    f.answer(&request, b"file:///tmp/good\r\n");
    f.session.service_pending().unwrap();
    assert!(f.sink.0.lock().unwrap().is_empty());
    assert!(f.reports().is_empty());
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_incremental_drop_is_bounded_and_window_cancellation_refuses_admission() {
    use x11rb::protocol::xproto::Property;
    let mut f = Fixture::new();
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.connection
        .change_window_attributes(
            request.requestor,
            &xproto::ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .change_property32(
            PropMode::REPLACE,
            request.requestor,
            request.property,
            f.atom("INCR"),
            &[64],
        )
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .send_event(
            false,
            request.requestor,
            EventMask::NO_EVENT,
            SelectionNotifyEvent {
                response_type: xproto::SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: request.time,
                requestor: request.requestor,
                selection: request.selection,
                target: request.target,
                property: request.property,
            },
        )
        .unwrap()
        .check()
        .unwrap();
    for bytes in [b"file:///tmp/raw-%".as_slice(), b"FF\r\n", b""] {
        f.next(|e| matches!(e, Event::PropertyNotify(e) if e.window == request.requestor && e.atom == request.property && e.state == Property::DELETE));
        f.connection
            .change_property8(
                PropMode::REPLACE,
                request.requestor,
                request.property,
                request.target,
                bytes,
            )
            .unwrap()
            .check()
            .unwrap();
    }
    assert_eq!(f.message("XdndFinished")[1] & 1, 1);
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.connection
        .change_property32(
            PropMode::REPLACE,
            request.requestor,
            request.property,
            f.atom("INCR"),
            &[(MAX_FILE_CLIPBOARD_BYTES + 1) as u32],
        )
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .send_event(
            false,
            request.requestor,
            EventMask::NO_EVENT,
            SelectionNotifyEvent {
                response_type: xproto::SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: request.time,
                requestor: request.requestor,
                selection: request.selection,
                target: request.target,
                property: request.property,
            },
        )
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(f.message("XdndFinished")[1] & 1, 0);
    assert_eq!(f.reports(), [TOO_LARGE]);
    f.offer("XdndActionCopy");
    f.lifetime.cancel();
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    f.session.service_pending().unwrap();
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    assert_eq!(f.reports(), [TOO_LARGE], "a closed window adds no report");
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_source_requires_live_button_and_copies_to_external_target_without_clipboard_change() {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let mut f = Fixture::new();
    let files = FileClipboard {
        paths: vec!["/tmp/a 文#.txt".into()],
        intent: FileClipboardIntent::Copy,
    };
    assert_eq!(
        f.session
            .start_drag(&files, &Cancellation::default())
            .unwrap(),
        DragFinish::Cancelled
    );
    let clipboard = f.atom("CLIPBOARD");
    f.connection
        .set_selection_owner(f.source, clipboard, x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    let root = f.connection.setup().roots[0].root;
    f.connection
        .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 135, 245)
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)
        .unwrap()
        .check()
        .unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let peer = thread::spawn(move || {
        let (c, screen) = x11rb::connect(None).unwrap();
        let root = c.setup().roots[screen].root;
        let target = c.generate_id().unwrap();
        c.create_window(
            0,
            target,
            root,
            500,
            200,
            200,
            200,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .unwrap()
        .check()
        .unwrap();
        c.map_window(target).unwrap().check().unwrap();
        let atom = |name: &[u8]| c.intern_atom(false, name).unwrap().reply().unwrap().atom;
        let aware = atom(b"XdndAware");
        let position = atom(b"XdndPosition");
        let status = atom(b"XdndStatus");
        let drop_atom = atom(b"XdndDrop");
        let finished = atom(b"XdndFinished");
        let copy = atom(b"XdndActionCopy");
        let selection = atom(b"XdndSelection");
        let uri = atom(b"text/uri-list");
        let property = atom(b"PEER_FILE_DROP");
        c.change_property32(PropMode::REPLACE, target, aware, AtomEnum::ATOM, &[5])
            .unwrap()
            .check()
            .unwrap();
        c.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 550, 250)
            .unwrap()
            .check()
            .unwrap();
        ready_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut source = 0;
        let result = loop {
            assert!(Instant::now() < deadline, "outbound XDND timed out");
            let Some(event) = c.poll_for_event().unwrap() else {
                thread::sleep(Duration::from_millis(2));
                continue;
            };
            match event {
                Event::ClientMessage(e) if e.type_ == position => {
                    let data = e.data.as_data32();
                    source = data[0];
                    assert_eq!(data[4], copy);
                    c.send_event(
                        false,
                        source,
                        EventMask::NO_EVENT,
                        ClientMessageEvent::new(32, source, status, [target, 3, 0, 0, copy]),
                    )
                    .unwrap()
                    .check()
                    .unwrap();
                    c.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)
                        .unwrap()
                        .check()
                        .unwrap();
                }
                Event::ClientMessage(e) if e.type_ == drop_atom => {
                    c.convert_selection(target, selection, uri, property, e.data.as_data32()[2])
                        .unwrap()
                        .check()
                        .unwrap();
                }
                Event::SelectionNotify(e) => {
                    assert_eq!(e.property, property);
                    let bytes = c
                        .get_property(true, target, property, uri, 0, 1000)
                        .unwrap()
                        .reply()
                        .unwrap()
                        .value;
                    c.send_event(
                        false,
                        source,
                        EventMask::NO_EVENT,
                        ClientMessageEvent::new(32, source, finished, [target, 1, copy, 0, 0]),
                    )
                    .unwrap()
                    .check()
                    .unwrap();
                    break bytes;
                }
                _ => {}
            }
        };
        c.destroy_window(target).unwrap().check().unwrap();
        result
    });
    ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let outcome = f.session.start_drag(&files, &Cancellation::default());
    assert_eq!(
        peer.join().unwrap(),
        b"file:///tmp/a%20%E6%96%87%23.txt\r\n"
    );
    assert_eq!(outcome.unwrap(), DragFinish::Copied);
    assert_eq!(
        f.connection
            .get_selection_owner(clipboard)
            .unwrap()
            .reply()
            .unwrap()
            .owner,
        f.source
    );
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_source_and_target_share_proxy_for_incremental_internal_panel_copy() {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let mut f = Fixture::new();
    let files = FileClipboard {
        paths: (0..1000)
            .map(|index| format!("/tmp/{index}-{}", "a".repeat(90)).into())
            .collect(),
        intent: FileClipboardIntent::Copy,
    };
    let root = f.connection.setup().roots[0].root;
    f.connection
        .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 135, 245)
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)
        .unwrap()
        .check()
        .unwrap();
    let sink = f.sink.clone();
    let release = thread::spawn(move || {
        let (c, _) = x11rb::connect(None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !sink.2.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "own proxy was not targeted");
            thread::sleep(Duration::from_millis(2));
        }
        c.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    });
    let result = f.session.start_drag(&files, &Cancellation::default());
    release.join().unwrap();
    assert_eq!(result.unwrap(), DragFinish::Copied);
    let requests = f.sink.0.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].files.paths.len(), 1000);
    assert_eq!(
        requests[0].files.paths[0],
        std::path::PathBuf::from(format!("/tmp/0-{}", "a".repeat(90)))
    );
}

#[test]
fn xdnd_position_preserves_negative_monitor_coordinates() {
    assert_eq!(unpack_position(0xffff_fffe), (-1, -2));
    assert_eq!(unpack_position(0x8000_7fff), (-32768, 32767));
    assert_eq!(unpack_position(0x0023_002d), (35, 45));
}

#[test]
fn refused_drop_lists_are_reported_in_one_fixed_text_without_source_bytes() {
    let candidate = Candidate {
        revision: 9,
        target: FileDropTarget {
            pane: PaneId::Right,
            navigation: NavigationId(11),
            directory: Arc::new("/tmp/destination 文档".into()),
            rect: DropRect {
                x: 30,
                y: 40,
                width: 100,
                height: 80,
            },
        },
    };
    let request = selection::drop_request(b"# list\r\nfile:///tmp/a%20b\r\n", &candidate).unwrap();
    assert_eq!(request.files.paths, [std::path::PathBuf::from("/tmp/a b")]);
    assert_eq!(request.targets_revision, 9);
    let mut too_many = Vec::new();
    for index in 0..=MAX_FILE_CLIPBOARD_PATHS {
        too_many.extend_from_slice(format!("file:///tmp/{index}\n").as_bytes());
    }
    for bytes in [
        b"".as_slice(),
        b"# comments only\n",
        b"https://secret.example/payload\n",
        b"file://secret-host/share\n",
        b"file:///tmp/%ZZ-secret\n",
        b"file:///tmp/%00-secret\n",
        b"file:///tmp/unescaped secret\n",
        too_many.as_slice(),
    ] {
        let refusal = selection::drop_request(bytes, &candidate).unwrap_err();
        assert_eq!(refusal.message, REFUSED_LIST);
        assert!(!refusal.to_string().contains("secret"), "{refusal}");
    }
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_target_negotiates_copy_from_move_proposal_only_with_explicit_bounded_copy_offer() {
    let mut f = Fixture::new();
    let copy = f.atom("XdndActionCopy");
    let move_action = f.atom("XdndActionMove");
    let list = f.atom("XdndActionList");
    f.connection
        .change_property32(
            PropMode::REPLACE,
            f.source,
            list,
            AtomEnum::ATOM,
            &[copy, move_action],
        )
        .unwrap()
        .check()
        .unwrap();
    let status = f.offer("XdndActionMove");
    assert_eq!(
        status[1] & 1,
        1,
        "a peer offering Copy and Move can safely negotiate Copy"
    );
    assert_eq!(status[4], copy);
    let finished = f.payload(b"file:///tmp/retained-source\r\n");
    assert_eq!(finished[1] & 1, 1);
    assert_eq!(finished[2], copy);
    assert_eq!(
        f.sink.0.lock().unwrap()[0].files.intent,
        FileClipboardIntent::Copy
    );
    for actions in [vec![move_action], vec![copy; 17], vec![]] {
        f.connection
            .change_property32(PropMode::REPLACE, f.source, list, AtomEnum::ATOM, &actions)
            .unwrap()
            .check()
            .unwrap();
        let status = f.offer("XdndActionMove");
        assert_eq!(status[1] & 1, 0);
        assert_eq!(status[4], x11rb::NONE);
    }
    f.connection
        .change_property8(
            PropMode::REPLACE,
            f.source,
            list,
            AtomEnum::STRING,
            b"XdndActionCopy",
        )
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(f.offer("XdndActionMove")[1] & 1, 0);
    f.connection
        .delete_property(f.source, list)
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(f.offer("XdndActionMove")[1] & 1, 0);
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_escape_cancels_drag_and_releases_native_grabs() {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let mut f = Fixture::new();
    let root = f.connection.setup().roots[0].root;
    f.connection
        .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 135, 245)
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)
        .unwrap()
        .check()
        .unwrap();
    let sink = f.sink.clone();
    let cancel = thread::spawn(move || {
        let (c, _) = x11rb::connect(None).unwrap();
        let first = c.setup().min_keycode;
        let count = c.setup().max_keycode - first + 1;
        let keys = c
            .get_keyboard_mapping(first, count)
            .unwrap()
            .reply()
            .unwrap();
        let escape = keys
            .keysyms
            .chunks(keys.keysyms_per_keycode as usize)
            .position(|key| key.contains(&0xff1b))
            .unwrap() as u8
            + first;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !sink.2.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        c.xtest_fake_input(xproto::KEY_PRESS_EVENT, escape, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
        c.xtest_fake_input(xproto::KEY_RELEASE_EVENT, escape, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    });
    let result = f.session.start_drag(
        &FileClipboard {
            paths: vec!["/tmp/source".into()],
            intent: FileClipboardIntent::Copy,
        },
        &Cancellation::default(),
    );
    cancel.join().unwrap();
    f.connection
        .xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(result.unwrap(), DragFinish::Cancelled);
    assert!(f.sink.0.lock().unwrap().is_empty());
    assert_eq!(
        f.connection
            .get_selection_owner(f.atom("XdndSelection"))
            .unwrap()
            .reply()
            .unwrap()
            .owner,
        x11rb::NONE
    );
    let status = f
        .connection
        .grab_pointer(
            false,
            f.host,
            EventMask::BUTTON_RELEASE,
            xproto::GrabMode::ASYNC,
            xproto::GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    assert_eq!(status, xproto::GrabStatus::SUCCESS);
    f.connection
        .ungrab_pointer(x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_source_waits_for_ui_connection_pointer_handoff_without_blocking_ui() {
    use x11rb::protocol::xtest::ConnectionExt as _;
    let mut f = Fixture::new();
    let root = f.connection.setup().roots[0].root;
    f.connection
        .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 135, 245)
        .unwrap()
        .check()
        .unwrap();
    f.connection
        .xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)
        .unwrap()
        .check()
        .unwrap();
    let held = f
        .connection
        .grab_pointer(
            false,
            f.host,
            EventMask::BUTTON_RELEASE,
            xproto::GrabMode::ASYNC,
            xproto::GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    assert_eq!(held, xproto::GrabStatus::SUCCESS);
    let cancel = Cancellation::default();
    let sink = f.sink.clone();
    let connection = &f.connection;
    let result = thread::scope(|scope| {
        let cancel_worker = cancel.clone();
        let release = scope.spawn(move || {
            thread::sleep(Duration::from_millis(40));
            connection
                .ungrab_pointer(x11rb::CURRENT_TIME)
                .unwrap()
                .check()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !sink.2.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            cancel_worker.cancel();
            connection
                .xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)
                .unwrap()
                .check()
                .unwrap();
        });
        let result = f.session.start_drag(
            &FileClipboard {
                paths: vec!["/tmp/handoff".into()],
                intent: FileClipboardIntent::Copy,
            },
            &cancel,
        );
        release.join().unwrap();
        result
    });
    assert_eq!(
        result.unwrap(),
        DragFinish::Cancelled,
        "the source must survive the prior UI grab"
    );
}

#[test]
fn native_wayland_is_explicitly_unavailable() {
    let result = NativeFileDragDrop.open_session(
        DragHost {
            generation: 1,
            native: NativeDragWindow::Unsupported,
            lifetime: Cancellation::default(),
        },
        Arc::default(),
        Arc::new(Sink::default()),
    );
    assert!(result.is_err());
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_registers_verified_proxy_and_restores_previous_host_properties() {
    assert_eq!(
        std::env::var("MIDNIGHT_DND_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    // Read through a different X11 connection immediately after each teardown.
    // Repetition exposes a flush-only cleanup's cross-connection ordering race.
    for _ in 0..32 {
        assert_proxy_registration_restored();
    }
}

fn assert_proxy_registration_restored() {
    let (connection, screen) = x11rb::connect(None).unwrap();
    let window = connection.generate_id().unwrap();
    connection
        .create_window(
            0,
            window,
            connection.setup().roots[screen].root,
            10,
            20,
            200,
            200,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap()
        .check()
        .unwrap();
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .unwrap()
            .reply()
            .unwrap()
            .atom
    };
    let aware = atom(b"XdndAware");
    let proxy = atom(b"XdndProxy");
    connection
        .change_property32(PropMode::REPLACE, window, aware, AtomEnum::ATOM, &[4])
        .unwrap()
        .check()
        .unwrap();
    let property = |window, atom| {
        connection
            .get_property(false, window, atom, AtomEnum::ANY, 0, 4)
            .unwrap()
            .reply()
            .unwrap()
    };
    let session = NativeFileDragDrop
        .open_session(
            DragHost {
                generation: 1,
                native: NativeDragWindow::X11 { window },
                lifetime: Cancellation::default(),
            },
            Arc::default(),
            Arc::new(Sink::default()),
        )
        .expect("a real host window must acquire an XDND proxy");
    let proxy_window = property(window, proxy).value32().unwrap().next().unwrap();
    assert_ne!(window, proxy_window);
    assert_eq!(
        property(window, aware).type_,
        u32::from(AtomEnum::NONE),
        "legacy clients must not route file payloads into winit's UTF-8/canonicalizing parser"
    );
    assert_eq!(
        property(proxy_window, proxy)
            .value32()
            .unwrap()
            .collect::<Vec<_>>(),
        vec![proxy_window]
    );
    assert_eq!(
        property(proxy_window, aware)
            .value32()
            .unwrap()
            .collect::<Vec<_>>(),
        vec![5]
    );
    drop(session);
    assert_eq!(property(window, proxy).type_, u32::from(AtomEnum::NONE));
    assert_eq!(
        property(window, aware)
            .value32()
            .unwrap()
            .collect::<Vec<_>>(),
        vec![4]
    );
    connection.destroy_window(window).unwrap().check().unwrap();
}

impl Fixture {
    /// Sends the XdndEnter and the XdndPosition of a Copy drag over the right pane.
    fn enter_right_pane(&self) {
        let (uri, copy) = (self.atom("text/uri-list"), self.atom("XdndActionCopy"));
        self.send("XdndEnter", [self.source, 5 << 24, uri, 0, 0]);
        self.send("XdndPosition", [self.source, 0, (135 << 16) | 245, 0, copy]);
    }
    /// The data of the last `name` client message that the session sent the drag source
    /// so far. Interning `name` is a round trip, and x11rb queues the events that precede
    /// its reply; the session's sends are round trips too, so none is still in flight.
    fn received(&self, name: &str) -> Option<[u32; 5]> {
        let atom = self.atom(name);
        let mut last = None;
        while let Some(event) = self.connection.poll_for_event().unwrap() {
            if let Event::ClientMessage(event) = event
                && event.type_ == atom
            {
                last = Some(event.data.as_data32());
            }
        }
        last
    }
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_idle_wait_ends_for_its_waker_events_already_read_and_a_shown_destination() {
    let mut f = Fixture::new();
    // Serve the property changes that opening the session queued.
    f.session.service_pending().unwrap();
    let waker = f.session.waker().expect("the session has an eventfd");
    // A wake made before the wait ends it.
    waker();
    let started = Instant::now();
    f.session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "an earlier wake must end the next wait");
    // A drag enters. Its events precede the reply to a round trip on the session's
    // connection, so reading that reply queues them in x11rb and leaves the socket
    // empty: poll() alone would sleep through them, so the wait serves them first.
    f.enter_right_pane();
    f.session
        .connection
        .get_input_focus()
        .unwrap()
        .reply()
        .unwrap();
    let started = Instant::now();
    f.session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "events already read must end the wait");
    assert_eq!(
        f.received("XdndStatus").map(|status| status[1] & 1),
        Some(1),
        "the wait served the drag and accepted Copy"
    );
    assert!(f.sink.2.load(Ordering::Acquire), "the destination is shown");
    // No event marks the UI withdrawing that destination: while it is shown, the wait
    // ends in time for the next pass to recheck it.
    {
        let mut targets = f.targets.lock().unwrap();
        targets.revision += 1;
        targets.targets.clear();
    }
    let started = Instant::now();
    f.session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "a shown destination must be rechecked");
    f.session.service_pending().unwrap();
    assert!(
        !f.sink.2.load(Ordering::Acquire),
        "the withdrawn destination is no longer shown"
    );
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_idle_wait_ends_for_xdnd_input_arriving_after_the_queue_was_served() {
    let mut f = Fixture::new();
    // `wait_pending` first serves what x11rb queued or can read without blocking, until
    // nothing is left (here, the property changes that opening the session queued).
    f.session.service_pending().unwrap();
    while f.session.serve_events().unwrap() {}
    // Then another client's drag arrives. Each send is a round trip of the source's
    // connection, so the server has processed it, but the session has not read it.
    // Only poll() on the X11 connection can end the wait for it: no waker was called,
    // and without a shown destination the wait has no shorter limit.
    f.enter_right_pane();
    let started = Instant::now();
    f.session.wait_for_input(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "XDND input must end the wait in poll()");
    // The next pass serves it.
    f.session.service_pending().unwrap();
    assert_eq!(
        f.received("XdndStatus").map(|status| status[1] & 1),
        Some(1),
        "the session accepted Copy"
    );
    assert!(f.sink.2.load(Ordering::Acquire), "the destination is shown");
}

/// Measurement for review item C10-2: wakeups of the native drag actor's idle loop over
/// five seconds with a real X11 session, for the 10 ms pump the actor ran before and
/// for the session wait it runs now. It prints counts only. CI runs the ignored tests
/// of this module, so this one also needs `MIDNIGHT_DND_IDLE_MEASUREMENT=1`:
/// `xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" env
/// MIDNIGHT_DND_TEST_ISOLATED=1 MIDNIGHT_DND_IDLE_MEASUREMENT=1 "$executable"
/// file_drag_drop::tests::measure_idle_wakeups_of_an_open_session --ignored --nocapture`.
#[test]
#[ignore = "manual measurement: ten idle seconds; requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn measure_idle_wakeups_of_an_open_session() {
    if std::env::var("MIDNIGHT_DND_IDLE_MEASUREMENT").as_deref() != Ok("1") {
        return;
    }
    let mut f = Fixture::new();
    f.session.service_pending().unwrap();
    let measured = Duration::from_secs(5);
    let (_controller, signal) = std::sync::mpsc::sync_channel::<()>(1);
    let pumped = idle_wakeups(measured, |_| {
        f.session.service_pending().unwrap();
        // The actor's former pump: a 10 ms wait for a controller signal.
        let _ = signal.recv_timeout(Duration::from_millis(10));
    });
    let waited = idle_wakeups(measured, |left| {
        f.session.service_pending().unwrap();
        // The actor's idle wait, here until just past the window: only native input or
        // a spurious return ends it within the window.
        f.session
            .wait_pending(left + Duration::from_millis(50))
            .unwrap();
    });
    eprintln!(
        "idle wakeups in {measured:?} with an open X11 session: 10 ms pump {pumped}, session wait {waited}"
    );
}

/// Wakeups of an idle loop within `measured`: returns of `iteration`, which gets the
/// time left, before the window ended.
fn idle_wakeups(measured: Duration, mut iteration: impl FnMut(Duration)) -> u32 {
    let started = Instant::now();
    let mut wakeups = 0;
    loop {
        let left = measured.saturating_sub(started.elapsed());
        if left.is_zero() {
            return wakeups;
        }
        iteration(left);
        if started.elapsed() < measured {
            wakeups += 1;
        }
    }
}
