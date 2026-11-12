//! Run beside an explicitly staged trusted `viewers/` directory. This probe never uses settings.
mod acceptance;

use midnight_core::*;
use midnight_viewer_ui::{ViewerWindow, ViewerWindows, WindowPlacement};
use slint::ComponentHandle;
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let output = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: native_acceptance output-directory [existing-fixtures-directory]")?,
    );
    let existing = arguments.next().map(PathBuf::from);
    eprintln!("Probe executable: {}", std::env::current_exe()?.display());
    eprintln!(
        "Selected plugin runtime: {}",
        midnight_plugin_host::installed_executable()?.display()
    );
    let paths = acceptance::fixtures(&output, existing.as_deref())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let manager = Rc::new(ViewerWindows::new(midnight_viewer_engine::Service::new(
        runtime.handle().clone(),
    )));
    let parent = ViewerWindow::new()?;
    parent.set_caption("Viewer acceptance parent — no preferences".into());
    parent
        .window()
        .set_size(slint::LogicalSize::new(1000.0, 700.0));
    parent
        .window()
        .set_position(slint::PhysicalPosition::new(90, 90));
    parent.show()?;
    let entries: Arc<[Entry]> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| Entry::new(EntryId(index as u64), path.clone(), EntryKind::File))
        .collect();
    let bindings = ShortcutMap::new(&[])?;
    let opening_manager = manager.clone();
    // Capture after the native event loop establishes the parent's monitor DPI and client size.
    let mut open = Some(move |parent: &ViewerWindow| {
        opening_manager.open(
            ViewerLaunch {
                find: None,
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(Default::default()),
                focused: paths[0].clone(),
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                archive: None,
                basic: false,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 16.0,
                foreground: [225, 229, 236, 255],
                background: [25, 29, 37, 255],
                selection: [55, 86, 124, 255],
                accent: [133, 185, 244, 255],
            },
            WindowPlacement::capture(parent.window()),
            Rc::new(move |chord, search| {
                bindings
                    .resolve(
                        chord,
                        if search {
                            ShortcutContext::ViewerSearch
                        } else {
                            ShortcutContext::Viewer
                        },
                    )
                    .map(|action| action.id.clone())
            }),
            Rc::new(|_| {}),
        )
    });
    let mut window: Option<slint::Weak<ViewerWindow>> = None;
    let probe = Rc::new(RefCell::new(acceptance::Probe::new(
        output,
        runtime.handle().clone(),
    )));
    let active = probe.clone();
    let parent_weak = parent.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(parent) = parent_weak.upgrade() else {
                return;
            };
            if let Some(open) = open.take() {
                match open(&parent) {
                    Ok(opened) => window = Some(opened),
                    Err(error) => {
                        active.borrow_mut().error = Some(error);
                        let _ = slint::quit_event_loop();
                    }
                }
                return;
            }
            let Some(window) = window.as_ref().and_then(slint::Weak::upgrade) else {
                return;
            };
            let mut probe = active.borrow_mut();
            match probe.step(&window, &parent) {
                Ok(false) => {}
                result => {
                    probe.error = result.err().map(|error| error.to_string());
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    slint::run_event_loop()?;
    timer.stop();
    manager.close_all();
    parent.hide()?;
    let mut probe = probe.borrow_mut();
    for write in probe.writes.drain(..) {
        runtime.block_on(write)??;
    }
    if let Some(error) = probe.error.take() {
        return Err(error.into());
    }
    if probe.stage != acceptance::COMPLETE {
        return Err("Viewer acceptance ended before completing all checks".into());
    }
    eprintln!("PASS: native text/binary/source/Markdown/image/PDF, controls, placement and Escape");
    Ok(())
}
