//! Opt-in real Chromium boundary qualification; never counts as mock rendering.
#![cfg(feature = "native-cef")]
use midnight_web_viewer_api::*;
use std::{
    io::Write,
    process::{Command as Process, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
#[path = "support/runtime_linux.rs"]
mod runtime_linux;

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

#[test]
#[ignore = "requires matching packaged CEF runtime and interactive Windows or Linux desktop"]
fn chromium_renders_captured_html_and_denies_default_scripts() {
    let executable =
        std::env::var_os("MIDNIGHT_HTML_TEST_EXECUTABLE").expect("set packaged bootstrap path");
    let profile =
        std::env::var_os("MIDNIGHT_WEB_VIEWER_PROFILE").expect("set owned private profile");
    let mut child = Process::new(executable)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", profile)
        .env(HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut stdout) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let html = format!("<!doctype html><html><head><title>CEF native fixture</title><link rel=stylesheet href=assets/style.css><style>html,body{{margin:0}}h1{{color:white}}#zoom{{position:absolute;top:100px;left:0;width:100px;height:20px;background:#ff0000}}select{{position:absolute;left:180px;top:100px;width:120px;height:20px}}</style></head><body><h1>Chromium fixture searchable</h1><div id=zoom></div><input style=position:absolute;left:0;top:70px;width:150px;height:20px value=abc><select><option>First</option><option>Second</option><option>Third</option></select><img src=http://{address}/denied><script>document.body.style.background='#00ff00';document.title='SCRIPT EXECUTED'</script></body></html>").into_bytes();
    let css = b"html,body{background:#123456}";
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
                width: 320,
                height: 200,
                scale: 1.0,
            },
            policy: Policy::default(),
        },
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut rendered = false;
    let mut found = false;
    let mut ready = false;
    let mut find_sent = false;
    let mut asset_resolved = false;
    let mut zoom_sent = false;
    let mut zoomed = false;
    let mut resized = false;
    let mut popup_stage = 0;
    let mut popup_visible = false;
    let mut popup_hidden = false;
    let mut cycle_shortcut = false;
    let mut physical_shortcuts = Vec::new();
    let mut edit = Edit::Idle;
    let mut copy_pending = false;
    let mut editable_space = false;
    let mut repeat_find_sequence = 0;
    let mut repeated_find = false;
    while Instant::now() < deadline
        && !(rendered
            && found
            && ready
            && asset_resolved
            && zoomed
            && resized
            && popup_visible
            && popup_hidden
            && cycle_shortcut)
    {
        let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(100)) else {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("Chromium terminated before rendering: {status}");
            }
            // Selection notifications are asynchronous; probe again until they arrive.
            if edit != Edit::Idle && !editable_space && !copy_pending {
                select_all_and_copy(&mut stdin, &mut sequence);
                copy_pending = true;
            }
            continue;
        };
        if matches!(event.kind, EventKind::Frame(_))
            && edit != Edit::Idle
            && !editable_space
            && !copy_pending
        {
            select_all_and_copy(&mut stdin, &mut sequence);
            copy_pending = true;
        }
        match event.kind {
            EventKind::Ready => ready = true,
            EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => {
                let data: &[u8] = match resource {
                    1 => &html,
                    2 => css,
                    _ => panic!("unexpected resource"),
                };
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
            EventKind::ResourceResolve { request, base, url } => {
                assert_eq!(base, 1);
                let reply = if url == "assets/style.css" {
                    asset_resolved = true;
                    ResourceReply::Resolved {
                        request,
                        resource: Resource {
                            id: 2,
                            length: css.len() as u64,
                            mime: "text/css".into(),
                        },
                    }
                } else {
                    ResourceReply::Denied {
                        request,
                        reason: "fixture has no other sibling capabilities".into(),
                    }
                };
                send(&mut stdin, &mut sequence, CommandKind::ResourceReply(reply));
            }
            EventKind::Frame(frame) => {
                if popup_stage == 1 {
                    let pixel = (140 * frame.width + 200) as usize * 4;
                    if frame.rgba[pixel..pixel + 4] != [0x12, 0x34, 0x56, 255] {
                        popup_visible = true;
                        click(&mut stdin, &mut sequence, 310.0, 190.0);
                        popup_stage = 2;
                    }
                } else if popup_stage == 2 {
                    let pixel = (140 * frame.width + 200) as usize * 4;
                    if frame.rgba[pixel..pixel + 4] == [0x12, 0x34, 0x56, 255] {
                        popup_hidden = true;
                        popup_stage = 3;
                        send_key(
                            &mut stdin,
                            &mut sequence,
                            "Backspace",
                            "Backspace",
                            Modifiers::default(),
                        );
                        for key in ["ArrowLeft", "ArrowRight"] {
                            send_key(
                                &mut stdin,
                                &mut sequence,
                                key,
                                key,
                                Modifiers {
                                    alt: true,
                                    ..Default::default()
                                },
                            );
                        }
                        send(
                            &mut stdin,
                            &mut sequence,
                            CommandKind::Key(Key {
                                key: " ".into(),
                                code: "Space".into(),
                                pressed: true,
                                repeat: false,
                                modifiers: Modifiers::default(),
                            }),
                        );
                    }
                }
                let pixel = ((frame.height - 5) * frame.width + frame.width - 5) as usize * 4;
                if frame.rgba[pixel..pixel + 4] == [0x12, 0x34, 0x56, 255] {
                    rendered = true;
                    if !find_sent {
                        send(
                            &mut stdin,
                            &mut sequence,
                            CommandKind::Find(Find {
                                query: "searchable".into(),
                                forward: true,
                                match_case: false,
                                find_next: false,
                            }),
                        );
                        find_sent = true;
                    }
                }
                if zoom_sent && frame.viewport_revision == 1 {
                    let pixel = (160 * frame.width + 125) as usize * 4;
                    if frame.rgba[pixel..pixel + 4] == [255, 0, 0, 255] {
                        zoomed = true;
                        send(
                            &mut stdin,
                            &mut sequence,
                            CommandKind::Resize(Viewport {
                                revision: 2,
                                width: 801,
                                height: 301,
                                scale: 1.25,
                            }),
                        );
                    }
                }
                if frame.viewport_revision == 2 && frame.width == 801 && frame.height == 301 {
                    resized = true;
                }
            }
            EventKind::FindResult(result) => {
                if result.complete && result.matches == 1 && repeat_find_sequence == 0 {
                    send(
                        &mut stdin,
                        &mut sequence,
                        CommandKind::Find(Find {
                            query: "searchable".into(),
                            forward: true,
                            match_case: false,
                            find_next: true,
                        }),
                    );
                    repeat_find_sequence = sequence;
                } else if result.complete && result.request_sequence == repeat_find_sequence {
                    repeated_find = result.matches == 1;
                }
                found |= result.complete && result.matches == 1;
            }
            EventKind::State(state) => assert_ne!(state.title, "SCRIPT EXECUTED"),
            EventKind::HostShortcut(
                value @ (HostShortcut::Backspace | HostShortcut::AltLeft | HostShortcut::AltRight),
            ) => {
                assert!(
                    edit == Edit::Idle,
                    "editable focus escaped to physical host navigation"
                );
                physical_shortcuts.push(value);
            }
            EventKind::HostShortcut(HostShortcut::CycleNext) => {
                assert!(
                    edit == Edit::Idle,
                    "Space in an editable field escaped to file cycling"
                );
                cycle_shortcut = true;
            }
            EventKind::Selection(text) if edit != Edit::Idle => {
                copy_pending = false;
                if edit == Edit::Focusing && text == "abc" {
                    // Select-all reached the input, so Chromium has focused it: Space
                    // must now replace the selected value instead of cycling files.
                    send_key(
                        &mut stdin,
                        &mut sequence,
                        " ",
                        "Space",
                        Modifiers::default(),
                    );
                    send(&mut stdin, &mut sequence, CommandKind::Text(" ".into()));
                    edit = Edit::Typed;
                    select_all_and_copy(&mut stdin, &mut sequence);
                    copy_pending = true;
                } else if edit == Edit::Typed {
                    editable_space = text == " ";
                }
            }
            EventKind::Error { message, .. } => panic!("provider error: {message}"),
            _ => {}
        }
        if rendered && found && repeated_find && popup_stage == 0 {
            click(&mut stdin, &mut sequence, 200.0, 110.0);
            popup_stage = 1;
        }
        if cycle_shortcut && physical_shortcuts.len() == 3 && edit == Edit::Idle {
            // No fixed delay: probe focus by selecting all until the input's value is
            // the selection (Space is sent only then).
            edit = Edit::Focusing;
            click(&mut stdin, &mut sequence, 20.0, 75.0);
            select_all_and_copy(&mut stdin, &mut sequence);
            copy_pending = true;
        }
        if rendered && found && popup_hidden && cycle_shortcut && editable_space && !zoom_sent {
            send(&mut stdin, &mut sequence, CommandKind::Zoom(1.5));
            zoom_sent = true;
        }
    }
    #[cfg(target_os = "linux")]
    if rendered {
        runtime_linux::verify_renderer_sandbox(child.id());
    }
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let status = wait_for_exit(child);
    assert!(
        ready
            && rendered
            && found
            && asset_resolved
            && zoomed
            && resized
            && popup_visible
            && popup_hidden
            && cycle_shortcut,
        "browser ready={ready}, rendered={rendered}, asset={asset_resolved}, find={found}, zoom={zoomed}, fractionalDPI={resized}, popup={popup_visible}/{popup_hidden}, space={cycle_shortcut}"
    );
    assert_eq!(
        physical_shortcuts,
        vec![
            HostShortcut::Backspace,
            HostShortcut::AltLeft,
            HostShortcut::AltRight
        ]
    );
    assert!(
        status.is_some_and(|s| s.success()),
        "CEF did not close cleanly: {status:?}"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "offline Chromium contacted the denied HTTP image endpoint"
    );
}

