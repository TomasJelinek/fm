//! Installed file-list presentation with deterministic UI time and command capture.
use super::modeless;
use midnight_core::*;
use midnight_ui::{FileListPresentation, MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct Boundary {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}
impl Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

pub struct Harness {
    pub main: MainWindow,
    pub native: Rc<MinimalSoftwareWindow>,
    pub state: Rc<RefCell<Presenter>>,
    pub commands: Rc<RefCell<Vec<Command>>>,
    pub accepting: Rc<Cell<bool>>,
    pub root: PathBuf,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}
impl Harness {
    pub fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(Boundary {
            windows: windows.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        let main = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&main);
        let root = std::env::temp_dir().join("embedded-file-list 文档");
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
        let mut preferences = UserPreferences::default();
        for tab in &mut preferences.tabs {
            tab.paths = vec![root.clone()];
        }
        preferences.file_list.template_history =
            vec!["文档$(FileName)$(CRLF)".into(), "$(FileSize)$(LF)".into()];
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let accepting = Rc::new(Cell::new(true));
        install(
            &main,
            state.clone(),
            Rc::new({
                let commands = commands.clone();
                let accepting = accepting.clone();
                move |command| {
                    if !accepting.get() {
                        return false;
                    }
                    commands.borrow_mut().push(command);
                    true
                }
            }),
        );
        for pane in [PaneId::Left, PaneId::Right] {
            let navigation = commands
                .borrow()
                .iter()
                .rev()
                .find_map(|command| match command {
                    Command::Navigate {
                        pane: target,
                        navigation,
                        ..
                    } if *target == pane => Some(*navigation),
                    _ => None,
                })
                .unwrap();
            let entries: Arc<[Entry]> = vec![
                Entry::new(EntryId(1), root.join("source 文档.txt"), EntryKind::File),
                Entry::new(EntryId(2), root.join("folder"), EntryKind::Directory),
            ]
            .into();
            state.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            });
        }
        main.set_preferences_ready(true);
        main.show().unwrap();
        refresh(&main, &state.borrow());
        let row = main
            .get_left_rows()
            .iter()
            .position(|row| row.name == "source 文档.txt")
            .unwrap();
        main.invoke_row_click(0, row as i32, false, false);
        commands.borrow_mut().clear();
        main.invoke_make_file_list(0);
        let native = windows.borrow()[0].clone();
        let h = Self {
            main,
            native,
            state,
            commands,
            accepting,
            root,
            windows,
            clock,
        };
        h.tick(0);
        h
    }
    pub fn tick(&self, millis: u64) {
        self.clock.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
        modeless::draw(&self.native, 1000, 740, "installed-file-list");
        assert_eq!(
            self.windows.borrow().len(),
            1,
            "file-list created a native child"
        );
    }
    pub fn apply(&self, event: Event) {
        self.state.borrow_mut().apply(event);
        refresh(&self.main, &self.state.borrow());
    }
    pub fn request(&self) -> FileListRequest {
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::RunFileList(request) => Some((**request).clone()),
                _ => None,
            })
            .unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        let form = self.main.global::<FileListPresentation>();
        if form.get_active() {
            self.accepting.set(true);
            form.invoke_cancel(form.get_token());
        }
        self.main.invoke_close_file_list_window();
        self.main.hide().unwrap();
        self.state.borrow_mut().retire_all();
    }
}
