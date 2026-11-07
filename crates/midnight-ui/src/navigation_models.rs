//! Cached native navigation targets become display-only choice rows.
use crate::*;

/// Metadata changes must preserve delegates holding a pointer gesture. Replace
/// the model only when a target identity, its position, or the count changes.
pub(super) fn update<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    values: Vec<T>,
    key: impl Fn(&T) -> slint::SharedString,
    set: impl FnOnce(ModelRc<T>),
) {
    let same_targets = current.row_count() == values.len()
        && values.iter().enumerate().all(|(index, value)| {
            let identity = key(value);
            !identity.is_empty()
                && current
                    .row_data(index)
                    .is_some_and(|old| key(&old) == identity)
        });
    if same_targets && let Some(model) = current.as_any().downcast_ref::<VecModel<T>>() {
        for (index, value) in values.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&value) {
                model.set_row_data(index, value);
            }
        }
    } else {
        set(Rc::new(VecModel::from(values)).into());
    }
}

pub(super) fn choices(presenter: &Presenter) -> Vec<NavigationChoiceRow> {
    presenter
        .navigation_choices()
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let location = &choice.location;
            let volume = presenter
                .volumes
                .iter()
                .find(|volume| volume.id == location.id && volume.root == location.path);
            let details = volume.and_then(|volume| volume_presentation::details(presenter, volume));
            let is_drive = location.kind == LocationKind::Volume;
            let label = if is_drive {
                volume
                    .map(|volume| {
                        volume_presentation::menu_label(volume, details, &presenter.number_locale)
                    })
                    .unwrap_or_else(|| format!("{} — {}", location.label, location.path.display()))
            } else {
                format!("{} — {}", location.label, location.path.display())
            };
            let tooltip = if is_drive {
                format!(
                    "{}\n{}",
                    location.path.display(),
                    volume_presentation::tooltip(details, &presenter.number_locale)
                )
            } else {
                format!("{}\n{}", location.label, location.path.display())
            };
            let compact = if is_drive {
                location
                    .drive_letter
                    .map(|letter| format!("{letter}:"))
                    .unwrap_or_else(|| location.label.clone())
            } else {
                location.label.clone()
            };
            NavigationChoiceRow {
                identity: presenter.navigation_choice_token(index).into(),
                label: label.into(),
                compact_label: compact.into(),
                tooltip: tooltip.into(),
                icon: presenter.desktop.image(&IconKey::for_location(location)),
                kind: match location.kind {
                    LocationKind::Volume => LocationRole::Drive,
                    LocationKind::Home => LocationRole::Home,
                    LocationKind::Desktop => LocationRole::Desktop,
                    LocationKind::Documents => LocationRole::Documents,
                    LocationKind::Downloads => LocationRole::Downloads,
                    LocationKind::Pictures
                    | LocationKind::Videos
                    | LocationKind::Music
                    | LocationKind::Temporary
                    | LocationKind::OneDrive => LocationRole::Folder,
                },
            }
        })
        .collect()
}

/// Keep every account in the menu, with only the primary OneDrive shortcut on
/// the compact toolbar. A second identical cloud icon has no readable identity.
pub(super) fn toolbar_choices(
    presenter: &Presenter,
    choices: &[NavigationChoiceRow],
) -> Vec<NavigationChoiceRow> {
    let mut cloud_seen = false;
    presenter
        .navigation_choices()
        .iter()
        .zip(choices)
        .filter_map(|(native, row)| {
            if native.location.kind == LocationKind::OneDrive {
                if cloud_seen {
                    return None;
                }
                cloud_seen = true;
            }
            Some(row.clone())
        })
        .collect()
}

pub(super) fn locations(
    choices: &[NavigationChoiceRow],
    selected: Option<&str>,
) -> Vec<LocationRow> {
    choices
        .iter()
        .map(|choice| LocationRow {
            identity: choice.identity.clone(),
            kind: choice.kind,
            label: choice.label.clone(),
            compact_label: choice.compact_label.clone(),
            is_drive: choice.kind == LocationRole::Drive,
            path: "".into(),
            tooltip: choice.tooltip.clone(),
            icon: choice.icon.clone(),
            selected: selected == Some(choice.identity.as_str()),
        })
        .collect()
}
