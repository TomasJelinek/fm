use midnight_compare_ui::{ComparatorWindows, WindowPlacement};
use midnight_core::{PaneId, ViewerAppearance};
use midnight_ui::{MainWindow, Presenter, Theme};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    runtime: &tokio::runtime::Handle,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> Rc<ComparatorWindows> {
    let windows = Rc::new(ComparatorWindows::new(service(runtime, filesystem)));
    let manager = windows.clone();
    let weak = window.as_weak();
    window.on_compare_files(move |pane| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let result = (|| {
            let launch = presenter.borrow().file_compare_launch(if pane == 0 {
                PaneId::Left
            } else {
                PaneId::Right
            })?;
            let theme = window.global::<Theme>();
            let rgba = |color: slint::Color| {
                let color = color.to_argb_u8();
                [color.red, color.green, color.blue, color.alpha]
            };
            let appearance = ViewerAppearance {
                family: theme.get_font_family().to_string(),
                font_size: theme.get_font_size().max(11.0),
                foreground: rgba(theme.get_text()),
                background: rgba(theme.get_pane_background()),
                selection: rgba(theme.get_selection_background()),
                accent: rgba(theme.get_accent()),
            };
            let placement = WindowPlacement::capture(window.window());
            let state = presenter.clone();
            let keys = Rc::new(move |chord: &_, control| state.borrow().viewer_key(chord, control));
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

pub(crate) fn service(
    runtime: &tokio::runtime::Handle,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> midnight_compare_engine::Service {
    midnight_compare_engine::Service::with_native_capture(
        runtime.clone(),
        crate::native_content::factory(filesystem),
    )
}