/// Editable-field steps. Chromium does not acknowledge processed input in frames, so
/// focus and text are observed through the copied selection instead of fixed delays.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Idle,
    Focusing,
    Typed,
}

fn select_all_and_copy(stdin: &mut impl Write, sequence: &mut u64) {
    send_key(
        stdin,
        sequence,
        "a",
        "KeyA",
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    send(stdin, sequence, CommandKind::Copy);
}

/// Waits for the provider to exit after Close; the deadline only elapses on failure.
/// CefShutdown alone took 10-12 s on a loaded 20-thread development machine.
fn wait_for_exit(mut child: std::process::Child) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if status.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    status
}

/// A printable key as the host sends it: key down, its text, key up.
fn typed_key(
    stdin: &mut impl Write,
    sequence: &mut u64,
    key: &str,
    code: &str,
    modifiers: Modifiers,
) {
    let event = |pressed| {
        CommandKind::Key(Key {
            key: key.into(),
            code: code.into(),
            pressed,
            repeat: false,
            modifiers,
        })
    };
    send(stdin, sequence, event(true));
    send(stdin, sequence, CommandKind::Text(key.into()));
    send(stdin, sequence, event(false));
}

fn send_key(
    stdin: &mut impl Write,
    sequence: &mut u64,
    key: &str,
    code: &str,
    modifiers: Modifiers,
) {
    for pressed in [true, false] {
        send(
            stdin,
            sequence,
            CommandKind::Key(Key {
                key: key.into(),
                code: code.into(),
                pressed,
                repeat: false,
                modifiers,
            }),
        );
    }
}

