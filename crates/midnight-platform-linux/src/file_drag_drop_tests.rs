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

#[derive(Default)]
struct Sink(Mutex<Vec<FileDropRequest>>, AtomicBool, AtomicBool);
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
    session: Box<dyn FileDragDropSession>,
    targets: SharedDropTargets,
    sink: Arc<Sink>,
    lifetime: Cancellation,
}
impl Fixture {
    fn new() -> Self {
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
        let session = NativeFileDragDrop
            .open_session(
                DragHost {
                    generation: 7,
                    native: NativeDragWindow::X11 { window: host },
                    lifetime: lifetime.clone(),
                },
                targets.clone(),
                sink.clone(),
            )
            .unwrap();
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
        self.connection
            .set_selection_owner(self.source, self.atom("XdndSelection"), x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        self.send(
            "XdndEnter",
            [self.source, 5 << 24, self.atom("text/uri-list"), 0, 0],
        );
        // Host root origin (100,200), client point (35,45) inside the right panel.
        self.send(
            "XdndPosition",
            [self.source, 0, (135 << 16) | 245, 0, self.atom(action)],
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
    fn payload(&mut self, bytes: &[u8]) -> [u32; 5] {
        let request = self.request_drop();
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
                    property: request.property,
                },
            )
            .unwrap()
            .check()
            .unwrap();
        self.message("XdndFinished")
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
}

#[test]
#[ignore = "requires isolated Xvfb DISPLAY and MIDNIGHT_DND_TEST_ISOLATED=1"]
fn x11_drop_rejects_target_navigation_between_position_and_selection() {
    let mut f = Fixture::new();
    f.offer("XdndActionCopy");
    let request = f.request_drop();
    f.targets.lock().unwrap().revision += 1;
    f.connection
        .change_property8(
            PropMode::REPLACE,
            request.requestor,
            request.property,
            request.target,
            b"file:///tmp/good\r\n",
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
    assert!(f.sink.0.lock().unwrap().is_empty());
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
    f.offer("XdndActionCopy");
    f.lifetime.cancel();
    f.send("XdndDrop", [f.source, 0, 0, 0, 0]);
    f.session.service_pending().unwrap();
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
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
