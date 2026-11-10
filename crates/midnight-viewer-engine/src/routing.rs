//! One host-side choice across native viewers and restricted image/document workers.
use midnight_core::ViewerAssociations;
use midnight_plugin_host::{PluginInfo, api::viewer};
use std::path::Path;

pub(crate) struct ProcessCandidate<'a> {
    pub id: &'a str,
    pub extensions: &'a [String],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Native {
        index: usize,
        mode: u16,
        detect_binary: bool,
    },
    Process {
        index: usize,
    },
}

pub(crate) fn choose<'a>(
    native: &'a [PluginInfo],
    process: impl IntoIterator<Item = ProcessCandidate<'a>>,
    path: &Path,
    basic: bool,
    disabled: &[String],
    associations: &ViewerAssociations,
) -> Option<Route> {
    // Native identities remain raw. An undecodable filename simply has no text suffix match.
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut best = None;
    for (index, info) in native.iter().enumerate() {
        if disabled.contains(&info.id) {
            continue;
        }
        let matched = (!basic)
            .then(|| associations.suffix_match(&info.id, &info.extensions, filename))
            .flatten();
        // User overrides alter association matches, never manufacture/remove the Basic role.
        let fallback = info.extensions.is_empty() && info.modes.contains(&viewer::MODE_TEXT);
        if matched.is_none() && !fallback {
            continue;
        }
        let mode = if matched.is_some() && info.modes.contains(&viewer::MODE_DOCUMENT) {
            viewer::MODE_DOCUMENT
        } else if matched.is_some() && info.modes.contains(&viewer::MODE_SOURCE) {
            viewer::MODE_SOURCE
        } else if info.modes.contains(&viewer::MODE_TEXT) {
            viewer::MODE_TEXT
        } else {
            continue;
        };
        consider(
            &mut best,
            (
                matched.unwrap_or(0),
                matched.map_or(0, |_| associations.priority(&info.id)),
                false,
                mode,
                info.priority,
            ),
            &info.id,
            Route::Native {
                index,
                mode,
                detect_binary: !basic
                    && fallback
                    && mode == viewer::MODE_TEXT
                    && info.modes.contains(&viewer::MODE_BINARY),
            },
        );
    }
    if !basic {
        for (index, provider) in process.into_iter().enumerate() {
            if disabled.iter().any(|id| id == provider.id) {
                continue;
            }
            if let Some(length) =
                associations.suffix_match(provider.id, provider.extensions, filename)
            {
                consider(
                    &mut best,
                    (length, associations.priority(provider.id), true, 0, 0),
                    provider.id,
                    Route::Process { index },
                );
            }
        }
    }
    best.map(|(_, _, route)| route)
}

// Specific suffix > user order > existing transport/mode/manifest defaults.
type Rank = (usize, usize, bool, u16, i32);

fn consider<'a>(best: &mut Option<(Rank, &'a str, Route)>, rank: Rank, id: &'a str, route: Route) {
    if best.as_ref().is_none_or(|(previous, previous_id, _)| {
        rank > *previous
            || rank == *previous
                && if rank.2 {
                    id < *previous_id
                } else {
                    id > *previous_id
                }
    }) {
        *best = Some((rank, id, route));
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