/// Final textarea value: Enter and Shift+Enter each insert one newline; "." and the
/// shifted quote/bracket are inserted text, never Delete/PageDown/ArrowDown commands
/// that would remove or select the "y" after the caret; the IME commit is inserted.
const TEXTAREA_EXPECTED: &str = "first\nsecond\nthirdx.\"(日本y";

#[test]
#[ignore = "requires matching packaged CEF runtime and interactive Windows or Linux desktop"]
fn textarea_enter_and_shift_enter_preserve_newlines_through_key_text_and_copy() {
    let executable =
        std::env::var_os("MIDNIGHT_HTML_TEST_EXECUTABLE").expect("set packaged bootstrap path");
    let profile = std::path::PathBuf::from(
        std::env::var_os("MIDNIGHT_WEB_VIEWER_PROFILE").expect("set owned private profile"),
    )
    .join("textarea-newlines");
    let mut child = Process::new(executable)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", profile)
        .env(HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut stdout) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let html = b"<!doctype html><title>Textarea newline fixture</title><style>html,body{margin:0}textarea{position:absolute;left:0;top:0;width:300px;height:160px}</style><textarea aria-label='Newline fixture'>seed</textarea>";
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
                width: 320,
                height: 200,
                scale: 1.0,
            },
            policy: Policy::default(),
        },
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut loaded = false;
    let mut titled = false;
    let mut painted = false;
    let mut edit = Edit::Idle;
    let mut copy_pending = false;
    let mut selection = None;
    while Instant::now() < deadline && selection.as_deref() != Some(TEXTAREA_EXPECTED) {
        let mut probe = false;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Message::Event(event)) => match event.kind {
                EventKind::ResourceRead {
                    request,
                    resource,
                    offset,
                    length,
                } => {
                    assert_eq!(resource, 1);
                    let end = (offset as usize + length as usize).min(html.len());
                    send(
                        &mut stdin,
                        &mut sequence,
                        CommandKind::ResourceReply(ResourceReply::Data {
                            request,
                            resource,
                            offset,
                            bytes: html[offset as usize..end].to_vec(),
                        }),
                    );
                }
                EventKind::ResourceResolve { request, .. } => send(
                    &mut stdin,
                    &mut sequence,
                    CommandKind::ResourceReply(ResourceReply::Denied {
                        request,
                        reason: "fixture has no siblings".into(),
                    }),
                ),
                EventKind::State(state) => {
                    loaded |= !state.loading;
                    titled |= state.title == "Textarea newline fixture";
                }
                EventKind::Frame(_) => {
                    painted |= titled;
                    probe = true;
                }
                EventKind::Selection(text) => {
                    copy_pending = false;
                    if edit == Edit::Focusing && text == "seed" {
                        // Select-all reached the textarea, so it has focus; the first
                        // text replaces the selected seed.
                        type_textarea(&mut stdin, &mut sequence);
                        edit = Edit::Typed;
                        select_all_and_copy(&mut stdin, &mut sequence);
                        copy_pending = true;
                    } else if edit == Edit::Typed {
                        selection = Some(text);
                    }
                }
                EventKind::HostShortcut(shortcut) => {
                    panic!("textarea editing escaped to host: {shortcut:?}")
                }
                EventKind::Error { message, .. } => panic!("textarea provider error: {message}"),
                _ => {}
            },
            _ => {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("Chromium terminated before textarea qualification: {status}");
                }
                probe = true;
            }
        }
        if loaded && titled && painted && edit == Edit::Idle {
            edit = Edit::Focusing;
            click(&mut stdin, &mut sequence, 40.0, 40.0);
            select_all_and_copy(&mut stdin, &mut sequence);
            copy_pending = true;
        } else if probe && edit != Edit::Idle && !copy_pending {
            // Selection notifications are asynchronous; probe again until they arrive.
            select_all_and_copy(&mut stdin, &mut sequence);
            copy_pending = true;
        }
    }
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let status = wait_for_exit(child);
    assert_eq!(
        selection.as_deref(),
        Some(TEXTAREA_EXPECTED),
        "Enter/Shift+Enter, punctuation and IME commit must each insert exactly their text"
    );
    assert!(
        status.is_some_and(|status| status.success()),
        "CEF did not close cleanly"
    );
}

