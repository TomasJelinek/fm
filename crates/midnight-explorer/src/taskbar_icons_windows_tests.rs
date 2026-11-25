//! Query the actual HWND from another process while its UI thread pumps messages.
use crate::native_events::{MainWindowEvents, Observer};
use midnight_core::AppIcon;
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::WindowEvent,
            platform::windows::EventLoopBuilderExtWindows,
            raw_window_handle::{HasWindowHandle, RawWindowHandle},
        },
    },
};
use std::{path::PathBuf, process::Stdio, rc::Rc, sync::mpsc, time::Duration};

// The explorer forbids unsafe Rust. Windows PowerShell can read both borrowed HICONs
// through user32 and sample their artwork without changing either window property.
const QUERY_ICONS: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class IconProbe {
    [DllImport("user32.dll", ExactSpelling=true)]
    public static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr kind,
        IntPtr unused, uint flags, uint timeout, out IntPtr result);
}
'@
$window = [IntPtr]::new([long]$env:MIDNIGHT_ICON_TEST_HWND)
foreach ($kind in 0, 1) {
    $handle = [IntPtr]::Zero
    if ([IconProbe]::SendMessageTimeoutW($window, 0x7f, [IntPtr]::new($kind),
            [IntPtr]::Zero, 2, 3000, [ref]$handle) -eq [IntPtr]::Zero -or
        $handle -eq [IntPtr]::Zero) { throw "WM_GETICON type $kind yielded no icon" }
    $icon = [Drawing.Icon]::FromHandle($handle)
    $bitmap = $icon.ToBitmap()
    $pixel = $bitmap.GetPixel([int]($bitmap.Width / 2), [int]($bitmap.Height / 2))
    $pixel.ToArgb().ToString('X8')
    $bitmap.Dispose()
    $icon.Dispose()
}
"#;

fn icon_colors(hwnd: isize) -> Result<Vec<[u8; 3]>, String> {
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").ok_or("Missing SystemRoot")?)
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = std::process::Command::new(powershell);
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", QUERY_ICONS])
        .env("MIDNIGHT_ICON_TEST_HWND", hwnd.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = midnight_worker_platform::spawn_command(&mut command)
        .and_then(|child| child.wait_with_output())
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            u32::from_str_radix(line.trim(), 16)
                .map(|argb| {
                    let [_, r, g, b] = argb.to_be_bytes();
                    [r, g, b]
                })
                .map_err(|error| format!("WM_GETICON pixel {line:?}: {error}"))
        })
        .collect()
}

fn selected_color(main: &MainWindow) -> [u8; 3] {
    let image = main.get_current_app_icon().to_rgba8().unwrap();
    let pixel =
        image.as_slice()[((image.height() / 2) * image.width() + image.width() / 2) as usize];
    [pixel.r, pixel.g, pixel.b]
}

#[test]
#[ignore = "requires an interactive Windows desktop; run by itself with --test-threads=1"]
fn first_show_and_live_change_set_both_native_icons() {
    let mut builder: slint::winit_030::EventLoopBuilder =
        slint::winit_030::winit::event_loop::EventLoop::with_user_event();
    builder.with_any_thread(true);
    let events = Rc::new(MainWindowEvents::default());
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_event_loop_builder(builder)
        .with_winit_custom_application_handler(Observer::new(None, events.clone()))
        .select()
        .unwrap();
    let main = MainWindow::new().unwrap();
    events.bind(&main);
    main.set_app_icon_index(AppIcon::OrigamiTrail.index());
    let initial_color = selected_color(&main);
    let (drawn, first_draw) = mpsc::channel();
    let mut drawn = Some(drawn);
    main.window().on_winit_window_event(move |window, event| {
        if matches!(event, WindowEvent::RedrawRequested)
            && let Some(signal) = drawn.take()
        {
            window.with_winit_window(|native| {
                if let RawWindowHandle::Win32(handle) = native.window_handle().unwrap().as_raw() {
                    let _ = signal.send(handle.hwnd.get());
                }
            });
        }
        EventResult::Propagate
    });
    // Showing before the event loop realizes the HWND exercises Slint's cached-attributes path.
    assert!(!main.window().has_winit_window());
    main.show().unwrap();
    let weak = main.as_weak();
    let worker = std::thread::spawn(move || {
        let result = (|| -> Result<_, String> {
            let wait = Duration::from_secs(20);
            let hwnd = first_draw.recv_timeout(wait).map_err(|e| e.to_string())?;
            let opening = icon_colors(hwnd)?;
            let (applied, changed) = mpsc::channel();
            weak.upgrade_in_event_loop(move |main| {
                main.set_app_icon_index(AppIcon::Interleave.index());
                let expected = selected_color(&main);
                slint::Timer::single_shot(Duration::ZERO, move || {
                    let _ = applied.send(expected);
                });
            })
            .map_err(|e| e.to_string())?;
            let new_color = changed.recv_timeout(wait).map_err(|e| e.to_string())?;
            Ok((opening, icon_colors(hwnd)?, new_color))
        })();
        let _ = slint::quit_event_loop();
        result
    });
    slint::run_event_loop().unwrap();
    main.hide().unwrap();
    let (opening, changed, new_color) = worker.join().unwrap().unwrap();
    assert_ne!(initial_color, new_color);
    // Icon conversion and desktop scaling can round these nearly opaque centre pixels.
    for (observed, expected) in [(opening, initial_color), (changed, new_color)] {
        assert_eq!(observed.len(), 2, "expected both ICON_SMALL and ICON_BIG");
        for pixel in observed {
            assert!(
                pixel
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 12),
                "native icon centre {pixel:?}, expected {expected:?}"
            );
        }
    }
}
