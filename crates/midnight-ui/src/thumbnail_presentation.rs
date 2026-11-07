//! Visible identities and bounded Slint image handles. No source access or decoding occurs here.
use crate::{Presenter, presenter::Pane};
use midnight_core::{Entry, EntryKind, PaneId, PanelViewMode, StorageKind};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailTarget {
    pub identity: String,
    pub path: PathBuf,
    pub selected: bool,
    pub hovered: bool,
    pub confirmed_ssd: bool,
}
#[derive(Default)]
pub(crate) struct ThumbnailState {
    visible: Vec<ThumbnailTarget>,
    images: HashMap<String, Vec<Option<Image>>>,
    pulse: usize,
}
pub(crate) fn identity(pane: &Pane, entry: &Entry, side: PaneId) -> String {
    format!(
        "{}:{}:{}:{}:entry:{}",
        side.index(),
        pane.generation,
        pane.displayed_generation().0,
        pane.search.view_revision,
        entry.id.0
    )
}
impl Presenter {
    pub fn report_thumbnail_error(&mut self, error: String) {
        self.desktop.report("Previews", error);
    }
    pub fn clear_thumbnail_images(&mut self) {
        self.thumbnails.images.clear();
    }
    pub fn thumbnail_targets(
        &mut self,
        ranges: [(usize, usize); 2],
        hovered: [&str; 2],
    ) -> Vec<ThumbnailTarget> {
        let mut targets = Vec::new();
        if !self.preferences.values.thumbnails_enabled {
            self.thumbnails.images.clear();
            self.thumbnails.visible.clear();
            return targets;
        }
        for side in [PaneId::Left, PaneId::Right] {
            let slot = side.index();
            if self.preferences.values.panel_view_modes[slot] != PanelViewMode::Thumbnails {
                continue;
            }
            let pane = &self.panes[slot];
            if !pane.native_capabilities() {
                continue;
            }
            // Classification can arrive after the listing. Unknown must not start a read
            // before a delayed network classification when network previews are disabled.
            if matches!(
                pane.visible_storage_kind(),
                StorageKind::Network | StorageKind::Unknown
            ) && !self.preferences.values.thumbnails_network
            {
                continue;
            }
            let (first, end) = ranges[slot];
            for row in first..end.min(first.saturating_add(128)) {
                let Some(index) = row.checked_sub(pane.parent_offset()) else {
                    continue;
                };
                let Some(entry) = pane.display_entry(index) else {
                    continue;
                };
                if entry.kind != EntryKind::File {
                    continue;
                }
                let Some(path) = entry.native_path() else {
                    continue;
                };
                let key = identity(pane, entry, side);
                // Slint's existing row identity lacks pane prefix; compare it without the prefix.
                let row_identity = key.split_once(':').map_or("", |(_, value)| value);
                targets.push(ThumbnailTarget {
                    hovered: row_identity == hovered[slot],
                    identity: key,
                    path: path.to_owned(),
                    selected: pane.selection_visible(entry),
                    confirmed_ssd: pane.visible_storage_kind() == StorageKind::LocalSsd,
                });
            }
        }
        self.thumbnails
            .images
            .retain(|identity, _| targets.iter().any(|target| &target.identity == identity));
        self.thumbnails.visible = targets.clone();
        targets
    }
    pub fn receive_thumbnail(
        &mut self,
        identity: &str,
        frame: u8,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> bool {
        if frame >= 6
            || width == 0
            || height == 0
            || width > 96
            || height > 96
            || rgba.len() != width as usize * height as usize * 4
            || !self
                .thumbnails
                .visible
                .iter()
                .any(|target| target.identity == identity)
        {
            return false;
        }
        let frames = self
            .thumbnails
            .images
            .entry(identity.to_owned())
            .or_insert_with(|| vec![None; 6]);
        frames[usize::from(frame)] = Some(Image::from_rgba8(
            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, width, height),
        ));
        true
    }
    pub fn advance_thumbnail_animation(&mut self) -> bool {
        self.thumbnails.pulse = self.thumbnails.pulse.wrapping_add(1);
        self.thumbnails.visible.iter().any(|target| {
            (target.selected || target.hovered)
                && self
                    .thumbnails
                    .images
                    .get(&target.identity)
                    .is_some_and(|frames| frames.iter().flatten().count() > 1)
        })
    }
    pub(crate) fn thumbnail_image(&self, side: PaneId, entry: &Entry) -> Option<Image> {
        if self.preferences.values.panel_view_modes[side.index()] != PanelViewMode::Thumbnails {
            return None;
        }
        let key = identity(&self.panes[side.index()], entry, side);
        let target = self
            .thumbnails
            .visible
            .iter()
            .find(|target| target.identity == key)?;
        let frames = self.thumbnails.images.get(&key)?;
        if target.hovered || target.selected {
            let count = frames.iter().flatten().count();
            if count > 0 {
                return frames
                    .iter()
                    .flatten()
                    .nth(self.thumbnails.pulse % count)
                    .cloned();
            }
        }
        frames.first()?.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn presenter() -> Presenter {
        let mut presenter = Presenter::new(PathBuf::new(), PathBuf::new());
        presenter.preferences.values.panel_view_modes = [PanelViewMode::Thumbnails; 2];
        presenter
    }
    fn target(key: String, selected: bool) -> ThumbnailTarget {
        ThumbnailTarget {
            identity: key,
            path: PathBuf::from("sample.avi"),
            selected,
            hovered: false,
            confirmed_ssd: false,
        }
    }
    #[test]
    fn stale_offscreen_and_oversized_pixels_never_enter_presentation() {
        let mut p = presenter();
        p.thumbnails.visible.push(target("current".into(), false));
        assert!(!p.receive_thumbnail("old-generation", 0, 1, 1, &[0; 4]));
        assert!(!p.receive_thumbnail("current", 6, 1, 1, &[0; 4]));
        assert!(!p.receive_thumbnail("current", 0, 97, 1, &[0; 388]));
        assert!(!p.receive_thumbnail("current", 0, 1, 1, &[0; 3]));
        assert!(p.receive_thumbnail("current", 0, 1, 1, &[0; 4]));
        p.thumbnail_targets([(0, 0); 2], ["", ""]);
        assert!(p.thumbnails.images.is_empty());
        assert!(!p.receive_thumbnail("current", 1, 1, 1, &[0; 4]));
    }
    #[test]
    fn every_visible_selected_video_animates_and_ordinary_video_stays_static() {
        let mut p = presenter();
        let entries: Vec<_> = (0..3)
            .map(|id| {
                Entry::new(
                    midnight_core::EntryId(id),
                    PathBuf::from(format!("{id}.avi")),
                    EntryKind::File,
                )
            })
            .collect();
        for (index, entry) in entries.iter().enumerate() {
            let key = identity(&p.panes[0], entry, PaneId::Left);
            p.thumbnails.visible.push(target(key.clone(), index < 2));
            assert!(p.receive_thumbnail(&key, 0, 1, 1, &[0; 4]));
            assert!(p.receive_thumbnail(&key, 1, 2, 1, &[0; 8]));
        }
        assert!(p.advance_thumbnail_animation());
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(
                p.thumbnail_image(PaneId::Left, entry).unwrap().size().width,
                if index < 2 { 2 } else { 1 }
            );
        }
        p.clear_thumbnail_images();
        assert!(
            entries
                .iter()
                .all(|entry| p.thumbnail_image(PaneId::Left, entry).is_none())
        );
    }
}
