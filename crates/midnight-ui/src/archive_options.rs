//! Archive association controls share the backend catalog and persist through preferences.
use crate::{ArchiveAssociationRow, ArchiveOptionsView, MainWindow, Presenter};
use midnight_core::{ARCHIVE_ASSOCIATIONS, Command};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<ArchiveOptionsView>();
    let filter = view.get_filter().trim().to_lowercase();
    let associations = ARCHIVE_ASSOCIATIONS.iter().filter(|association| {
        association.extension.contains(&filter)
            || association.label.to_lowercase().contains(&filter)
    });
    let current = view.get_rows();
    if current.row_count() == associations.clone().count()
        && associations
            .clone()
            .zip(current.iter())
            .all(|(association, row)| {
                row.extension.as_str() == association.extension
                    && row.enabled
                        == presenter
                            .preferences
                            .values
                            .archive_association_enabled(association.extension)
            })
    {
        return;
    }
    let rows: Vec<_> = associations
        .map(|association| ArchiveAssociationRow {
            extension: association.extension.into(),
            label: association.label.into(),
            enabled: presenter
                .preferences
                .values
                .archive_association_enabled(association.extension),
        })
        .collect();
    view.set_rows(ModelRc::new(VecModel::from(rows)));
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ArchiveOptionsView>()
        .on_search(move |filter| {
            let Some(window) = weak.upgrade() else { return };
            window.global::<ArchiveOptionsView>().set_filter(filter);
            refresh(&window, &state.borrow());
        });
    let weak = window.as_weak();
    window
        .global::<ArchiveOptionsView>()
        .on_toggle(move |extension, enabled| {
            let Some(window) = weak.upgrade() else { return };
            presenter.borrow_mut().set_archive_association_enabled(
                extension.as_str(),
                enabled,
                send.as_ref(),
            );
            // Rows must publish the changed archive badge as soon as this choice changes.
            crate::refresh(&window, &presenter.borrow());
        });
}