fn type_textarea(stdin: &mut impl Write, sequence: &mut u64) {
    send(stdin, sequence, CommandKind::Text("first".into()));
    enter(stdin, sequence, false);
    send(stdin, sequence, CommandKind::Text("second".into()));
    enter(stdin, sequence, true);
    send(stdin, sequence, CommandKind::Text("third".into()));
    send_key(
        stdin,
        sequence,
        "Backspace",
        "Backspace",
        Modifiers::default(),
    );
    send(stdin, sequence, CommandKind::Text("dxy".into()));
    send_key(
        stdin,
        sequence,
        "ArrowLeft",
        "ArrowLeft",
        Modifiers::default(),
    );
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    // US "." and "\"", Czech QWERTZ "(" (Shift+BracketRight), typed before the "y".
    typed_key(stdin, sequence, ".", "Period", Modifiers::default());
    typed_key(stdin, sequence, "\"", "Quote", shift);
    typed_key(stdin, sequence, "(", "BracketRight", shift);
    for ime in [
        Ime::Start,
        Ime::Update {
            text: "日本".into(),
            selection_start: 6,
            selection_end: 6,
        },
        Ime::Commit("日本".into()),
        Ime::End,
    ] {
        send(stdin, sequence, CommandKind::Ime(ime));
    }
    for key in ["ArrowLeft", "ArrowRight"] {
        send_key(
            stdin,
            sequence,
            key,
            key,
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
    }
}

fn enter(stdin: &mut impl Write, sequence: &mut u64, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    send(
        stdin,
        sequence,
        CommandKind::Key(Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed: true,
            repeat: false,
            modifiers,
        }),
    );
    send(stdin, sequence, CommandKind::Text("\r".into()));
    send(
        stdin,
        sequence,
        CommandKind::Key(Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed: false,
            repeat: false,
            modifiers,
        }),
    );
}

