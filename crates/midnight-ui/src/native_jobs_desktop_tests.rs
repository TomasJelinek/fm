//! Opt-in native Jobs/decision qualification; no actual backend or file mutation.
use super::*;
use midnight_core::*;
use slint::Model;
use std::{io, sync::Arc};

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> isize;
    fn SetForegroundWindow(window: isize) -> i32;
    fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
}
fn modeless_child(owner: isize, title: &'static str) -> Result<Option<isize>, String> {
    let Some(child) = operation_adapters::find_child(owner, title)? else {
        return Ok(None);
    };
    // SAFETY: child and owner are live same-thread probe HWNDs. These calls read
    // scalar state only and retain no pointers.
    unsafe {
        if IsWindowEnabled(owner) == 0 {
            return Err(format!("{title} disabled its modeless owner"));
        }
        if GetWindowLongPtrW(child, -20) & (0x0004_0000 | 0x0000_0080) != 0 {
            return Err(format!("{title} has a separate taskbar/tool-window style"));
        }
    }
    Ok(Some(child))
}
fn native_close(child: isize) -> Result<(), String> {
    // SAFETY: child is the exact same-thread HWND returned by modeless_child.
    // WM_CLOSE has scalar parameters; no mutation command or pointer is sent.
    if unsafe { PostMessageW(child, 0x0010, 0, 0) } == 0 {
        return Err("Unable to post native WM_CLOSE".into());
    }
    Ok(())
}

