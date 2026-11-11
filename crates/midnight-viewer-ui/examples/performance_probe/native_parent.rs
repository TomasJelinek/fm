//! Start a viewer only after its owned parent has a native window and real DPI.
use midnight_ui_input::WindowPlacement;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

slint::slint! {
    export component ProbeParent inherits Window {
        title: "Viewer diagnostic parent";
        width: 1000px;
        height: 700px;
        background: #191d25;
    }
}

pub(super) fn run<T: 'static>(
    native_parent: bool,
    setup: impl FnOnce(WindowPlacement) -> Result<T, Box<dyn std::error::Error>> + 'static,
) -> Result<T, Box<dyn std::error::Error>> {
    if !native_parent {
        let running = setup(slint::LogicalSize::new(1000.0, 700.0).into())?;
        slint::run_event_loop()?;
        return Ok(running);
    }
    let parent = ProbeParent::new()?;
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
            *delivered.borrow_mut() = Some(Err("native probe parent disappeared".into()));
            let _ = slint::quit_event_loop();
            return;
        };
        let native = parent.window().with_winit_window(|native| {
            (native.inner_size(), native.scale_factor())
        });
        let Some((size, scale)) = native.filter(|(size, scale)| size.width > 0 && size.height > 0 && *scale > 0.0) else {
            if started.elapsed() >= Duration::from_secs(10) {
                *delivered.borrow_mut() = Some(Err("native probe parent readiness timeout".into()));
                let _ = slint::quit_event_loop();
            }
            return;
        };
        eprintln!("probe_parent native_window=true physical={size:?} native_scale={scale} slint_scale={}", parent.window().scale_factor());
        let placement = WindowPlacement::capture(parent.window());
        let running = setup.take().expect("parent setup runs once")(placement);
        let failed = running.is_err();
        *delivered.borrow_mut() = Some(running);
        // Keep the owned parent behind its child, matching the application.
        // Hiding the final native window here could exit before child creation.
        if failed {
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop()?;
    timer.stop();
    let _ = parent.hide();
    result
        .borrow_mut()
        .take()
        .unwrap_or_else(|| Err("native parent never started the viewer".into()))
}
