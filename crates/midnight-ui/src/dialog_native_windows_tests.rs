//! Opt-in desktop qualification: actual HWND ownership/styles, movement, visibility and cleanup.
use super::*;
use crate::dialog_host::{DialogHost, DialogModality, DialogOwner, DialogSessions};
use slint::winit_030::winit::platform::windows::EventLoopBuilderExtWindows;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[link(name = "user32")]
unsafe extern "system" {
    fn GetWindow(window: isize, command: u32) -> isize;
    fn GetWindowLongPtrW(window: isize, index: i32) -> isize;
    fn IsWindowVisible(window: isize) -> i32;
    fn IsWindowEnabled(window: isize) -> i32;
}

fn hwnd(window: &Window) -> Result<isize, String> {
    match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => Ok(handle.hwnd.get()),
        _ => Err("No live HWND".into()),
    }
}

fn qualify(main: &slint::Window, child: &slint::Window) -> Result<(), String> {
    main.with_winit_window(|main_native| {
        child.with_winit_window(|child_native| {
            let main_hwnd = hwnd(main_native)?;
            let child_hwnd = hwnd(child_native)?;
            // SAFETY: both HWNDs are borrowed from live Winit windows on their UI thread.
            // The read-only user32 calls retain no pointers and return scalar values.
            let (owner, child_style, main_style, visible) = unsafe {
                (GetWindow(child_hwnd, 4), // GW_OWNER
                 GetWindowLongPtrW(child_hwnd, -20), // GWL_EXSTYLE
                 GetWindowLongPtrW(main_hwnd, -20),
                 IsWindowVisible(child_hwnd))
            };
            if owner != main_hwnd { return Err(format!("Owner HWND {owner:#x} != main {main_hwnd:#x}")); }
            if child_style & 0x0004_0000 != 0 { return Err("Dialog has WS_EX_APPWINDOW".into()); }
            if child_style & 0x0000_0080 != 0 { return Err("Dialog has WS_EX_TOOLWINDOW".into()); }
            if main_style & 0x0004_0000 == 0 { return Err("Main lost WS_EX_APPWINDOW".into()); }
            if visible == 0 { return Err("Dialog HWND is hidden".into()); }
            // SAFETY: the HWND is borrowed from the live child above; scalar query only.
            if unsafe { IsIconic(child_hwnd) } != 0 { return Err("Explicit activation left the dialog minimized".into()); }
            let owner_position = main_native.outer_position().map_err(|error| error.to_string())?;
            let owner_size = main_native.outer_size();
            let child_position = child_native.outer_position().map_err(|error| error.to_string())?;
            if i64::from(child_position.x) < i64::from(owner_position.x) + i64::from(owner_size.width) {
                return Err(format!("Dialog remained within main: {child_position:?}, {owner_position:?}, {owner_size:?}"));
            }
            println!("native-dialog HWND={child_hwnd:#x} owner={owner:#x} exstyle={child_style:#x} main-exstyle={main_style:#x} position={child_position:?} visible={visible}");
            Ok(())
        }).ok_or("Child native window was not realized")?
    }).ok_or("Main native window was not realized")?
}

fn qualify_tool(tool: &slint::Window) -> Result<(), String> {
    tool.with_winit_window(|native| {
        let handle = hwnd(native)?;
        // SAFETY: borrowed live Winit HWND, scalar UI-thread queries only.
        let (owner, style, visible) = unsafe {
            (GetWindow(handle, 4), GetWindowLongPtrW(handle, -20), IsWindowVisible(handle))
        };
        if owner != 0 { return Err("Working tool unexpectedly has a native owner".into()); }
        if style & 0x0004_0000 == 0 { return Err("Working tool lost WS_EX_APPWINDOW".into()); }
        if style & 0x0000_0080 != 0 { return Err("Working tool has WS_EX_TOOLWINDOW".into()); }
        if visible == 0 { return Err("Working tool HWND is hidden".into()); }
        println!("native-working-tool HWND={handle:#x} owner={owner:#x} exstyle={style:#x} visible={visible}; taskbar/Alt-Tab eligible under the process AppID");
        Ok(())
    }).ok_or("Working tool native window was not realized")?
}

fn qualify_initial_outer_bounds(main: &slint::Window, child: &slint::Window) -> Result<(), String> {
    main.with_winit_window(|owner| {
        let area = work_area(owner).ok_or("No owner monitor work area")?;
        child.with_winit_window(|native| {
            let position = native.outer_position().map_err(|error| error.to_string())?;
            let size = native.outer_size();
            if position.x < area.0 || position.y < area.1
                || i64::from(position.x) + i64::from(size.width) > i64::from(area.0) + i64::from(area.2)
                || i64::from(position.y) + i64::from(size.height) > i64::from(area.1) + i64::from(area.3)
            {
                return Err(format!("Decorated window exceeds monitor work area: {position:?} {size:?} area={area:?}"));
            }
            println!("native-initial-outer position={position:?} outer={size:?} workarea={area:?} scale={}", native.scale_factor());
            Ok(())
        }).ok_or("No realized child for initial outer bounds")?
    }).ok_or("No realized owner for initial outer bounds")?
}

