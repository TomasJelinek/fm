//! Pure presentation of cached native metadata; hover never triggers I/O.
use super::*;

pub(super) fn panel_space(presenter: &Presenter, pane: PaneId) -> String {
    if let Some(space) = presenter.panes[pane.index()].visible_space() {
        return format!(
            "{} free",
            format_size(space.available, &presenter.number_locale)
        );
    }
    // Lexical prefixes cannot identify a filesystem across mounts or junctions.
    // Only a result tied to this pane's current navigation can report its capacity.
    "Free space —".into()
}

pub(super) fn details<'a>(presenter: &'a Presenter, volume: &Volume) -> Option<&'a VolumeDetails> {
    presenter
        .volume_details
        .get(&volume.id)
        .filter(|(root, _)| root == &volume.root)
        .map(|(_, details)| details)
}

pub(super) fn tooltip(details: Option<&VolumeDetails>, locale: &NumberLocale) -> String {
    let Some(details) = details else {
        return "Loading free space…".into();
    };
    let capacity = match &details.space {
        Ok(space) => format!("{} free", format_size(space.available, locale)),
        Err(_) => "Free space unavailable".into(),
    };
    match &details.label {
        Ok(Some(label)) if !label.is_empty() => format!("{}\n{capacity}", label.to_string_lossy()),
        Err(_) => format!("Volume name unavailable\n{capacity}"),
        _ => capacity,
    }
}

pub(super) fn menu_label(
    volume: &Volume,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    let root = volume.root.to_string_lossy();
    let title = if volume.label == root {
        volume.label.clone()
    } else {
        format!("{} ({root})", volume.label)
    };
    format!(
        "{title} — {}",
        tooltip(details, locale).replace('\n', " — ")
    )
}

pub(super) fn volume_tooltip(
    volume: &Volume,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    if volume.label == volume.root.to_string_lossy() {
        tooltip(details, locale)
    } else {
        format!(
            "{}\n{}\n{}",
            volume.label,
            volume.root.display(),
            tooltip(details, locale)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, io, path::Path};

    #[test]
    fn panel_capacity_never_guesses_a_filesystem_from_its_lexical_path_prefix() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.join("junction-to-another-volume"), root.clone());
        p.volumes.push(Volume {
            id: "root".into(),
            label: "Root".into(),
            root: root.clone(),
        });
        p.volume_details.insert(
            "root".into(),
            (
                root.clone(),
                VolumeDetails {
                    label: Ok(None),
                    space: Ok(SpaceInfo {
                        available: 1 << 30,
                        total: 2 << 30,
                    }),
                },
            ),
        );
        assert_eq!(panel_space(&p, PaneId::Left), "Free space —");
        p.panes[0].space = Some(SpaceInfo {
            available: 123,
            total: 999,
        });
        assert_eq!(panel_space(&p, PaneId::Left), "123 B free");
        assert!(p.navigate(PaneId::Left, root.join("another-volume"), None, &|_| true));
        assert_eq!(panel_space(&p, PaneId::Left), "Free space —");
        p.apply(Event::SpaceUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(0),
            space: SpaceInfo {
                available: 999,
                total: 999,
            },
        });
        assert_eq!(panel_space(&p, PaneId::Left), "Free space —");
        let navigation = NavigationId(p.panes[0].generation);
        p.apply(Event::SpaceUpdated {
            pane: PaneId::Left,
            navigation,
            space: SpaceInfo {
                available: 456,
                total: 999,
            },
        });
        assert_eq!(panel_space(&p, PaneId::Left), "456 B free");
    }

    #[test]
    fn native_name_and_space_are_literal_unicode_without_affecting_the_root() {
        let label = "文档 <b> & [disk](file:///x) Žluťoučký 👩🏽‍💻";
        let volume = Volume {
            id: "v".into(),
            label: "Q:".into(),
            root: "Q:".into(),
        };
        let info = VolumeDetails {
            label: Ok(Some(OsString::from(label))),
            space: Ok(SpaceInfo {
                available: 1 << 30,
                total: 2 << 30,
            }),
        };
        assert_eq!(
            tooltip(Some(&info), &NumberLocale::default()),
            format!("{label}\n1.0 GB free")
        );
        assert_eq!(
            menu_label(&volume, Some(&info), &NumberLocale::default()),
            format!("Q: — {label} — 1.0 GB free")
        );
        assert_eq!(volume.root, Path::new("Q:"));
    }

    #[test]
    fn unlabeled_platforms_show_capacity_and_missing_data_never_fakes_zero() {
        let mut info = VolumeDetails {
            label: Ok(None),
            space: Ok(SpaceInfo {
                available: 0,
                total: 100,
            }),
        };
        assert_eq!(tooltip(Some(&info), &NumberLocale::default()), "0 B free");
        info.space = Err(FsError::from_io(
            "space",
            Path::new("raw"),
            io::Error::other("offline"),
        ));
        assert_eq!(
            tooltip(Some(&info), &NumberLocale::default()),
            "Free space unavailable"
        );
        assert_eq!(
            tooltip(None, &NumberLocale::default()),
            "Loading free space…"
        );
        info.label = Err(FsError::from_io(
            "label",
            Path::new("raw"),
            io::Error::other("offline"),
        ));
        assert_eq!(
            tooltip(Some(&info), &NumberLocale::default()),
            "Volume name unavailable\nFree space unavailable"
        );
    }

    #[test]
    fn special_alias_choices_include_their_actual_native_path() {
        let volume = Volume {
            id: "home".into(),
            label: "Home".into(),
            root: "/users/文档".into(),
        };
        assert_eq!(
            menu_label(&volume, None, &NumberLocale::default()),
            "Home (/users/文档) — Loading free space…"
        );
        assert_eq!(
            volume_tooltip(&volume, None, &NumberLocale::default()),
            "Home\n/users/文档\nLoading free space…"
        );
    }
}
