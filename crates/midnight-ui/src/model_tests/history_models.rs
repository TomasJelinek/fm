use super::*;
use i_slint_core::model::{ModelChangeListener, ModelChangeListenerContainer};
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::pin::Pin;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn setup() -> (MainWindow, Rc<RefCell<Presenter>>) {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native))).unwrap();
    let root = std::env::temp_dir().join("history-model-fixture");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let window = MainWindow::new().unwrap();
    window.set_left_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Left)));
    window.set_right_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Right)));
    (window, presenter)
}

fn finish(presenter: &mut Presenter) {
    presenter.apply_with_sender(
        Event::ListingFinished {
            pane: PaneId::Left,
            navigation: NavigationId(presenter.panes[0].generation),
            entries: Arc::from([]),
            index: Arc::new(DirectoryIndex::from_entries(&[])),
        },
        &|_| true,
    );
}

fn visit(presenter: &mut Presenter, name: &str) {
    let path = std::env::temp_dir()
        .join("history-model-fixture")
        .join(name);
    assert!(presenter.navigate(PaneId::Left, path, None, &|_| true));
    finish(presenter);
}

#[derive(Default)]
struct Observer {
    resets: Cell<usize>,
    changes: Cell<usize>,
    model: RefCell<Option<ModelRc<NavigationChoiceRow>>>,
}

impl ModelChangeListener for Observer {
    fn row_changed(self: Pin<&Self>, row: usize) {
        self.changes.set(self.changes.get() + 1);
        assert!(
            self.model
                .borrow()
                .as_ref()
                .unwrap()
                .row_data(row)
                .is_some()
        );
    }
    fn row_added(self: Pin<&Self>, _: usize, _: usize) {}
    fn row_removed(self: Pin<&Self>, _: usize, _: usize) {}
    fn reset(self: Pin<&Self>) {
        self.resets.set(self.resets.get() + 1);
        let model = self.model.borrow();
        let model = model.as_ref().unwrap();
        if model.row_count() != 0 {
            assert!(
                model.row_data(0).is_some(),
                "notifications permit reentrant reads"
            );
        }
    }
}

fn observe(
    model: &ModelRc<NavigationChoiceRow>,
) -> Pin<Box<ModelChangeListenerContainer<Observer>>> {
    let observer = Box::pin(ModelChangeListenerContainer::<Observer>::default());
    *observer.model.borrow_mut() = Some(model.clone());
    model
        .model_tracker()
        .attach_peer(observer.as_ref().model_peer());
    observer
}

#[test]
fn history_models_follow_commits_branches_and_equal_length_tab_histories() {
    let (window, presenter) = setup();
    for name in ["a", "b", "c"] {
        visit(&mut presenter.borrow_mut(), name);
    }
    refresh(&window, &presenter.borrow());
    let back = window.get_left_back_history();
    let forward = window.get_left_forward_history();
    assert_eq!(back.row_count(), 2);
    let old = back.row_data(0).unwrap();
    assert!(old.label.ends_with('b'));
    let observer = observe(&back);
    presenter.borrow_mut().apply(Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation: NavigationId(0),
        scan: FolderSizeScanId(0),
        cancelled: false,
    });
    refresh(&window, &presenter.borrow());
    assert_eq!(observer.resets.get(), 0);
    assert_eq!(observer.changes.get(), 0);
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::folder(),
        pixels: Arc::new(IconPixels::new(1, 1, vec![23, 45, 67, 255]).unwrap()),
    });
    refresh(&window, &presenter.borrow());
    assert_eq!(
        observer.resets.get(),
        0,
        "late artwork must retain delegates"
    );
    assert_eq!(observer.changes.get(), 2);
    assert_eq!(back.row_data(0).unwrap().identity, old.identity);
    assert_eq!(back.row_data(0).unwrap().icon.size().width, 1);
    refresh(&window, &presenter.borrow());
    assert_eq!(observer.resets.get(), 0);
    assert_eq!(
        observer.changes.get(),
        2,
        "unchanged loaded artwork stays cached"
    );
    {
        let mut state = presenter.borrow_mut();
        assert!(state.choose_history_entry(PaneId::Left, false, &old.identity, &|_| true));
    }
    refresh(&window, &presenter.borrow());
    let pending_resets = observer.resets.get();
    finish(&mut presenter.borrow_mut());
    refresh(&window, &presenter.borrow());
    assert!(
        observer.resets.get() > pending_resets,
        "commit changes the current index"
    );
    assert_eq!(back.row_count(), 1);
    assert_eq!(forward.row_count(), 1);
    assert!(!presenter.borrow_mut().choose_history_entry(
        PaneId::Left,
        false,
        &old.identity,
        &|_| panic!("stale history cannot navigate"),
    ));
    let next = forward.row_data(0).unwrap();
    assert!(presenter.borrow_mut().choose_history_entry(
        PaneId::Left,
        true,
        &next.identity,
        &|_| true,
    ));
    finish(&mut presenter.borrow_mut());
    refresh(&window, &presenter.borrow());
    let previous = back.row_data(0).unwrap();
    assert!(presenter.borrow_mut().choose_history_entry(
        PaneId::Left,
        false,
        &previous.identity,
        &|_| true,
    ));
    finish(&mut presenter.borrow_mut());
    visit(&mut presenter.borrow_mut(), "d");
    refresh(&window, &presenter.borrow());
    assert_eq!(back.row_count(), 2);
    assert_eq!(
        forward.row_count(),
        0,
        "a new branch removes the former future"
    );
    let first = presenter.borrow().tabs(PaneId::Left)[0].id;
    assert!(presenter.borrow_mut().new_tab(PaneId::Left, &|_| true));
    finish(&mut presenter.borrow_mut());
    for name in ["e", "f"] {
        visit(&mut presenter.borrow_mut(), name);
    }
    refresh(&window, &presenter.borrow());
    assert_eq!(back.row_count(), 2);
    assert!(back.row_data(0).unwrap().label.ends_with('e'));
    let second_token = back.row_data(0).unwrap().identity;
    assert!(
        presenter
            .borrow_mut()
            .select_tab(PaneId::Left, first, &|_| true)
    );
    finish(&mut presenter.borrow_mut());
    refresh(&window, &presenter.borrow());
    assert_eq!(back.row_count(), 2);
    assert!(back.row_data(0).unwrap().label.ends_with('b'));
    assert_ne!(back.row_data(0).unwrap().identity, second_token);
    assert_eq!(window.get_right_back_history().row_count(), 0);
}

