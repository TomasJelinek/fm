//! Start a viewer only after its owned parent has a native window and real DPI.
use midnight_ui_input::WindowPlacement;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    io::Write,
    rc::Rc,
    time::{Duration, Instant},
};

slint::slint! {
    export component ProbeParent inherits Window {
        title: "Viewer diagnostic parent";
        preferred-width: 1000px;
        preferred-height: 700px;
        background: #191d25;
    }
}

pub(super) fn run<T: 'static>(
    native_parent: bool,
    setup: impl FnOnce(WindowPlacement) -> Result<T, Box<dyn std::error::Error>> + 'static,
) -> Result<T, Box<dyn std::error::Error>> {
    let terminal_logging = std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--inactive-native-source");
    let size = std::env::var("MIDNIGHT_VIEWER_PROBE_SIZE")
        .ok()
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some(slint::LogicalSize::new(
                width.parse().ok()?,
                height.parse().ok()?,
            ))
        })
        .filter(|size| {
            (320.0..=4096.0).contains(&size.width) && (240.0..=4096.0).contains(&size.height)
        })
        .unwrap_or(slint::LogicalSize::new(1000.0, 700.0));
    if !native_parent {
        let running = setup(size.into())?;
        slint::run_event_loop()?;
        return Ok(running);
    }
    let parent = ProbeParent::new()?;
    parent.window().set_size(size);
    parent.window().on_close_requested(|| {
        eprintln!("probe_exit native_parent_close_requested=true");
        slint::CloseRequestResponse::HideWindow
    });
    parent.show()?;
    let waiting = parent.as_weak();
    let result = Rc::new(RefCell::new(None));
    let delivered = result.clone();
    let mut setup = Some(setup);
    let started = Instant::now();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(4), move || {
        if setup.is_none() {
            return;
        }
        let Some(parent) = waiting.upgrade() else {
            if terminal_logging {
                let _ = writeln!(
                    std::io::stderr(),
                    "probe_terminal schema=1 context=native_parent_readiness error=\"native probe parent disappeared\""
                );
            }
            *delivered.borrow_mut() = Some(Err("native probe parent disappeared".into()));
            let _ = slint::quit_event_loop();
            return;
        };
        let native = parent.window().with_winit_window(|native| {
            (native.inner_size(), native.scale_factor())
        });
        let Some((size, scale)) = native.filter(|(size, scale)| size.width > 0 && size.height > 0 && *scale > 0.0) else {
            if started.elapsed() >= Duration::from_secs(10) {
                if terminal_logging {
                    let _ = writeln!(
                        std::io::stderr(),
                        "probe_terminal schema=1 context=native_parent_readiness error=\"native probe parent readiness timeout\""
                    );
                }
                *delivered.borrow_mut() = Some(Err("native probe parent readiness timeout".into()));
                let _ = slint::quit_event_loop();
            }
            return;
        };
        eprintln!("probe_parent native_window=true physical={size:?} native_scale={scale} slint_scale={}", parent.window().scale_factor());
        let placement = WindowPlacement::capture(parent.window());
        let running = setup.take().expect("parent setup runs once")(placement);
        let failed = running.is_err();
        if terminal_logging && let Err(error) = &running {
            let _ = writeln!(
                std::io::stderr(),
                "probe_terminal schema=1 context=native_parent_setup error={error:?}"
            );
        }
        *delivered.borrow_mut() = Some(running);
        // Keep the owned parent behind its child, matching the application.
        // Hiding the final native window here could exit before child creation.
        if failed {
            let _ = slint::quit_event_loop();
        }
    });
    let loop_result = slint::run_event_loop();
    if terminal_logging {
        let _ = match result.borrow().as_ref() {
            Some(Ok(_)) => writeln!(
                std::io::stderr(),
                "probe_terminal schema=1 context=event_loop_return loop_result={loop_result:?} setup_status=ok"
            ),
            Some(Err(error)) => writeln!(
                std::io::stderr(),
                "probe_terminal schema=1 context=event_loop_return loop_result={loop_result:?} setup_status=error error={error:?}"
            ),
            None => writeln!(
                std::io::stderr(),
                "probe_terminal schema=1 context=event_loop_return loop_result={loop_result:?} setup_status=missing error=\"native parent never started the viewer\""
            ),
        };
    }
    loop_result?;
    timer.stop();
    let _ = parent.hide();
    result
        .borrow_mut()
        .take()
        .unwrap_or_else(|| Err("native parent never started the viewer".into()))
}
