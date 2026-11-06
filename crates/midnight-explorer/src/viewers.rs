use midnight_core::{Command, PaneId, ViewerAppearance};
use midnight_ui::{MainWindow, Presenter, Theme};
use midnight_viewer_ui::ViewerWindows;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};
use tokio::sync::mpsc;

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    commands: mpsc::Sender<Command>,
    runtime: &tokio::runtime::Handle,
) -> Rc<ViewerWindows> {
    let viewers = Rc::new(ViewerWindows::new(midnight_viewer_engine::Service::new(
        runtime.clone(),
    )));
    let manager = viewers.clone();
    let weak = window.as_weak();
    window.on_view_file(move |pane, basic| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let pane = if pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        let launch = presenter.borrow().viewer_launch(pane, basic);
        let Some(launch) = launch else {
            return;
        };
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
        let key_state = presenter.clone();
        let keys = Rc::new(move |event: &_, search| key_state.borrow().viewer_key(event, search));
        let copy_state = presenter.clone();
        let sender = commands.clone();
        let copy = Rc::new(move |text| {
            copy_state
                .borrow_mut()
                .copy_viewer_text(text, &|command| sender.try_send(command).is_ok())
        });
        let size = window
            .window()
            .size()
            .to_logical(window.window().scale_factor());
        if let Err(error) = manager.open(launch, appearance, size, keys, copy) {
            presenter.borrow_mut().report_viewer_error(error);
            midnight_ui::refresh(&window, &presenter.borrow());
        }
    });
    viewers
}
