//! Small Slint view models for the backend-supplied viewer catalog.
use crate::{MainWindow, Presenter, ViewerPluginRow, ViewerPluginsView};
use midnight_core::{Command, FilesystemPluginKind};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<ViewerPluginsView>();
    let state = &presenter.viewer_plugins;
    view.set_pending(state.pending || state.filesystems_pending || state.thumbnailers_pending);
    view.set_thumbnailer_error(state.thumbnailer_error.as_str().into());
    view.set_thumbnailer_selected(
        presenter
            .thumbnailer_plugin_selected()
            .map_or(-1, |index| index as i32),
    );
    view.set_filesystem_error(state.filesystem_error.as_str().into());
    view.set_filesystem_selected(
        presenter
            .filesystem_plugin_selected()
            .map_or(-1, |index| index as i32),
    );
    view.set_error(
        [state.error.as_str(), state.association_error.as_str()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    );
    view.set_settings_ready(presenter.viewer_plugin_settings_ready());
    let disabled = &presenter.preferences.values.disabled_viewer_plugins;
    let startup = state.startup_disabled.as_deref().unwrap_or_default();
    view.set_restart_needed(
        disabled.iter().any(|id| !startup.contains(id))
            || startup.iter().any(|id| !disabled.contains(id))
            || presenter.preferences.values.viewer_associations != state.startup_associations,
    );
    let catalog = presenter.viewer_plugins_ordered();
    let selected = presenter.viewer_plugin_selected(&catalog);
    view.set_selected(selected.map_or(-1, |index| index as i32));
    let current = selected.and_then(|index| catalog.get(index));
    let associations = &presenter.preferences.values.viewer_associations;
    let draft = state
        .extension_draft
        .as_ref()
        .filter(|draft| current.is_some_and(|plugin| plugin.id == draft.id));
    view.set_extensions(
        draft
            .map_or_else(
                || {
                    current.map_or_else(String::new, |plugin| {
                        associations
                            .overrides
                            .get(&plugin.id)
                            .unwrap_or(&plugin.extensions)
                            .join("; ")
                    })
                },
                |draft| draft.text.clone(),
            )
            .into(),
    );
    view.set_extension_override(
        draft.is_some()
            || state.extension_input_rejected
            || current.is_some_and(|plugin| associations.overrides.contains_key(&plugin.id)),
    );
    view.set_can_move_up(selected.is_some_and(|index| {
        !catalog[index].id.is_empty() && catalog[..index].iter().any(|plugin| !plugin.id.is_empty())
    }));
    view.set_can_move_down(selected.is_some_and(|index| {
        !catalog[index].id.is_empty()
            && catalog[index + 1..]
                .iter()
                .any(|plugin| !plugin.id.is_empty())
    }));
    let revision = format!(
        "{}:{}:{}",
        state.revision,
        presenter.preferences.revision.0,
        state.startup_disabled.is_some()
    );
    if view.get_revision().as_str() == revision {
        return;
    }
    let rows = catalog
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
                description: plugin.description.as_str().into(),
                kind: "".into(),
                version: plugin.version.as_str().into(),
                status: status.into(),
                location: plugin.location.to_string_lossy().into_owned().into(),
                enabled,
                configurable: !plugin.id.is_empty(),
                fallback: plugin.fallback,
            }
        })
        .collect::<Vec<_>>();
    view.set_rows(ModelRc::new(VecModel::from(rows)));
    let filesystems = state
        .filesystems
        .iter()
        .map(|plugin| ViewerPluginRow {
            id: plugin.id.as_str().into(),
            name: plugin.name.as_str().into(),
            version: plugin.version.as_str().into(),
            kind: match plugin.kind {
                Some(FilesystemPluginKind::Native) => "Native filesystem",
                Some(FilesystemPluginKind::Archive) => "Archive filesystem",
                Some(FilesystemPluginKind::Network) => "Network filesystem",
                None => "Package diagnostic",
            }
            .into(),
            status: plugin.status.as_str().into(),
            location: plugin
                .location
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    view.set_filesystem_rows(ModelRc::new(VecModel::from(filesystems)));
    let thumbnailers = state
        .thumbnailers
        .iter()
        .map(|plugin| ViewerPluginRow {
            id: plugin.id.as_str().into(),
            name: plugin.name.as_str().into(),
            version: plugin.version.as_str().into(),
            kind: if plugin.video {
                "Video previews · static and animated frames"
            } else {
                "Image previews"
            }
            .into(),
            description: format!("Extensions: {}", plugin.extensions.join("; ")).into(),
            status: plugin.status.as_str().into(),
            location: plugin.location.to_string_lossy().into_owned().into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    view.set_thumbnailer_rows(ModelRc::new(VecModel::from(thumbnailers)));
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
        let mut presenter = state.borrow_mut();
        if presenter.save_viewer_extension_draft(sender.as_ref()) {
            presenter.request_viewer_plugins(sender.as_ref());
        }
        drop(presenter);
        crate::refresh_properties(&window, &state.borrow(), false);
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window
        .global::<ViewerPluginsView>()
        .on_toggle(move |id, enabled| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            state
                .borrow_mut()
                .set_viewer_plugin_enabled(id.as_str(), enabled, sender.as_ref());
            crate::refresh_properties(&window, &state.borrow(), false);
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window
        .global::<ViewerPluginsView>()
        .on_select(move |index| {
            if let Ok(index) = usize::try_from(index) {
                change(&weak, &state, |presenter| {
                    presenter.select_viewer_plugin(index, sender.as_ref())
                });
            }
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ViewerPluginsView>()
        .on_select_filesystem(move |index| {
            if let Ok(index) = usize::try_from(index) {
                change(&weak, &state, |presenter| {
                    presenter.select_filesystem_plugin(index)
                });
            }
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ViewerPluginsView>()
        .on_select_thumbnailer(move |index| {
            if let Ok(index) = usize::try_from(index) {
                change(&weak, &state, |presenter| {
                    presenter.select_thumbnailer_plugin(index)
                });
            }
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    window
        .global::<ViewerPluginsView>()
        .on_edit_extensions(move |text| {
            change(&weak, &state, |presenter| {
                presenter.edit_viewer_extensions(text.as_str())
            });
        });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window
        .global::<ViewerPluginsView>()
        .on_reset_extensions(move || {
            change(&weak, &state, |presenter| {
                presenter.reset_viewer_extensions(sender.as_ref())
            });
        });
    let weak = window.as_weak();
    window.global::<ViewerPluginsView>().on_move(move |delta| {
        change(&weak, &presenter, |presenter| {
            presenter.move_viewer_plugin(delta, send.as_ref())
        });
    });
}

fn change(
    weak: &slint::Weak<MainWindow>,
    state: &Rc<RefCell<Presenter>>,
    apply: impl FnOnce(&mut Presenter),
) {
    let Some(window) = weak.upgrade() else {
        return;
    };
    apply(&mut state.borrow_mut());
    crate::refresh_properties(&window, &state.borrow(), false);
}
