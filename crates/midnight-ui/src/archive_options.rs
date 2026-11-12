//! Archive association controls share backend policy; callbacks perform presentation only.
use crate::{ArchiveAssociationRow, ArchiveOptionsView, MainWindow, Presenter};
use midnight_core::{ARCHIVE_ASSOCIATIONS, ARCHIVE_FORMATS, ArchiveFormat, Command};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

fn selected_format(view: &ArchiveOptionsView<'_>) -> Option<ArchiveFormat> {
    usize::try_from(view.get_format_index())
        .ok()
        .and_then(|index| ARCHIVE_FORMATS.get(index))
        .copied()
}

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<ArchiveOptionsView>();
    if view.get_formats().row_count() != ARCHIVE_FORMATS.len() {
        view.set_formats(ModelRc::new(VecModel::from(
            ARCHIVE_FORMATS
                .iter()
                .map(|format| SharedString::from(format.label()))
                .collect::<Vec<_>>(),
        )));
    }
    let Some(format) = selected_format(&view) else {
        return;
    };
    let filter = view.get_filter().trim().to_lowercase();
    let rows: Vec<_> = ARCHIVE_ASSOCIATIONS
        .iter()
        .filter(|known| {
            known.format == format
                && (known.extension.contains(&filter)
                    || known.label.to_lowercase().contains(&filter))
        })
        .map(|known| ArchiveAssociationRow {
            extension: known.extension.into(),
            label: known.label.into(),
            enabled: presenter
                .preferences
                .values
                .archive_association_enabled(known.extension),
        })
        .collect();
    if view.get_rows().iter().collect::<Vec<_>>() != rows {
        view.set_rows(ModelRc::new(VecModel::from(rows)));
    }
    let custom: Vec<SharedString> = presenter
        .preferences
        .values
        .archive_custom_extensions
        .iter()
        .filter(|(_, assigned)| **assigned == format)
        .map(|(extension, _)| extension.into())
        .collect();
    if view.get_custom_extensions().iter().collect::<Vec<_>>() != custom {
        view.set_custom_extensions(ModelRc::new(VecModel::from(custom)));
    }
    view.set_ready(presenter.archive_options_ready());
    view.set_availability(if presenter.archive_format_available(format) {
        "Decoder available. OK saves changes; Cancel restores them.".into()
    } else {
        "Decoder unavailable. These assignments are retained; browsing needs a compatible archive provider.".into()
    });
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
    let state = presenter.clone();
    window
        .global::<ArchiveOptionsView>()
        .on_choose_format(move |index| {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= ARCHIVE_FORMATS.len())
            {
                return;
            }
            let Some(window) = weak.upgrade() else { return };
            let view = window.global::<ArchiveOptionsView>();
            view.set_format_index(index);
            view.set_filter("".into());
            view.set_error("".into());
            view.set_conflict_extension("".into());
            refresh(&window, &state.borrow());
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window
        .global::<ArchiveOptionsView>()
        .on_toggle(move |extension, enabled| {
            let Some(window) = weak.upgrade() else { return };
            state.borrow_mut().set_archive_association_enabled(
                extension.as_str(),
                enabled,
                sender.as_ref(),
            );
            crate::refresh(&window, &state.borrow());
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.global::<ArchiveOptionsView>().on_add(move || {
        let Some(window) = weak.upgrade() else { return };
        let view = window.global::<ArchiveOptionsView>();
        let Some(format) = selected_format(&view) else {
            return;
        };
        let draft = view.get_draft();
        let result =
            state
                .borrow_mut()
                .add_archive_extension(draft.as_str(), format, sender.as_ref());
        view.set_conflict_extension("".into());
        match result {
            Ok(()) => {
                view.set_draft("".into());
                view.set_error("".into());
            }
            Err(message) => {
                view.set_error(message.into());
                if let Ok(extension) = midnight_core::normalize_archive_extension(draft.as_str())
                    && ARCHIVE_ASSOCIATIONS
                        .iter()
                        .any(|known| known.extension == extension)
                {
                    view.set_conflict_extension(extension.into());
                }
            }
        }
        crate::refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ArchiveOptionsView>()
        .on_show_builtin(move || {
            let Some(window) = weak.upgrade() else { return };
            let view = window.global::<ArchiveOptionsView>();
            let extension = view.get_conflict_extension();
            let Some(known) = ARCHIVE_ASSOCIATIONS
                .iter()
                .find(|known| known.extension == extension.as_str())
            else {
                return;
            };
            if let Some(index) = ARCHIVE_FORMATS
                .iter()
                .position(|format| *format == known.format)
            {
                view.set_format_index(index as i32);
                view.set_filter(extension);
                view.set_error("".into());
                view.set_conflict_extension("".into());
                refresh(&window, &state.borrow());
            }
        });
    let weak = window.as_weak();
    window
        .global::<ArchiveOptionsView>()
        .on_remove(move |extension| {
            let Some(window) = weak.upgrade() else { return };
            let view = window.global::<ArchiveOptionsView>();
            let Some(format) = selected_format(&view) else {
                return;
            };
            presenter.borrow_mut().remove_archive_extension(
                extension.as_str(),
                format,
                send.as_ref(),
            );
            view.set_error("".into());
            view.set_conflict_extension("".into());
            crate::refresh(&window, &presenter.borrow());
        });
}