#[test]
#[ignore = "requires an interactive Windows desktop; run alone with --ignored --test-threads=1"]
fn native_jobs_close_preserves_work_and_background_decision_preserves_foreground() {
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
    let other = crate::AboutWindow::new().unwrap();
    main.window()
        .set_size(slint::LogicalSize::new(640.0, 480.0));
    let path = std::env::temp_dir().join("native-jobs-probe-no-io");
    let state = Rc::new(RefCell::new(crate::Presenter::new(
        path.clone(),
        path.clone(),
    )));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let observed = sent.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        observed.borrow_mut().push(command);
        true
    });
    crate::install(&main, state.clone(), send.clone());
    main.set_preferences_ready(true);
    let result = Rc::new(RefCell::new(Err("Native Jobs probe timed out".to_owned())));
    let outcome = result.clone();
    let weak = main.as_weak();
    let other_weak = other.as_weak();
    let mut phase = 0;
    let mut ticks = 0;
    let mut other_hwnd = 0;
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
        ticks += 1;
        let step = (|| -> Result<bool, String> {
            if ticks > 80 {
                return Err(format!("Native Jobs probe timed out at phase {phase}"));
            }
            let main = weak.upgrade().ok_or("Main adapter was lost")?;
            let Some(owner) = main.window().with_winit_window(hwnd).transpose()? else {
                return Ok(false);
            };
            match phase {
                0 => {
                    state.borrow_mut().apply_with_sender(Event::DragDrop(DragDropEvent::DropAccepted {
                        drag: None, token: DropToken(1), pane: PaneId::Right, item_count: 1,
                        intent: TransferIntent::Copy, verification: None,
                        destination: Arc::new(path.clone()),
                    }), send.as_ref());
                    crate::refresh(&main, &state.borrow());
                    if !main.get_operations().iter().any(|row| row.identity == "copy-job:1") {
                        return Err("Probe did not obtain exact copy operation 1".into());
                    }
                    main.invoke_show_operations();
                    sent.borrow_mut().clear();
                    phase = 1;
                }
                1 => {
                    let Some(jobs) = modeless_child(owner, "Jobs")? else { return Ok(false) };
                    native_close(jobs)?;
                    phase = 2;
                }
                2 => {
                    if modeless_child(owner, "Jobs")?.is_some() { return Ok(false) }
                    if sent.borrow().iter().any(|command| matches!(command,
                        Command::CancelCopy(_) | Command::DecideCopy(_))) {
                        return Err("Closing Jobs canceled backend work".into());
                    }
                    let other = other_weak.upgrade().ok_or("Foreground probe was lost")?;
                    other.show().map_err(|error| error.to_string())?;
                    other_hwnd = other.window().with_winit_window(hwnd)
                        .ok_or("Foreground probe was not realized")??;
                    // SAFETY: activates only this test's own live independent HWND.
                    if unsafe { SetForegroundWindow(other_hwnd) } == 0 {
                        // Windows may preserve a user's foreground application instead.
                        // Observe that independent window; never bypass foreground policy.
                        // SAFETY: scalar query only; no input is sent to this HWND.
                        other_hwnd = unsafe { GetForegroundWindow() };
                        if other_hwnd == 0 || other_hwnd == owner {
                            return Err("No independent foreground window for this probe".into());
                        }
                    }
                    println!("independent foreground baseline HWND={other_hwnd:#x}");
                    phase = 3;
                }
                3 => {
                    // SAFETY: scalar query of current foreground HWND.
                    if unsafe { GetForegroundWindow() } != other_hwnd {
                        return Err("Foreground setup was not retained".into());
                    }
                    state.borrow_mut().apply_with_sender(Event::Copy(CopyEvent::Failure(Arc::new(
                        CopyFailure {
                            id: OperationId(1), version: 9,
                            error: FsError::from_io("write", &path,
                                io::Error::from(io::ErrorKind::PermissionDenied)),
                            source: Some(path.join("source.txt")), destination: path.join("target.txt"),
                            stage: CopyErrorStage::Writing, can_retry: true, can_skip: true,
                        }
                    ))), send.as_ref());
                    crate::refresh(&main, &state.borrow());
                    phase = 4;
                }
                4 => {
                    let Some(decision) = modeless_child(owner, "Operation error")? else {
                        return Ok(false);
                    };
                    // SAFETY: scalar query; no input sent to any user application.
                    if unsafe { GetForegroundWindow() } != other_hwnd {
                        return Err("Background operation decision stole foreground".into());
                    }
                    native_close(decision)?;
                    phase = 5;
                }
                5 => {
                    if modeless_child(owner, "Operation error")?.is_some() { return Ok(false) }
                    let commands = sent.borrow();
                    let decisions: Vec<_> = commands.iter().filter_map(|command| match command {
                        Command::DecideCopy(decision) => Some(decision), _ => None,
                    }).collect();
                    if decisions.len() != 1 || decisions[0].id != OperationId(1)
                        || decisions[0].version != 9 || !matches!(decisions[0].choice, CopyChoice::Cancel) {
                        return Err("Native decision close did not cancel its one exact request".into());
                    }
                    // SAFETY: read-only scalar queries on this test's windows.
                    if unsafe { GetForegroundWindow() } != other_hwnd
                        || unsafe { IsWindowEnabled(owner) } == 0 {
                        return Err("Decision closure changed unrelated focus or owner modality".into());
                    }
                    println!("native Jobs/Decision: owned without taskbar entry; owner stays enabled; Jobs close preserves work; background show/close preserves independent foreground; WM_CLOSE sends exact Cancel 1:9");
                    return Ok(true);
                }
                _ => unreachable!(),
            }
            Ok(false)
        })();
        match step {
            Ok(false) => {}
            Ok(true) => { *outcome.borrow_mut() = Ok(()); let _ = slint::quit_event_loop(); }
            Err(error) => { *outcome.borrow_mut() = Err(error); let _ = slint::quit_event_loop(); }
        }
    });
    let event_loop_result = main.run();
    drop(timer);
    main.invoke_close_decision_windows();
    main.invoke_close_jobs_window();
    other.hide().unwrap();
    main.hide().unwrap();
    event_loop_result.unwrap();
    assert!(result.borrow().is_ok(), "{:?}", result.borrow());
}
