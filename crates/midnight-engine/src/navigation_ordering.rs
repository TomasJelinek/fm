//! Prepare native navigation collation on discovery workers, never in UI comparators.
use midnight_core::{
    FsError, LocaleService, LocationKind, MAX_COLLATION_KEY_BYTES, NavigationLocation,
    SystemLocation, SystemLocationChoice, Volume, compare_collated_text,
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
        let key = prepare_key(label, path, locale, &mut used)?;
        prepared.push((item, key));
    }
    prepared.sort_by(|(left, left_key), (right, right_key)| {
        let (left_section, left_label, left_id, left_path) = fields(left);
        let (right_section, right_label, right_id, right_path) = fields(right);
        left_section
            .cmp(&right_section)
            .then_with(|| {
                label_order(
                    left_label,
                    left_key.as_deref(),
                    right_label,
                    right_key.as_deref(),
                )
            })
            .then_with(|| left_path.as_os_str().cmp(right_path.as_os_str()))
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(prepared.into_iter().map(|(item, _)| item).collect())
}

pub(crate) fn system_locations(
    locations: &[NavigationLocation],
    recycle_bin_available: bool,
    locale: Option<&dyn LocaleService>,
) -> Result<Vec<SystemLocationChoice>, FsError> {
    if !recycle_bin_available {
        return Ok(Vec::new());
    }
    // Fixed cached text is shared by both desktop platforms, never a path locator.
    let label = "Recycle Bin";
    let mut used = 0;
    let key = prepare_key(label, Path::new(""), locale, &mut used)?;
    let mut special_position = 0;
    for local in locations
        .iter()
        .filter(|local| local.kind != LocationKind::Volume)
    {
        let local_key = prepare_key(&local.label, &local.path, locale, &mut used)?;
        // Native entries with the same label precede this one finite typed entry.
        if !label_order(&local.label, local_key.as_deref(), label, key.as_deref()).is_gt() {
            special_position += 1;
        }
    }
    Ok(vec![SystemLocationChoice {
        location: SystemLocation::RecycleBin,
        label: label.into(),
        special_position,
    }])
}

fn label_order(
    left: &str,
    left_key: Option<&[u8]>,
    right: &str,
    right_key: Option<&[u8]>,
) -> std::cmp::Ordering {
    // Named labels precede raw mount-path labels within their section.
    left.starts_with('/')
        .cmp(&right.starts_with('/'))
        .then_with(|| {
            compare_collated_text(
                left_key,
                OsStr::new(left),
                right_key,
                OsStr::new(right),
                false,
            )
        })
        .then_with(|| OsStr::new(left).cmp(OsStr::new(right)))
}

fn prepare_key(
    label: &str,
    path: &Path,
    locale: Option<&dyn LocaleService>,
    used: &mut usize,
) -> Result<Option<Vec<u8>>, FsError> {
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
        *used = used.saturating_add(key.len());
        if key.len() > MAX_COLLATION_KEY_BYTES || *used > KEY_BUDGET {
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
    Ok(key)
}
