//! Tiles of files that carry their own artwork, such as programs: metadata-only
//! views for the backend and admission of the artwork it returns into the thumbnail
//! image cache, where it is retained and evicted like any other preview.
use super::{ThumbnailState, ThumbnailTarget};
use crate::{Presenter, ThumbnailContent};
use midnight_core::{Command, FileIconTarget, IconPixels, MAX_ICON_EDGE};
use std::ffi::OsStr;

#[cfg(windows)]
#[path = "file_icons_windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "file_icons_unix.rs"]
mod platform;

/// Whether files of `extension` show their own artwork in tiles instead of their
/// type icon. The native provider reads it (`IconService::load_file_icon`).
pub(crate) fn carries_own_artwork(extension: Option<&OsStr>) -> bool {
    extension.is_some_and(platform::carries_own_artwork)
}

/// What was last offered to the backend.
#[derive(Default)]
pub(super) struct State {
    generation: u64,
    /// The visible tiles the last view was taken from, with their device edges.
    visible: Vec<(String, u16)>,
    /// The view the backend accepted last.
    sent: Vec<FileIconTarget>,
}

/// A tile of `logical` pixels in device pixels, as the window draws it.
fn device_edge(logical: u32, scale: f32) -> u16 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (logical as f32 * scale)
        .round()
        .clamp(1.0, f32::from(MAX_ICON_EDGE)) as u16
}

impl ThumbnailState {
    pub(super) fn forget_file_icons(&mut self) {
        // A cleared cache has to ask again for every visible tile.
        self.file_icons.visible.clear();
    }
}

impl Presenter {
    /// Offers the backend the visible tiles of files with their own artwork that
    /// still lack it at their device edge. A new view is sent only when the visible
    /// tiles or their edges change, so artwork arriving for the current view does not
    /// cancel the load in progress; scrolling does. Called by the thumbnail pulse
    /// with the targets it took from the window.
    pub fn refresh_file_icons(
        &mut self,
        rows: &[ThumbnailTarget],
        scale: f32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let visible: Vec<(String, u16)> = rows
            .iter()
            .filter(|row| matches!(row.content, ThumbnailContent::FileIcon(_)))
            .map(|row| (row.identity.clone(), device_edge(row.edge, scale)))
            .collect();
        // A refused view leaves `visible` as it was, so the next pass offers it again.
        if visible == self.thumbnails.file_icons.visible {
            return;
        }
        let targets: Vec<FileIconTarget> = rows
            .iter()
            .filter_map(|row| {
                let ThumbnailContent::FileIcon(path) = &row.content else {
                    return None;
                };
                let edge = device_edge(row.edge, scale);
                (self.thumbnail_coverage(&row.identity, 0) < u32::from(edge)).then(|| {
                    FileIconTarget {
                        identity: row.identity.clone(),
                        path: path.clone(),
                        edge,
                    }
                })
            })
            .collect();
        let state = &mut self.thumbnails.file_icons;
        // Nothing to load, and nothing was asked for that is still loading.
        if targets.is_empty() && state.sent.is_empty() {
            state.visible = visible;
            return;
        }
        let Some(generation) = state.generation.checked_add(1) else {
            return;
        };
        if send(Command::SetFileIconView {
            generation,
            targets: targets.clone(),
        }) {
            state.generation = generation;
            state.sent = targets;
            state.visible = visible;
        }
    }

    /// Admits artwork of the current view for a tile that is still visible. Returns
    /// whether the tile changed.
    pub(crate) fn receive_file_icon(
        &mut self,
        generation: u64,
        identity: &str,
        edge: u16,
        pixels: &IconPixels,
    ) -> bool {
        let state = &self.thumbnails.file_icons;
        if state.generation != generation
            || !state
                .sent
                .iter()
                .any(|target| target.identity == identity && target.edge == edge)
        {
            return false;
        }
        self.receive_thumbnail_at_resolution(
            identity,
            0,
            pixels.width,
            pixels.height,
            u32::from(edge),
            &pixels.rgba,
        )
    }
}

#[cfg(test)]
#[path = "file_icons_tests.rs"]
mod tests;