#[test]
#[ignore = "requires an interactive Windows desktop; run this test alone with --ignored --test-threads=1"]
fn native_owned_dialog_has_no_separate_taskbar_button() {
    let mut builder: slint::winit_030::EventLoopBuilder =
        slint::winit_030::winit::event_loop::EventLoop::with_user_event();
    builder.with_any_thread(true);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_event_loop_builder(builder)
        .with_winit_window_attributes_hook(crate::configure_dialog_window_attributes)
        .select()
        .unwrap();
    let main = crate::MainWindow::new().unwrap();
    main.window()
        .set_size(slint::LogicalSize::new(640.0, 480.0));
    main.window()
        .set_position(slint::PhysicalPosition::new(80, 80));
    let weak = main.as_weak();
    let result = Rc::new(RefCell::new(Err("Desktop probe timed out".to_owned())));
    let outcome = result.clone();
    let mut child: Option<(crate::HelpWindow, DialogHost)> = None;
    let mut tool: Option<(crate::FindWindow, DialogHost)> = None;
    let mut cancelled: Option<slint::Weak<crate::HelpWindow>> = None;
    let mut sessions = DialogSessions::default();
    let mut initial_checked = false;
    let mut reactivated = false;
    let mut ticks = 0;
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(150),
        move || {
            ticks += 1;
            if ticks >= 40 {
                *outcome.borrow_mut() = Err("Native realization probe timed out".into());
                let _ = slint::quit_event_loop();
                return;
            }
            let Some(main) = weak.upgrade() else {
                let _ = slint::quit_event_loop();
                return;
            };
            if child.is_none() && main.window().has_winit_window() {
                let (pending, pending_host) = DialogHost::create(
                    sessions.allocate().unwrap(),
                    DialogOwner::new(&main),
                    DialogModality::OwnerModal,
                    crate::HelpWindow::new,
                )
                .unwrap();
                pending_host.show(pending.window()).unwrap();
                if pending.window().has_winit_window() {
                    *outcome.borrow_mut() =
                        Err("Probe did not exercise deferred native creation".into());
                    pending_host.close(pending.window());
                    let _ = slint::quit_event_loop();
                    return;
                }
                cancelled = Some(pending.as_weak());
                pending_host.close(pending.window());
                drop(pending);
                drop(pending_host);
                let creation = DialogHost::create(
                    sessions.allocate().unwrap(),
                    DialogOwner::new(&main),
                    DialogModality::OwnerModal,
                    crate::HelpWindow::new,
                );
                match creation {
                    Ok((component, host)) => {
                        component
                            .window()
                            .set_size(slint::LogicalSize::new(3200.0, 2000.0));
                        if let Err(error) = host.show(component.window()) {
                            *outcome.borrow_mut() = Err(error.to_string());
                            let _ = slint::quit_event_loop();
                            return;
                        }
                        child = Some((component, host));
                    }
                    Err(error) => {
                        *outcome.borrow_mut() = Err(error.to_string());
                        let _ = slint::quit_event_loop();
                    }
                }
                return;
            }
            if let Some((component, host)) = tool.as_ref() {
                if !host.is_realized() {
                    return;
                }
                *outcome.borrow_mut() = qualify_tool(component.window());
                host.close(component.window());
                let _ = slint::quit_event_loop();
            } else if let Some((component, host)) = child.as_ref() {
                if cancelled
                    .as_ref()
                    .is_some_and(|weak| weak.upgrade().is_some())
                {
                    *outcome.borrow_mut() =
                        Err("Closed pending native dialog retained its component".into());
                    let _ = slint::quit_event_loop();
                    return;
                }
                if !host.is_realized() {
                    return;
                }
                if !initial_checked {
                    *outcome.borrow_mut() =
                        qualify_initial_outer_bounds(main.window(), component.window());
                    if outcome.borrow().is_err() {
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    component
                        .window()
                        .set_size(slint::LogicalSize::new(320.0, 200.0));
                    main.window().with_winit_window(|native| {
                        let position = native.outer_position().unwrap();
                        component
                            .window()
                            .set_position(slint::PhysicalPosition::new(
                                position.x + native.outer_size().width as i32 + 32,
                                position.y,
                            ));
                    });
                    component
                        .window()
                        .with_winit_window(|native| native.set_minimized(true));
                    initial_checked = true;
                    return;
                }
                if !reactivated {
                    let minimized = component
                        .window()
                        .with_winit_window(|native| {
                            let handle = hwnd(native).unwrap();
                            // SAFETY: borrowed live child HWND; scalar query only.
                            (unsafe { IsIconic(handle) }) != 0
                        })
                        .unwrap_or(false);
                    if !minimized {
                        *outcome.borrow_mut() =
                            Err("Native minimize request was not observed".into());
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    host.activate(component.window());
                    reactivated = true;
                    return;
                }
                *outcome.borrow_mut() = qualify(main.window(), component.window());
                host.close(component.window());
                main.window().with_winit_window(|native| {
                    let handle = hwnd(native).unwrap();
                    // SAFETY: borrowed live owner HWND, synchronous scalar query only.
                    if unsafe { IsWindowEnabled(handle) } == 0 {
                        *outcome.borrow_mut() = Err("Main remains disabled after close".into());
                    }
                });
                if outcome.borrow().is_err() {
                    let _ = slint::quit_event_loop();
                    return;
                }
                match DialogHost::create_tool(
                    sessions.allocate().unwrap(),
                    DialogOwner::new(&main),
                    crate::FindWindow::new,
                ) {
                    Ok((component, host)) => {
                        if let Err(error) = host.show(component.window()) {
                            *outcome.borrow_mut() = Err(error.to_string());
                            let _ = slint::quit_event_loop();
                            return;
                        }
                        tool = Some((component, host));
                    }
                    Err(error) => {
                        *outcome.borrow_mut() = Err(error.to_string());
                        let _ = slint::quit_event_loop();
                    }
                }
            } else if ticks >= 40 {
                let _ = slint::quit_event_loop();
            }
        },
    );
    let event_loop_result = main.run();
    // Drop callback-owned hosts before releasing the owner, including event-loop failure.
    drop(timer);
    main.hide().unwrap();
    event_loop_result.unwrap();
    assert!(result.borrow().is_ok(), "{:?}", result.borrow());
}
