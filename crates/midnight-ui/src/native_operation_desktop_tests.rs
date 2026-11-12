//! Embedded operation dialogs on an interactive desktop, without filesystem mutations.
use super::*;
use midnight_core::*;
use slint::Model;
use std::{cell::Cell, sync::Arc};

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumThreadWindows(
        thread: u32,
        callback: Option<unsafe extern "system" fn(isize, isize) -> i32>,
        data: isize,
    ) -> i32;
    fn GetWindowTextW(window: isize, text: *mut u16, capacity: i32) -> i32;
    fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

struct Search {
    owner: isize,
    title: &'static str,
    found: Option<isize>,
    duplicate: bool,
}
unsafe extern "system" fn find_owned(window: isize, data: isize) -> i32 {
    // SAFETY: EnumThreadWindows invokes synchronously on this UI thread; data points
    // to the exclusively borrowed stack Search for the complete enumeration.
    let search = unsafe { &mut *(data as *mut Search) };
    // SAFETY: enumeration supplies a live same-thread HWND; read-only scalar calls.
    if unsafe { GetWindow(window, 4) } != search.owner || unsafe { IsWindowVisible(window) } == 0 {
        return 1;
    }
    let mut text = [0u16; 256];
    // SAFETY: text is a writable stack buffer with exactly the passed capacity.
    let count = unsafe { GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) };
    if count > 0 && String::from_utf16_lossy(&text[..count as usize]).contains(search.title) {
        search.duplicate |= search.found.replace(window).is_some();
    }
    1
}
pub(super) fn find_child(owner: isize, title: &'static str) -> Result<Option<isize>, String> {
    let mut search = Search {
        owner,
        title,
        found: None,
        duplicate: false,
    };
    // SAFETY: same-thread enumeration retains neither callback nor the stack data.
    let ok = unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find_owned),
            &mut search as *mut _ as isize,
        )
    };
    if ok == 0 || search.duplicate {
        return Err("Unable to identify one exact owned operation dialog".into());
    }
    Ok(search.found)
}
#[test]
#[ignore = "requires an interactive Windows desktop; run this test alone with --ignored --test-threads=1"]
fn embedded_operation_keeps_main_enabled_and_native_close_cancels_without_mutation() {
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
        .set_position(slint::PhysicalPosition::new(60, 60));
    let root = std::env::temp_dir().join("native-adapter-desktop-fixture");
    let paths = [root.join("left"), root.join("right")];
    let state = Rc::new(RefCell::new(crate::Presenter::new(
        paths[0].clone(),
        paths[1].clone(),
    )));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let observed = sent.clone();
    let unexpected_mutation = Rc::new(Cell::new(false));
    let unexpected = unexpected_mutation.clone();
    crate::install(
        &main,
        state.clone(),
        Rc::new(move |command| {
            if matches!(
                command,
                Command::ConfirmOperation { .. }
                    | Command::ConfirmMetadata { .. }
                    | Command::EnqueueCopy(_)
                    | Command::EnqueueSelectedCopy(_)
                    | Command::EnqueueMove(_)
                    | Command::EnqueueSelectedMove(_)
                    | Command::EnqueueClipboardMove(_)
                    | Command::EnqueueCapturedTransfer(_)
                    | Command::EnqueueTransferWithMetadata(_)
                    | Command::EnqueueArchiveCopy(_)
            ) {
                unexpected.set(true);
                return false;
            }
            observed.borrow_mut().push(command);
            true
        }),
    );
    for pane in [PaneId::Left, PaneId::Right] {
        let navigation = sent
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: target,
                    navigation,
                    ..
                } if *target == pane => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(1),
            paths[pane.index()].join("source.txt"),
            EntryKind::File,
        )]
        .into();
        state.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    crate::refresh(&main, &state.borrow());
    main.set_preferences_ready(true);
    let weak = main.as_weak();
    let result = Rc::new(RefCell::new(Err(
        "Native operation probe timed out".to_owned()
    )));
    let outcome = result.clone();
    let mut phase = 0;
    let mut ticks = 0;
    let mut operation = None;
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(100), move || {
        ticks += 1;
        let step = (|| -> Result<bool, String> {
            if ticks > 80 { return Err(format!("Operation probe timed out at phase {phase}")); }
            let main = weak.upgrade().ok_or("Main adapter was lost")?;
            let Some(owner) = main.window().with_winit_window(hwnd).transpose()? else { return Ok(false) };
            match phase {
                0 => {
                    let row = main.get_left_rows().iter().position(|row| row.name == "source.txt").ok_or("No source row")?;
                    main.invoke_row_click(0, row as i32, false, false);
                    main.invoke_mutation(0, 0);
                    let request = sent.borrow().iter().find_map(|command| match command {
                        Command::PrepareOperation(request) => Some(request.clone()), _ => None,
                    }).ok_or("Rename did not prepare its target")?;
                    operation = Some(request.id);
                    state.borrow_mut().apply(Event::Operation(OperationEvent::Preview(Arc::new(OperationPreview {
                        id: request.id, kind: request.kind, files: 1, folders: 0, file_bytes: Some(1),
                        sample: vec![MutationItem { path: paths[0].join("source.txt"), kind: EntryKind::File, metadata: EntryMetadata::default() }],
                    }))));
                    crate::refresh(&main, &state.borrow());
                    phase = 1;
                }
                1 | 3 => {
                    if !main.global::<crate::OperationPresentation>().get_active() { return Ok(false) }
                    let title = if phase == 1 { "Rename" } else { "Copy" };
                    if find_child(owner, title)?.is_some() {
                        return Err("A blocking operation escaped into a native child window".into());
                    }
                    // SAFETY: live same-thread main HWND; scalar state query only.
                    if unsafe { IsWindowEnabled(owner) } == 0 {
                        return Err("Embedded operation disabled its own native input host".into());
                    }
                    if !main.get_embedded_modal_open() || main.get_dialog_input_blocked()
                        || i_slint_core::window::WindowInner::from_pub(main.window()).active_popups().iter()
                            .all(|popup| matches!(popup.window_kind, i_slint_core::window::WindowKind::ToolTip))
                    {
                        return Err("Operation did not open an embedded modal popup".into());
                    }
                    main.window().set_size(slint::LogicalSize::new(420.0, 300.0));
                    // SAFETY: live main HWND; queue ordinary native close without retaining pointers.
                    if unsafe { PostMessageW(owner, 0x0010, 0, 0) } == 0 {
                        return Err("Could not post main-window WM_CLOSE".into());
                    }
                    phase += 1;
                }
                2 => {
                    if state.borrow().operations.pending.is_some() { return Ok(false) }
                    if !sent.borrow().iter().any(|command| matches!(command, Command::CancelOperation(id) if Some(*id) == operation)) {
                        return Err("Native close did not cancel the exact embedded operation".into());
                    }
                    if !main.window().is_visible() {
                        return Err("Native close hid the application while dismissing a dialog".into());
                    }
                    state.borrow_mut().request_copy_dialog(PaneId::Left, &|_| false);
                    if state.borrow().copies.draft.is_none() { return Err("Copy draft was not captured".into()) }
                    crate::refresh(&main, &state.borrow());
                    phase = 3;
                }
                4 => {
                    if state.borrow().copies.draft.is_some() { return Ok(false) }
                    if main.get_embedded_modal_open()
                        || i_slint_core::window::WindowInner::from_pub(main.window()).active_popups().iter()
                            .any(|popup| !matches!(popup.window_kind, i_slint_core::window::WindowKind::ToolTip))
                    { return Ok(false) }
                    // SAFETY: live same-thread owner HWND, scalar state query only.
                    if unsafe { IsWindowEnabled(owner) } == 0 || main.get_dialog_input_blocked() {
                        return Err("Copy close did not release embedded modality".into());
                    }
                    if !main.window().is_visible() || unexpected_mutation.get() {
                        return Err("Embedded Cancel hid main or attempted a mutation".into());
                    }
                    println!("Rename and Copy remain inside enabled main HWND; native close cancels exact drafts without mutation");
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
    main.hide().unwrap();
    event_loop_result.unwrap();
    assert!(result.borrow().is_ok(), "{:?}", result.borrow());
}
