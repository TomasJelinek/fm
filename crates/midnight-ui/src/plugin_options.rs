//! Small Slint view models for the backend-supplied viewer catalog.
use crate::{MainWindow, Presenter, ViewerPluginRow, ViewerPluginsView};
use midnight_core::Command;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<ViewerPluginsView>();
    let state = &presenter.viewer_plugins;
    view.set_pending(state.pending);
    view.set_error(state.error.as_str().into());
    view.set_settings_ready(presenter.viewer_plugin_settings_ready());
    let disabled = &presenter.preferences.values.disabled_viewer_plugins;
    let startup = state.startup_disabled.as_deref().unwrap_or_default();
    view.set_restart_needed(
        disabled.iter().any(|id| !startup.contains(id))
            || startup.iter().any(|id| !disabled.contains(id)),
    );
    let revision = format!(
        "{}:{}:{}",
        state.revision,
        presenter.preferences.revision.0,
        state.startup_disabled.is_some()
    );
    if view.get_revision().as_str() == revision {
        return;
    }
    let mut rows = state
        .catalog
        .iter()
        .map(|plugin| {
            let enabled = !plugin.id.is_empty() && !disabled.contains(&plugin.id);
            let mut status = plugin.status.clone();
            if !plugin.id.is_empty() && state.startup_disabled.is_some() {
                let current = !startup.contains(&plugin.id);
                if current != enabled {
                    status.push_str(if enabled {
                        " · enabled after restart"
                    } else {
                        " · disabled after restart"
                    });
                } else if !current {
                    status.push_str(" · disabled this session");
                }
            }
            ViewerPluginRow {
                id: plugin.id.as_str().into(),
                name: plugin.name.as_str().into(),
                version: plugin.version.as_str().into(),
                status: status.into(),
                location: plugin.location.to_string_lossy().into_owned().into(),
                enabled,
                configurable: !plugin.id.is_empty(),
            }
        })
        .collect::<Vec<_>>();
    for id in disabled
        .iter()
        .filter(|id| !state.catalog.iter().any(|plugin| &plugin.id == *id))
    {
        rows.push(ViewerPluginRow {
            id: id.as_str().into(),
            name: id.as_str().into(),
            status: "Not installed · disabled preference retained".into(),
            configurable: true,
            ..Default::default()
        });
    }
    view.set_rows(ModelRc::new(VecModel::from(rows)));
    view.set_revision(revision.into());
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.global::<ViewerPluginsView>().on_refresh(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        state.borrow_mut().request_viewer_plugins(sender.as_ref());
        crate::refresh_properties(&window, &state.borrow(), false);
    });
    let weak = window.as_weak();
    window
        .global::<ViewerPluginsView>()
        .on_toggle(move |id, enabled| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            presenter
                .borrow_mut()
                .set_viewer_plugin_enabled(id.as_str(), enabled, send.as_ref());
            crate::refresh_properties(&window, &presenter.borrow(), false);
        });
}