/// A tall page of 10 px blocks whose colors number them, so a frame shows the page's
/// scroll offset.
fn ruler_page(blocks: u32) -> Vec<u8> {
    let mut html = String::from(
        "<!doctype html><title>Wheel fixture</title><style>html,body{margin:0}div{height:10px}</style>",
    );
    for index in 0..blocks {
        let (red, green) = (index % 256, index / 256 * 16 + 8);
        html.push_str(&format!(
            "<div style=background:rgb({red},{green},100)></div>"
        ));
    }
    html.into_bytes()
}

/// Scroll offset (CSS px at scale 1) of the frame's top edge: where the second block
/// in its middle column starts on the page, less the row it starts at in the frame.
fn ruler_offset(frame: &Frame) -> Option<u32> {
    let x = frame.width / 2;
    let block = |y: u32| {
        let pixel = ((y * frame.width + x) * 4) as usize;
        let [red, green, blue] = [
            frame.rgba[pixel],
            frame.rgba[pixel + 1],
            frame.rgba[pixel + 2],
        ];
        let high = green.checked_sub(8).filter(|value| value % 16 == 0)?;
        (blue == 100).then(|| u32::from(red) + u32::from(high) / 16 * 256)
    };
    let first = block(0)?;
    (1..frame.height).find_map(|y| {
        block(y)
            .filter(|index| *index != first)
            .and_then(|index| (index * 10).checked_sub(y))
    })
}

/// The lines a wheel notch scrolls on this machine (`SPI_GETWHEELSCROLLLINES`), which CEF
/// reads for every event; `None` when a notch scrolls a page or nothing.
#[cfg(windows)]
fn wheel_lines() -> Option<u32> {
    const SPI_GETWHEELSCROLLLINES: u32 = 0x0068;
    const WHEEL_PAGESCROLL: u32 = u32::MAX;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            parameter: u32,
            value: *mut std::ffi::c_void,
            ini: u32,
        ) -> i32;
    }
    let mut lines = 0_u32;
    // SAFETY: the action writes one u32 through the pointer, which outlives the call.
    let read =
        unsafe { SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, (&raw mut lines).cast(), 0) };
    (read != 0 && lines != 0 && lines != WHEEL_PAGESCROLL).then_some(lines)
}

/// The whole DIPs Chromium scrolls for one wheel event of `units` on this machine: CEF
/// on Windows reads `WHEEL_DELTA` units (120 per notch) of the current wheel lines,
/// 100/3 DIPs each, in `f32`, and Chromium drops each event's fraction; on Linux a unit
/// is a DIP. `None` when no distance can be expected (a notch scrolls a page).
fn chromium_dips() -> Option<impl Fn(u32) -> u32> {
    #[cfg(windows)]
    {
        let lines = wheel_lines()?;
        Some(move |units: u32| (units as f32 / 120.0 * (lines as f32 * (100.0_f32 / 3.0))) as u32)
    }
    #[cfg(not(windows))]
    {
        Some(|units: u32| units)
    }
}

