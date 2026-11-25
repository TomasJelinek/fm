//! Pure presentation of cached native metadata; hover never triggers I/O.
use super::*;
use std::path::Path;

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
    details_text(details, locale, None)
}

/// Free space, after the volume's name when it has one, line by line. A name equal to
/// `shown`, which the control already shows, is left out.
fn details_text(
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
    shown: Option<&str>,
) -> String {
    let Some(details) = details else {
        return "Loading free space…".into();
    };
    let capacity = match &details.space {
        Ok(space) => format!("{} free", format_size(space.available, locale)),
        Err(_) => "Free space unavailable".into(),
    };
    match &details.label {
        Ok(Some(label))
            if !label.is_empty() && shown.is_none_or(|shown| label.to_string_lossy() != shown) =>
        {
            format!("{}\n{capacity}", label.to_string_lossy())
        }
        Err(_) => format!("Volume name unavailable\n{capacity}"),
        _ => capacity,
    }
}

/// Whether a drive's `label` is a name of its own rather than its root's: Linux names a
/// mount by its volume label or its folder (`USB` for `/media/<user>/USB`). A label that
/// the root's text starts with names the root itself: a Windows drive letter (`C:` for
/// `C:\`) or the Linux root `/`.
pub(super) fn is_drive_name(label: &str, root: &Path) -> bool {
    !root.to_string_lossy().starts_with(label)
}

/// A drive's name spelled out: its volume name when `label` is that name shortened (its
/// start and end around `…`, as the backend shortens long names), else `label` itself.
/// The drive bar keeps the short name; tooltips and the drive menu have the room.
fn spelled_out(label: &str, details: Option<&VolumeDetails>) -> String {
    if let Some(Ok(Some(name))) = details.map(|details| &details.label) {
        let name = name.to_string_lossy();
        if shortens(label, &name) {
            return name.into_owned();
        }
    }
    label.to_owned()
}

/// Whether `short` is `full` with its middle replaced by one `…`.
fn shortens(short: &str, full: &str) -> bool {
    short.split_once('…').is_some_and(|(head, tail)| {
        !tail.contains('…')
            && full != short
            && full.len() >= head.len() + tail.len()
            && full.starts_with(head)
            && full.ends_with(tail)
    })
}

/// A drive's tooltip on the drive bar and in the drive menu: its whole path and details,
/// after its name, spelled out ([`spelled_out`]), when that is a name of its own
/// ([`is_drive_name`]).
pub(super) fn drive_tooltip(
    label: &str,
    root: &Path,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    if is_drive_name(label, root) {
        let name = spelled_out(label, details);
        format!(
            "{name}\n{}\n{}",
            root.display(),
            details_text(details, locale, Some(&name))
        )
    } else {
        format!("{}\n{}", root.display(), tooltip(details, locale))
    }
}

/// A drive's row in the drive menu. A name of its own ([`is_drive_name`]), spelled out
/// ([`spelled_out`]), stands for the path, which the row's tooltip and accessible
/// description give; a root's own name is followed by the root.
pub(super) fn drive_menu_label(
    label: &str,
    root: &Path,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    if is_drive_name(label, root) {
        let name = spelled_out(label, details);
        format!(
            "{name} — {}",
            details_text(details, locale, Some(&name)).replace('\n', " — ")
        )
    } else {
        root_menu_label(label, root, details, locale)
    }
}

pub(super) fn menu_label(
    volume: &Volume,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    root_menu_label(&volume.label, &volume.root, details, locale)
}

fn root_menu_label(
    label: &str,
    root: &Path,
    details: Option<&VolumeDetails>,
    locale: &NumberLocale,
) -> String {
    let root = root.to_string_lossy();
    let title = if label == root {
        label.to_owned()
    } else {
        format!("{label} ({root})")
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
            crate::error_presentation::path_text(&volume.root),
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
            format!("{label}\n1.0 GiB free")
        );
        assert_eq!(
            menu_label(&volume, Some(&info), &NumberLocale::default()),
            format!("Q: — {label} — 1.0 GiB free")
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
    fn a_name_is_spelled_out_only_from_the_name_it_shortens() {
        assert!(shortens("Document…eserver", "Documents on fileserver"));
        assert!(shortens("文档文档…档文档", "文档文档文档文档文档"));
        for (short, full) in [
            ("Document…eserver", "Documents on nas"),
            ("Document…eserver", "My Documents on fileserver"),
            ("USB1", "USB"),
            ("a…b", "a…b"),
            ("ab…ba", "aba"),
            ("a…b…c", "a-x-b-y-c"),
        ] {
            assert!(!shortens(short, full), "{short} / {full}");
        }
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