#[test]
fn command_history_models_follow_late_load_shell_dedup_clear_and_late_icons() {
    let (window, presenter) = setup();
    refresh(&window, &presenter.borrow());
    let model = window.get_command_history();
    assert_eq!(model.row_count(), 0);
    presenter.borrow_mut().apply(Event::DesktopContext {
        system_locations: Vec::new(),
        shell: ShellFlavor::PowerShell,
        locations: vec![],
        network_position: 0,
    });
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            command_history: vec![CommandHistoryEntry {
                command: "echo 文档".into(),
                shell: ShellFlavor::Cmd,
            }],
            ..Default::default()
        }))));
    refresh(&window, &presenter.borrow());
    assert_eq!(model.row_count(), 1);
    assert_eq!(model.row_data(0).unwrap().label, "cmd · echo 文档");
    assert_eq!(model.row_data(0).unwrap().identity, "echo 文档");
    let observer = observe(&model);
    refresh(&window, &presenter.borrow());
    assert_eq!(observer.resets.get(), 0);
    assert_eq!(
        observer.changes.get(),
        0,
        "missing command artwork stays cached"
    );
    let request = Cell::new(None);
    assert!(
        presenter
            .borrow_mut()
            .submit_command("echo 文档", &|command| {
                if let Command::LaunchTerminal { request: id, .. } = command {
                    request.set(Some(id));
                }
                true
            })
    );
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request: request.get().unwrap(),
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: CommandHistoryEntry {
                command: "echo 文档".into(),
                shell: ShellFlavor::PowerShell,
            },
        }),
    });
    refresh(&window, &presenter.borrow());
    assert_eq!(model.row_count(), 1);
    assert_eq!(model.row_data(0).unwrap().label, "PowerShell · echo 文档");
    assert_eq!(
        observer.resets.get(),
        1,
        "same text and count still changes shell metadata"
    );
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::action(ActionIcon::Terminal),
        pixels: Arc::new(IconPixels::new(1, 1, vec![23, 45, 67, 255]).unwrap()),
    });
    refresh(&window, &presenter.borrow());
    assert_eq!(observer.resets.get(), 1);
    assert_eq!(observer.changes.get(), 1);
    assert_eq!(model.row_data(0).unwrap().icon.size().width, 1);
    refresh(&window, &presenter.borrow());
    assert_eq!(observer.resets.get(), 1);
    assert_eq!(
        observer.changes.get(),
        1,
        "unchanged command artwork stays cached"
    );
    let token = model.row_data(0).unwrap().identity;
    assert_eq!(
        presenter.borrow_mut().recall_command(&token).as_deref(),
        Some("echo 文档")
    );
    presenter.borrow_mut().clear_command_history(&|_| true);
    refresh(&window, &presenter.borrow());
    assert_eq!(model.row_count(), 0);
    assert!(model.row_data(0).is_none());
    assert!(presenter.borrow_mut().recall_command(&token).is_none());
    assert_eq!(observer.resets.get(), 2);
    let retired = presenter.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}
