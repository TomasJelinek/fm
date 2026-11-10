//! Explicit desktop probe: only in-memory metadata previews, never native writes.
#[path = "../tests/support/ui.rs"]
mod ui;
use i_slint_core::{
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .select()?;
    let window = MainWindow::new()?;
    let root = std::env::current_dir()?;
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &window,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        root.join("Žluťoučký.TXT"),
        EntryKind::File,
    )]
    .into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::refresh(&window, &state.borrow());
    window
        .window()
        .set_size(slint::LogicalSize::new(1000., 700.));
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            let window = weak.upgrade().unwrap();
            eprintln!("metadata native step {step}");
            match step {
                0 => {
                    window.invoke_row_click(0, 1, false, false);
                    window.invoke_open_metadata(0, false);
                }
                1 | 8 => {
                    if step == 8 {
                        assert!(
                            WindowInner::from_pub(window.window())
                                .active_popups()
                                .iter()
                                .any(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
                            "visible tooltip before review"
                        );
                    }
                    let id = commands
                        .borrow()
                        .iter()
                        .find_map(|c| match c {
                            Command::PrepareOperation(request) => Some(request.id),
                            _ => None,
                        })
                        .unwrap();
                    let (revision, patch) = if step == 1 {
                        (0, MetadataPatch::default())
                    } else {
                        commands
                            .borrow()
                            .iter()
                            .rev()
                            .find_map(|c| match c {
                                Command::PreviewMetadata {
                                    revision, patch, ..
                                } => Some((*revision, patch.clone())),
                                _ => None,
                            })
                            .unwrap()
                    };
                    state
                        .borrow_mut()
                        .apply(Event::Operation(OperationEvent::MetadataPreview(Arc::new(
                            MetadataPreview {
                                id,
                                revision,
                                patch,
                                capabilities: MetadataCapabilities {
                                    windows_attributes: 39,
                                    compressed: true,
                                    created: true,
                                    modified: true,
                                    accessed: true,
                                    rename: true,
                                    ..Default::default()
                                },
                                total: 1,
                                conflicts: 0,
                                unsupported: 0,
                                hard_linked: 0,
                                sample: vec![],
                                errors: vec![],
                            },
                        ))));
                    midnight_ui::refresh(&window, &state.borrow());
                }
                2 => ui::click_popup(&window, "Read-only", AccessibleRole::Checkbox),
                3 => ui::click_popup(&window, "Review changes", AccessibleRole::Button),
                4 => {
                    let point = {
                        let item = ui::popup_item(&window, "Read-only", AccessibleRole::Checkbox);
                        let rect = item.geometry();
                        let origin = item.map_to_native_window(rect.origin);
                        slint::LogicalPosition::new(
                            origin.x + rect.width() / 2.,
                            origin.y + rect.height() / 2.,
                        )
                    };
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                            position: point,
                        });
                }
                5..=7 => {}
                9 => {
                    ui::popup_item(&window, "Apply", AccessibleRole::Button);
                }
                10 => ui::click_popup(&window, "Back to editing", AccessibleRole::Button),
                11 => ui::click_popup(&window, "Cancel", AccessibleRole::Button),
                _ => {
                    assert!(window.get_metadata_token().is_empty());
                    slint::quit_event_loop().unwrap();
                }
            }
            step += 1;
        },
    );
    window.run()?;
    eprintln!("metadata native review completed");
    Ok(())
}
