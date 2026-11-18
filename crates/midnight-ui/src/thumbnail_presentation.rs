//! Visible identities and bounded Slint image handles. No source access or decoding occurs here.
use crate::{Presenter, ThumbnailContent, presenter::Pane};
use midnight_core::{Entry, EntryKind, PaneId, PanelViewMode, StorageKind};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
#[path = "thumbnail_image_cache.rs"]
mod cache;

/// Targets per thumbnail request; the thumbnail host refuses larger views.
pub(crate) const MAX_THUMBNAIL_TARGETS: usize = midnight_core::MAX_THUMBNAIL_VIEW_TARGETS;

/// Split the request budget between the panes' visible candidates. Each pane is
/// guaranteed half; a pane that needs less leaves the rest to the other, so a
/// single large thumbnail grid can use the whole budget.
fn split_quota(budget: usize, wanted: [usize; 2]) -> [usize; 2] {
    let fair = budget / 2;
    let left = wanted[0].min(fair.max(budget.saturating_sub(wanted[1])));
    [left, wanted[1].min(budget - left)]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailTarget {
    pub identity: String,
    pub content: ThumbnailContent,
    pub selected: bool,
    pub hovered: bool,
    pub confirmed_ssd: bool,
    /// Desired panel edge in logical pixels; command artwork uses its fixed physical edge.
    pub edge: u32,
}
#[derive(Default)]
pub(crate) struct ThumbnailState {
    visible: Vec<ThumbnailTarget>,
    images: HashMap<String, Vec<Option<Image>>>,
    coverage: HashMap<String, [u32; 6]>,
    archive_network_policy: HashMap<String, bool>,
    /// Last use per identity and the same ticks in order: eviction walks the
    /// oldest entries instead of sorting every retained key per upload.
    used: HashMap<String, u64>,
    order: std::collections::BTreeMap<u64, String>,
    bytes: usize,
    tick: u64,
    pulse: usize,
}
pub(crate) fn identity(pane: &Pane, entry: &Entry, side: PaneId) -> String {
    if pane.tree_view().is_some() {
        format!("{}:{}", side.index(), pane.row_identity(entry))
    } else {
        // Pixels belong to the displayed listing, which stays in place while a
        // refresh is pending. Input identities still include the pending intent.
        format!(
            "{}:{}:entry:{}",
            side.index(),
            pane.displayed_generation().0,
            entry.id.0
        )
    }
}
impl Presenter {
    fn panel_thumbnails_allowed(&self, side: PaneId) -> bool {
        let pane = &self.panes[side.index()];
        self.preferences.values.thumbnails_enabled
            && self.preferences.values.panel_view_modes[side.index()] == PanelViewMode::Thumbnails
            && ((self.preferences.values.thumbnails_archives
                && pane
                    .thumbnail_archive_location()
                    .is_some_and(|location| location.mount.is_some()))
                || (pane.native_capabilities()
                    && (self.preferences.values.thumbnails_network
                        || !matches!(
                            pane.visible_storage_kind(),
                            StorageKind::Network | StorageKind::Unknown
                        ))))
    }
    pub fn report_thumbnail_error(&mut self, error: String) {
        self.desktop.report("Previews", error);
    }
    pub fn clear_thumbnail_images(&mut self) {
        self.thumbnails.clear();
        self.command_toolbar.native.borrow_mut().invalidate();
        self.command_toolbar
            .image_revision
            .set(self.command_toolbar.image_revision.get().wrapping_add(1));
    }
    pub fn thumbnail_targets(
        &mut self,
        ranges: [(usize, usize); 2],
        hovered: [&str; 2],
    ) -> Vec<ThumbnailTarget> {
        for (pane, range) in self.panes.iter_mut().zip(ranges) {
            pane.set_tree_viewport(range);
        }
        // Explicitly configured command artwork uses the same isolated decoder
        // and disk cache, independently of the panel-thumbnail display option.
        let mut targets = self.command_icon_targets();
        let mut candidates: [Vec<ThumbnailTarget>; 2] = Default::default();
        for side in [PaneId::Left, PaneId::Right] {
            let slot = side.index();
            // Classification can arrive after the listing. Unknown must not start a read
            // before a delayed network classification when network previews are disabled.
            if !self.panel_thumbnails_allowed(side) {
                continue;
            }
            let pane = &self.panes[slot];
            let archive_address = pane
                .thumbnail_archive_location()
                .map(|location| std::sync::Arc::new(location.address.clone()));
            let (first, end) = ranges[slot];
            // Viewport ranges are bounded by visible cells; the request budget is
            // applied below, after both panes' actual candidates are known.
            let targets = &mut candidates[slot];
            for row in first..end {
                let Some(index) = row.checked_sub(pane.parent_offset()) else {
                    continue;
                };
                let Some(entry) = pane.display_entry(index) else {
                    continue;
                };
                if entry.kind != EntryKind::File {
                    continue;
                }
                let Some(content) = thumbnail_content(
                    pane,
                    entry,
                    self.preferences.values.thumbnails_network,
                    archive_address.as_ref(),
                ) else {
                    continue;
                };
                let key = identity(pane, entry, side);
                targets.push(ThumbnailTarget {
                    hovered: pane.row_identity(entry) == hovered[slot],
                    identity: key,
                    confirmed_ssd: matches!(content, ThumbnailContent::Native(_))
                        && pane.visible_storage_kind() == StorageKind::LocalSsd,
                    content,
                    selected: pane.selection_visible(entry),
                    edge: u32::from(self.preferences.values.thumbnail_size),
                });
            }
        }
        let budget = MAX_THUMBNAIL_TARGETS.saturating_sub(targets.len());
        let quotas = split_quota(budget, [candidates[0].len(), candidates[1].len()]);
        for (pane, quota) in candidates.into_iter().zip(quotas) {
            // Rows arrive in viewport order: the top of each grid comes first.
            targets.extend(pane.into_iter().take(quota));
        }
        for target in &targets {
            if let ThumbnailContent::Archive {
                network_allowed, ..
            } = &target.content
                && self
                    .thumbnails
                    .archive_network_policy
                    .get(&target.identity)
                    .is_some_and(|old| old != network_allowed)
            {
                self.thumbnails.remove(&target.identity);
            }
            if self.thumbnails.images.contains_key(&target.identity) {
                self.thumbnails.touch(&target.identity);
            }
        }
        self.thumbnails.visible = targets.clone();
        self.thumbnails.trim();
        targets
    }
    /// Test helper: a 96-pixel request. The thumbnail host uses
    /// [`Self::receive_thumbnail_at_resolution`].
    #[doc(hidden)]
    pub fn receive_thumbnail(
        &mut self,
        identity: &str,
        frame: u8,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> bool {
        if width > 96 || height > 96 {
            return false;
        }
        self.receive_thumbnail_at_resolution(identity, frame, width, height, 96, rgba)
    }
    /// Retained request coverage acknowledges uploads and unchanged tiny-source pixels.
    /// Budget refusals leave the previous coverage intact, so the host can retry later.
    pub fn thumbnail_coverage(&self, identity: &str, frame: u8) -> u32 {
        self.thumbnails
            .coverage
            .get(identity)
            .and_then(|edges| edges.get(usize::from(frame)))
            .copied()
            .unwrap_or(0)
    }
    /// Present a completed requested resolution without discarding a resident fallback.
    /// Coverage records the request edge even when the source itself is smaller.
    pub fn receive_thumbnail_at_resolution(
        &mut self,
        identity: &str,
        frame: u8,
        width: u32,
        height: u32,
        edge: u32,
        rgba: &[u8],
    ) -> bool {
        if frame >= 6
            || width == 0
            || height == 0
            || edge == 0
            || edge > 512
            || width > edge
            || height > edge
            || rgba.len() != width as usize * height as usize * 4
            || !self
                .thumbnails
                .visible
                .iter()
                .any(|target| target.identity == identity)
        {
            return false;
        }
        let archive_policy = self
            .thumbnails
            .visible
            .iter()
            .find(|target| target.identity == identity)
            .and_then(|target| match &target.content {
                ThumbnailContent::Archive {
                    network_allowed, ..
                } => Some(*network_allowed),
                _ => None,
            });
        if archive_policy.is_some_and(|policy| {
            !self.preferences.values.thumbnails_archives
                || policy != self.preferences.values.thumbnails_network
        }) {
            return false;
        }
        let coverage = self.thumbnail_coverage(identity, frame);
        if edge <= coverage {
            return false;
        }
        let previous = self
            .thumbnails
            .images
            .get(identity)
            .and_then(|frames| frames[usize::from(frame)].as_ref())
            .map(Image::size);
        if previous.is_some_and(|size| {
            width < size.width
                || height < size.height
                || (width == size.width && height == size.height)
        }) {
            // A tiny source can satisfy a larger request without producing more pixels.
            self.thumbnails
                .coverage
                .entry(identity.to_owned())
                .or_insert([0; 6])[usize::from(frame)] = edge;
            return false;
        }
        let previous_bytes =
            previous.map_or(0, |size| size.width as usize * size.height as usize * 4);
        if !self
            .thumbnails
            .reserve(identity, previous_bytes, rgba.len())
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
        self.thumbnails.bytes -= previous_bytes;
        self.thumbnails
            .coverage
            .entry(identity.to_owned())
            .or_insert([0; 6])[usize::from(frame)] = edge;
        if let Some(policy) = archive_policy {
            self.thumbnails
                .archive_network_policy
                .insert(identity.to_owned(), policy);
        }
        self.thumbnails.uploaded(identity, rgba.len());
        if identity.starts_with("command-icon:") {
            self.command_toolbar
                .image_revision
                .set(self.command_toolbar.image_revision.get().wrapping_add(1));
        }
        true
    }
    pub(crate) fn custom_command_icon(&self, revision: &str, id: &str) -> Option<Image> {
        if revision.parse::<u64>().ok() != Some(self.command_toolbar.revision.get()) {
            return None;
        }
        self.thumbnails
            .images
            .get(&crate::user_command_presentation::command_icon_identity(
                revision, id,
            ))?
            .first()?
            .clone()
    }
    pub fn advance_thumbnail_animation(&mut self) -> bool {
        self.thumbnails.pulse = self.thumbnails.pulse.wrapping_add(1);
        !self.animated_thumbnail_identities().is_empty()
    }
    pub fn animated_thumbnail_identities(&self) -> Vec<String> {
        self.thumbnails
            .visible
            .iter()
            .filter(|target| {
                (target.selected || target.hovered)
                    && self
                        .thumbnails
                        .images
                        .get(&target.identity)
                        .is_some_and(|frames| frames.iter().flatten().count() > 1)
            })
            .map(|target| target.identity.clone())
            .collect()
    }
    pub(crate) fn thumbnail_image(&self, side: PaneId, entry: &Entry) -> Option<Image> {
        if entry.kind != EntryKind::File || !self.panel_thumbnails_allowed(side) {
            return None;
        }
        let key = identity(&self.panes[side.index()], entry, side);
        if entry.virtual_node.is_some()
            && self.thumbnails.archive_network_policy.get(&key)
                != Some(&self.preferences.values.thumbnails_network)
        {
            return None;
        }
        let target = self
            .thumbnails
            .visible
            .iter()
            .find(|target| target.identity == key);
        let frames = self.thumbnails.images.get(&key)?;
        if target.is_some_and(|target| target.hovered || target.selected) {
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

fn thumbnail_content(
    pane: &Pane,
    entry: &Entry,
    network_allowed: bool,
    archive_address: Option<&std::sync::Arc<midnight_core::ArchiveAddress>>,
) -> Option<ThumbnailContent> {
    if let Some(location) = pane.thumbnail_archive_location() {
        let mount = location.mount.as_ref()?;
        let node = entry.virtual_node.as_deref()?;
        if !node.readable
            || node.kind != EntryKind::File
            || node.size.is_none()
            || node.id.entry < 2
            || node.id.mount == 0
            || node.id.mount != location.mount_id()?
            || node.parent != location.directory
        {
            return None;
        }
        Some(ThumbnailContent::Archive {
            mount: mount.clone(),
            node: node.clone(),
            address: archive_address?.clone(),
            network_allowed,
        })
    } else {
        entry
            .native_path()
            .map(|path| ThumbnailContent::Native(path.to_owned()))
    }
}

#[cfg(test)]
#[path = "thumbnail_retention_tests.rs"]
mod retention_tests;
#[cfg(test)]
mod tests {
    use super::*;
    fn presenter() -> Presenter {
        let mut presenter = Presenter::new(PathBuf::new(), PathBuf::new());
        presenter.preferences.values.panel_view_modes = [PanelViewMode::Thumbnails; 2];
        // These synthetic visible rows have no native storage classification.
        // Their admission is simulated directly below; permit unknown storage
        // explicitly instead of bypassing the real cached-image policy.
        presenter.preferences.values.thumbnails_network = true;
        presenter
    }
    fn target(key: String, selected: bool) -> ThumbnailTarget {
        ThumbnailTarget {
            identity: key,
            content: ThumbnailContent::Native(PathBuf::from("sample.avi")),
            selected,
            hovered: false,
            confirmed_ssd: false,
            edge: 72,
        }
    }
    #[test]
    fn request_budget_is_shared_only_by_panes_with_candidates() {
        assert_eq!(split_quota(256, [187, 0]), [187, 0]);
        assert_eq!(split_quota(256, [0, 187]), [0, 187]);
        assert_eq!(split_quota(256, [187, 187]), [128, 128]);
        assert_eq!(split_quota(256, [50, 300]), [50, 206]);
        assert_eq!(split_quota(256, [300, 50]), [206, 50]);
        assert_eq!(split_quota(255, [300, 300]), [127, 128]);
        assert_eq!(split_quota(0, [10, 10]), [0, 0]);
    }
    #[test]
    fn one_large_thumbnail_grid_uses_the_whole_request_budget() {
        let root = std::env::current_dir().unwrap();
        let mut p = Presenter::new(root.clone(), root.clone());
        p.preferences.values.thumbnails_network = true;
        p.preferences.values.panel_view_modes =
            [PanelViewMode::Thumbnails, PanelViewMode::Detailed];
        let entries: std::sync::Arc<[Entry]> = (0..300)
            .map(|id| {
                Entry::new(
                    midnight_core::EntryId(id),
                    root.join(format!("{id}.png")),
                    EntryKind::File,
                )
            })
            .collect();
        p.panes[0].index =
            std::sync::Arc::new(midnight_core::DirectoryIndex::from_entries(&entries));
        p.panes[0].entries = entries;
        let offset = p.panes[0].parent_offset();
        let targets = p.thumbnail_targets([(offset, offset + 200), (0, 40)], ["", ""]);
        assert_eq!(
            targets.len(),
            200,
            "a Detailed right pane must not reserve half of the request"
        );
        let targets = p.thumbnail_targets([(offset, offset + 300), (0, 40)], ["", ""]);
        assert_eq!(targets.len(), MAX_THUMBNAIL_TARGETS);
        assert!(
            targets[0].identity.ends_with(":entry:0"),
            "the top of the grid keeps priority"
        );
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
        assert_eq!(p.thumbnails.images.len(), 1);
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
