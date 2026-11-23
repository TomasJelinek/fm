//! Show the presenter's installation notice in the main window and route its buttons.
use crate::{InstallationNoticePresentation, MainWindow, presenter::Presenter};
use midnight_core::Command;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let notice = window.global::<InstallationNoticePresentation>();
    let view = presenter.installation_notice.view();
    notice.set_visible(view.is_some());
    let view = view.unwrap_or_default();
    if notice.get_body().as_str() != view.body {
        notice.set_body(view.body.into());
    }
    if notice.get_detail().as_str() != view.detail {
        notice.set_detail(view.detail.into());
    }
    notice.set_busy(view.busy);
    let actions = notice.get_actions();
    if actions.row_count() != view.actions.len()
        || view
            .actions
            .iter()
            .enumerate()
            .any(|(index, label)| actions.row_data(index).as_deref() != Some(*label))
    {
        let labels: Vec<SharedString> = view.actions.iter().map(|label| (*label).into()).collect();
        notice.set_actions(ModelRc::from(Rc::new(VecModel::from(labels))));
    }
    if presenter.installation_notice.close_requested.take() {
        // The copy has started: close this session as Exit would, outside this refresh.
        let weak = window.as_weak();
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                window.invoke_exit();
            }
        });
    }
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let notice = window.global::<InstallationNoticePresentation>();
    let weak = window.as_weak();
    let (state, sender) = (presenter.clone(), send.clone());
    notice.on_act(move |index| {
        let (Some(window), Ok(index)) = (weak.upgrade(), usize::try_from(index)) else {
            return;
        };
        state
            .borrow_mut()
            .installation_notice_act(index, sender.as_ref());
        crate::refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    notice.on_dismiss(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        presenter
            .borrow_mut()
            .dismiss_installation_notice(send.as_ref());
        crate::refresh(&window, &presenter.borrow());
        window.invoke_focus_files();
    });
}