/// The host glides wheel notches and coasts touchpad flicks itself, one delta per frame,
/// so Chromium must apply each wheel event at once instead of animating it again. The
/// distances follow this machine's wheel setting: at Windows' default of three lines,
/// and on Linux, every whole DIP is reachable and they are exact.
#[test]
#[ignore = "requires matching packaged CEF runtime and interactive Windows or Linux desktop"]
fn chromium_applies_each_wheel_event_at_once() {
    let Some(dips) = chromium_dips() else {
        eprintln!("not run: a wheel notch scrolls a page or nothing on this machine");
        return;
    };
    // The closest distance one event can scroll without passing `wanted`, which the
    // provider sends; and the most one wheel unit more scrolls up to `wanted`, so that
    // no gesture can come closer.
    let closest = |wanted: u32| {
        (0..)
            .map(&dips)
            .take_while(|&scrolled| scrolled <= wanted)
            .last()
            .unwrap_or(0)
    };
    let gap = |wanted: u32| {
        (0..)
            .take_while(|&units| dips(units) <= wanted)
            .map(|units| dips(units + 1) - dips(units))
            .fold(1, u32::max)
    };
    let executable =
        std::env::var_os("MIDNIGHT_HTML_TEST_EXECUTABLE").expect("set packaged bootstrap path");
    let profile = std::path::PathBuf::from(
        std::env::var_os("MIDNIGHT_WEB_VIEWER_PROFILE").expect("set owned private profile"),
    )
    .join("wheel-at-once");
    let mut child = Process::new(executable)
        .env("MIDNIGHT_WEB_VIEWER_PROFILE", profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut stdout) {
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let html = ruler_page(1000);
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
                width: 320,
                height: 200,
                scale: 1.0,
            },
            policy: Policy::default(),
        },
    );
    // Offsets of the frames painted until the page is still for 300 ms.
    let mut settle = |stdin: &mut std::process::ChildStdin, sequence: &mut u64| {
        let mut offsets = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut quiet = Instant::now();
        while Instant::now() < deadline
            && (offsets.is_empty() || quiet.elapsed() < Duration::from_millis(300))
        {
            let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(20)) else {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("Chromium terminated during the wheel fixture: {status}");
                }
                continue;
            };
            match event.kind {
                EventKind::ResourceRead {
                    request,
                    resource,
                    offset,
                    length,
                } => {
                    let end = (offset as usize + length as usize).min(html.len());
                    send(
                        stdin,
                        sequence,
                        CommandKind::ResourceReply(ResourceReply::Data {
                            request,
                            resource,
                            offset,
                            bytes: html[offset as usize..end].to_vec(),
                        }),
                    );
                }
                EventKind::ResourceResolve { request, .. } => send(
                    stdin,
                    sequence,
                    CommandKind::ResourceReply(ResourceReply::Denied {
                        request,
                        reason: "fixture has no siblings".into(),
                    }),
                ),
                EventKind::Frame(frame) => {
                    if let Some(offset) = ruler_offset(&frame) {
                        offsets.push(offset);
                        quiet = Instant::now();
                    }
                }
                EventKind::Error { message, .. } => panic!("wheel provider error: {message}"),
                _ => {}
            }
        }
        offsets
    };
    assert_eq!(settle(&mut stdin, &mut sequence).last(), Some(&0));
    let wheel = |stdin: &mut std::process::ChildStdin, sequence: &mut u64, delta_y: f64| {
        send(
            stdin,
            sequence,
            CommandKind::Wheel(Wheel {
                x: 160.0,
                y: 100.0,
                delta_x: 0.0,
                delta_y,
                modifiers: Modifiers::default(),
            }),
        );
    };
    // One event scrolls its whole distance in the next frame (as closely as this
    // machine's wheel setting lets it). Chromium's own wheel animation would show it in
    // about ten frames over 170 ms instead.
    wheel(&mut stdin, &mut sequence, -120.0);
    let down = settle(&mut stdin, &mut sequence);
    let expected = closest(120);
    assert!(
        !down.is_empty() && down.iter().all(|offset| *offset == expected),
        "frames after one wheel event: {down:?}, expected {expected} px"
    );
    // A wheel past the top stops at the edge.
    wheel(&mut stdin, &mut sequence, 240.0);
    let up = settle(&mut stdin, &mut sequence);
    assert!(
        !up.is_empty() && up.iter().all(|offset| *offset == 0),
        "frames after a wheel past the top: {up:?}"
    );
    // A notch arrives as the host glides it, one small delta per frame. Together they
    // scroll the notch's whole 60 px, as in the text viewer (on Windows, CEF scrolls a
    // delta by the system's wheel lines; the provider converts its units), or less by
    // less than one unit's scroll, which the gesture's end drops.
    for _ in 0..25 {
        wheel(&mut stdin, &mut sequence, -2.4);
    }
    let glide = settle(&mut stdin, &mut sequence);
    let unit = gap(60);
    assert!(
        glide
            .last()
            .is_some_and(|offset| *offset <= 60 && *offset > 60 - unit),
        "frames after a notch glide: {glide:?}, expected 60 px or less by under {unit} px"
    );
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let status = wait_for_exit(child);
    assert!(
        status.is_some_and(|status| status.success()),
        "CEF did not close cleanly"
    );
}

fn click(stdin: &mut impl Write, sequence: &mut u64, x: f64, y: f64) {
    for kind in [PointerKind::Down, PointerKind::Up] {
        send(
            stdin,
            sequence,
            CommandKind::Pointer(Pointer {
                kind,
                button: PointerButton::Left,
                x,
                y,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
        );
    }
}
