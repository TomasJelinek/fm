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
    let mut edit_started = false;
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
            continue;
        };
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
            EventKind::HostShortcut(HostShortcut::CycleNext) => {
                assert!(
                    !edit_started,
                    "Space in an editable field escaped to file cycling"
                );
                cycle_shortcut = true;
            }
            EventKind::Selection(text) if edit_started => {
                editable_space = text.contains(' ') && text.replace(' ', "") == "abc";
            }
            EventKind::Error { message, .. } => panic!("provider error: {message}"),
            _ => {}
        }
        if rendered && found && repeated_find && popup_stage == 0 {
            click(&mut stdin, &mut sequence, 200.0, 110.0);
            popup_stage = 1;
        }
        if cycle_shortcut && !edit_started {
            edit_started = true;
            click(&mut stdin, &mut sequence, 20.0, 75.0);
            std::thread::sleep(Duration::from_millis(100));
            send_key(
                &mut stdin,
                &mut sequence,
                " ",
                "Space",
                Modifiers::default(),
            );
            send(&mut stdin, &mut sequence, CommandKind::Text(" ".into()));
            std::thread::sleep(Duration::from_millis(100));
            send_key(
                &mut stdin,
                &mut sequence,
                "a",
                "KeyA",
                Modifiers {
                    control: true,
                    ..Default::default()
                },
            );
            std::thread::sleep(Duration::from_millis(100));
            send(&mut stdin, &mut sequence, CommandKind::Copy);
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
    let close_deadline = Instant::now() + Duration::from_secs(10);
    let mut status = None;
    while Instant::now() < close_deadline {
        if let Some(done) = child.try_wait().unwrap() {
            status = Some(done);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if status.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
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
    let html = b"<!doctype html><title>Textarea newline fixture</title><style>html,body{margin:0}textarea{position:absolute;left:0;top:0;width:300px;height:160px}</style><textarea aria-label='Newline fixture'></textarea>";
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
    let mut entered = false;
    let mut selection = None;
    while Instant::now() < deadline && selection.is_none() {
        let Ok(Message::Event(event)) = rx.recv_timeout(Duration::from_millis(100)) else {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("Chromium terminated before textarea qualification: {status}");
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
            EventKind::Frame(_) if titled => painted = true,
            EventKind::Selection(text) => selection = Some(text),
            EventKind::HostShortcut(shortcut) => {
                panic!("textarea editing escaped to host: {shortcut:?}")
            }
            EventKind::Error { message, .. } => panic!("textarea provider error: {message}"),
            _ => {}
        }
        if loaded && titled && painted && !entered {
            entered = true;
            click(&mut stdin, &mut sequence, 40.0, 40.0);
            std::thread::sleep(Duration::from_millis(100));
            send(&mut stdin, &mut sequence, CommandKind::Text("first".into()));
            enter(&mut stdin, &mut sequence, false);
            send(
                &mut stdin,
                &mut sequence,
                CommandKind::Text("second".into()),
            );
            enter(&mut stdin, &mut sequence, true);
            send(&mut stdin, &mut sequence, CommandKind::Text("third".into()));
            std::thread::sleep(Duration::from_millis(100));
            send_key(
                &mut stdin,
                &mut sequence,
                "a",
                "KeyA",
                Modifiers {
                    control: true,
                    ..Default::default()
                },
            );
            std::thread::sleep(Duration::from_millis(100));
            send(&mut stdin, &mut sequence, CommandKind::Copy);
        }
    }
    send(&mut stdin, &mut sequence, CommandKind::Close);
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(done) = child.try_wait().unwrap() {
            status = Some(done);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if status.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    assert_eq!(
        selection.as_deref(),
        Some("first\nsecond\nthird"),
        "Enter and Shift+Enter must each insert exactly one textarea newline"
    );
    assert!(
        status.is_some_and(|status| status.success()),
        "CEF did not close cleanly"
    );
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
