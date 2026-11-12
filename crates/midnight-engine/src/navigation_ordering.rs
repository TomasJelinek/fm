//! Prepare native navigation collation on discovery workers, never in UI comparators.
use midnight_core::{
    FsError, LocaleService, LocationKind, MAX_COLLATION_KEY_BYTES, NavigationLocation, Volume,
    compare_collated_text,
};
use std::{collections::HashSet, ffi::OsStr, io, path::Path};

const KEY_BUDGET: usize = 16 * 1024 * 1024;

#[cfg(test)]
#[path = "navigation_ordering_tests.rs"]
mod tests;

pub(crate) fn volumes(
    volumes: Vec<Volume>,
    locale: Option<&dyn LocaleService>,
) -> Result<Vec<Volume>, FsError> {
    order(volumes, locale, |volume| {
        (false, &volume.label, &volume.id, &volume.root)
    })
}

pub(crate) fn locations(
    locations: Vec<NavigationLocation>,
    locale: Option<&dyn LocaleService>,
) -> Result<(Vec<NavigationLocation>, usize), FsError> {
    let mut locations = order(locations, locale, |location| {
        (
            location.kind != LocationKind::Volume,
            &location.label,
            &location.id,
            &location.path,
        )
    })?;
    let mut seen = HashSet::new();
    locations.retain(|location| {
        seen.insert((location.kind, location.id.clone(), location.path.clone()))
    });
    // Network is a virtual section boundary after drives, before every local
    // special folder. Its label never participates in native location sorting.
    Ok((locations, 0))
}

fn order<T>(
    items: Vec<T>,
    locale: Option<&dyn LocaleService>,
    fields: impl Fn(&T) -> (bool, &str, &str, &Path),
) -> Result<Vec<T>, FsError> {
    let mut used = 0usize;
    let mut prepared = Vec::with_capacity(items.len());
    for item in items {
        let (_, label, _, path) = fields(&item);
        let key = locale
            .map(|locale| locale.collation_key(OsStr::new(label)))
            .transpose()
            .map_err(|mut error| {
                if error.path.as_os_str().is_empty() {
                    error.path = path.to_path_buf();
                }
                error
            })?
            .flatten();
        if let Some(key) = &key {
            used = used.saturating_add(key.len());
            if key.len() > MAX_COLLATION_KEY_BYTES || used > KEY_BUDGET {
                return Err(FsError::from_io(
                    "order navigation locations",
                    path,
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Native navigation collation keys exceed their resource budget.",
                    ),
                ));
            }
        }
        prepared.push((item, key));
    }
    prepared.sort_by(|(left, left_key), (right, right_key)| {
        let (left_section, left_label, left_id, left_path) = fields(left);
        let (right_section, right_label, right_id, right_path) = fields(right);
        left_section
            .cmp(&right_section)
            // Named volumes precede raw mount-path labels within their section.
            // This is presentation semantics, independent of the host path parser.
            .then_with(|| {
                left_label
                    .starts_with('/')
                    .cmp(&right_label.starts_with('/'))
            })
            .then_with(|| {
                compare_collated_text(
                    left_key.as_deref(),
                    OsStr::new(left_label),
                    right_key.as_deref(),
                    OsStr::new(right_label),
                    false,
                )
            })
            .then_with(|| OsStr::new(left_label).cmp(OsStr::new(right_label)))
            .then_with(|| left_path.as_os_str().cmp(right_path.as_os_str()))
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(prepared.into_iter().map(|(item, _)| item).collect())
}
