use midnight_compare_ui::{ComparatorWindows, WindowPlacement};
use midnight_core::{Command, PaneId};
use midnight_ui::{MainWindow, Presenter};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<dyn Fn(Command) -> bool>,
    execution: &crate::ContentExecution,
    picker: Arc<dyn midnight_core::PathPickerService>,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    backend: crate::comparator_external::Backend,
) -> Rc<ComparatorWindows> {
    let windows = Rc::new(
        ComparatorWindows::new(
            backend
                .configure(service(
                    &execution.transient,
                    execution.resident.clone(),
                    filesystem,
                ))
                .with_path_picker(picker),
        )
        .with_picker_owner(midnight_ui::native_path_picker_owner),
    );
    // A wheel notch moves the compared text as far as in a viewer, and nothing glides
    // when the system asks for less motion; the windows read both as the viewers do.
    windows.set_wheel_setting(Rc::new(midnight_ui::system_wheel_setting));
    windows.set_motion_preference(Rc::new(midnight_ui::system_reduces_motion));
    let settings = presenter.clone();
    let compare_commands = commands.clone();
    windows.set_font_listener(Rc::new(move |family| {
        settings.borrow_mut().save_viewer_preferences(
            midnight_core::ViewerPreferenceChange::ComparisonFont(family),
            commands.as_ref(),
        );
    }));
    let manager = windows.clone();
    let weak = window.as_weak();
    window.on_compare_files(move |pane| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let result = (|| {
            // An open filename search closes; it never stops the comparison.
            let launch = presenter.borrow_mut().compare_command(
                if pane == 0 {
                    PaneId::Left
                } else {
                    PaneId::Right
                },
                compare_commands.as_ref(),
            );
            midnight_ui::refresh(&window, &presenter.borrow());
            let launch = launch?;
            let mut appearance = crate::content_appearance::capture(&window);
            // Document typography stays independent of the manager's primed UI font.
            appearance.family = presenter
                .borrow()
                .viewer_preferences()
                .comparison_font
                .clone();
            appearance.font_size = appearance.font_size.max(11.0);
            let placement = WindowPlacement::capture(window.window());
            let state = presenter.clone();
            let keys =
                Rc::new(move |chord: &_, control| state.borrow().comparison_key(chord, control));
            manager
                .open(launch, appearance, placement, keys)
                .map(|_| ())
        })();
        if let Err(error) = result {
            presenter.borrow_mut().report_viewer_error(error);
            midnight_ui::refresh(&window, &presenter.borrow());
        }
    });
    windows
}

/// Comparators opened before the initial settings read show the default comparison font;
/// apply the loaded one, merged with edits made before the read, as viewers restore their
/// preferences. Call once that read, successful or failed, was applied.
pub(crate) fn settings_applied(windows: &ComparatorWindows, presenter: &RefCell<Presenter>) {
    let family = presenter
        .borrow()
        .viewer_preferences()
        .comparison_font
        .clone();
    // No Presenter borrow is held while windows send their inputs.
    windows.restore_font(&family);
}

pub(crate) fn service(
    runtime: &tokio::runtime::Handle,
    compare_spawner: midnight_work_budget::resident::Spawner,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> midnight_compare_engine::Service {
    let storage = filesystem.clone();
    midnight_compare_engine::Service::with_native_capture(
        runtime.clone(),
        compare_spawner,
        crate::native_content::factory(filesystem),
    )
    // Queried by the comparison worker; any error or other kind stays exclusive.
    .with_storage_hint(Arc::new(move |path| {
        use midnight_core::StorageService;
        storage
            .storage_kind(path.parent().unwrap_or(path))
            .is_ok_and(|kind| kind == midnight_core::StorageKind::LocalSsd)
    }))
}
