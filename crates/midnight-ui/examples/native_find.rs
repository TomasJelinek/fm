//! Explicit native Find regression probe. Synthetic results only; no attachment contents are read.
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter};
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
    winit_030::{CustomApplicationHandler, EventResult, winit},
};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

type FindNative = Rc<RefCell<Option<Rc<dyn WindowAdapter>>>>;
struct ObserveFind(FindNative);
impl CustomApplicationHandler for ObserveFind {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        native: Option<&winit::window::Window>,
        window: Option<&slint::Window>,
        _: &winit::event::WindowEvent,
    ) -> EventResult {
        if native.is_some_and(|native| native.title() == "Find files · Midnight Explorer")
            && let Some(window) = window
        {
            *self.0.borrow_mut() = Some(WindowInner::from_pub(window).window_adapter());
        }
        EventResult::Propagate
    }
}

fn item(root: ItemRc, label: &str) -> Option<ItemRc> {
    if root.accessible_role() == AccessibleRole::Button
        && root
            .accessible_string_property(AccessibleStringProperty::Label)
            .as_deref()
            == Some(label)
    {
        return Some(root);
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = item(current, label) {
            return Some(found);
        }
    }
    None
}
fn click(window: &slint::Window, label: &str) {
    let button = item(
        ItemRc::new_root(WindowInner::from_pub(window).component()),
        label,
    )
    .unwrap_or_else(|| panic!("Missing Find button: {label}"));
    let bounds = button.geometry();
    let origin = button.map_to_native_window(bounds.origin);
    let position = slint::LogicalPosition::new(
        origin.x + bounds.width() / 2.,
        origin.y + bounds.height() / 2.,
    );
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let native: FindNative = Rc::default();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .with_winit_custom_application_handler(ObserveFind(native.clone()))
        .select()?;
    let main = MainWindow::new()?;
    let root = std::env::current_dir()?
        .join("_build/goal/find-crash/Attachments/ooa-native-fixture/long-result-root");
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::refresh(&main, &state.borrow());
    main.window().set_size(slint::LogicalSize::new(1000., 700.));
    let weak = main.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0;
    let mut waiting = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(350),
        move || {
            let main = weak.upgrade().unwrap();
            if step == 0 {
                main.invoke_find_files(0);
                step += 1;
                return;
            }
            let Some(find) = native.borrow().clone() else {
                waiting += 1;
                assert!(waiting < 40, "Native Find window was not observed");
                return;
            };
            eprintln!("native Find step {step}");
            let request = || {
                commands
                    .borrow()
                    .iter()
                    .find_map(|command| match command {
                        Command::StartFind(request) => Some(request.clone()),
                        _ => None,
                    })
                    .unwrap()
            };
            let apply = |update| {
                let request = request();
                state.borrow_mut().apply(Event::Find(Box::new(FindEvent {
                    session: request.session,
                    generation: request.generation,
                    update,
                })));
                midnight_ui::refresh(&main, &state.borrow());
            };
            match step {
                1 => {
                    find.window().set_size(slint::LogicalSize::new(1050., 700.));
                    click(find.window(), "Find");
                    assert_eq!(request().masks, "*.*");
                    assert!(request().recursive);
                    let results = ["b.rs", "a.txt"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, name)| {
                            Arc::new(FindResult {
                                id: FindResultId(index as u64 + 1),
                                duplicate_group: None,
                                duplicate_group_size: 0,
                                entry: FindEntry {
                                    path: root.join(name),
                                    kind: EntryKind::File,
                                    metadata: EntryMetadata {
                                        size: Some(1),
                                        ..Default::default()
                                    },
                                    identity: TransferIdentity {
                                        device: 1,
                                        object: [index as u8; 16],
                                    },
                                    revision: FindRevision([0; 6]),
                                },
                            })
                        })
                        .collect::<Vec<_>>();
                    apply(FindUpdate::Batch(results.into()));
                    apply(FindUpdate::Sorted(Arc::new(FindOrder {
                        revision: 0,
                        sort: FindSortSpec::default(),
                        ids: vec![FindResultId(1), FindResultId(2)].into(),
                        positions: vec![0, 1].into(),
                    })));
                    eprintln!(
                        "Find click accepted: recursive *.*; batch and initial order published"
                    );
                }
                2 => {
                    apply(FindUpdate::Sorted(Arc::new(FindOrder {
                        revision: 0,
                        sort: FindSortSpec::default(),
                        ids: vec![FindResultId(2), FindResultId(1)].into(),
                        positions: vec![1, 0].into(),
                    })));
                    eprintln!("Streamed sorted order replaced during normal presenter refresh");
                }
                3 | 4 => {
                    click(find.window(), "Sort by Extension");
                    let sort = commands
                        .borrow()
                        .iter()
                        .rev()
                        .find_map(|command| match command {
                            Command::SortFind(request) => Some(*request),
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(sort.sort.column, FindSortColumn::Extension);
                    assert_eq!(sort.sort.descending, step == 4);
                    let (ids, positions) = if step == 3 {
                        (vec![FindResultId(1), FindResultId(2)], vec![0, 1])
                    } else {
                        (vec![FindResultId(2), FindResultId(1)], vec![1, 0])
                    };
                    apply(FindUpdate::Sorted(Arc::new(FindOrder {
                        revision: sort.revision,
                        sort: sort.sort,
                        ids: ids.into(),
                        positions: positions.into(),
                    })));
                    eprintln!(
                        "Extension header click accepted: descending={}",
                        sort.sort.descending
                    );
                }
                5 => apply(FindUpdate::Finished(FindSummary {
                    completion: FindCompletion::Completed,
                    progress: FindProgress::default(),
                    reached_budgets: vec![],
                    diagnostics: vec![],
                })),
                6 => {
                    click(find.window(), "Find");
                    assert_eq!(
                        commands
                            .borrow()
                            .iter()
                            .filter(|command| matches!(command, Command::StartFind(_)))
                            .count(),
                        2
                    );
                    eprintln!("Find restart click accepted: second StartFind dispatched");
                }
                _ => {
                    main.invoke_close_find_windows();
                    state.borrow_mut().retire_all();
                    let retired = state.borrow_mut().take_retired();
                    std::thread::spawn(move || drop(retired)).join().unwrap();
                    eprintln!(
                        "native Find wildcard/sorted replacement/extension/restart probe passed"
                    );
                    slint::quit_event_loop().unwrap();
                    return;
                }
            }
            // Mirror the application's backend reclamation without dropping large
            // result owners while the native UI is notifying its models.
            let retired = state.borrow_mut().take_retired();
            std::thread::spawn(move || drop(retired)).join().unwrap();
            step += 1;
        },
    );
    main.run()?;
    Ok(())
}
