//! Frame identities that UI input may name.
//!
//! A selection repaint must not invalidate coordinates from the visible page. Copy
//! names what the UI showed: a later scroll, position refinement, restyle or reflow
//! that keeps the selection does not retire it, but a frame or retained surface
//! published before the selection changed cannot copy the new one.

use midnight_plugin_host::api::viewer::Anchor;

/// The first frame revision and retained raster surface published after the
/// selection last changed. Every one published since shows the current selection.
#[derive(Default)]
struct SelectionStamp {
    frame: u64,
    surface: u64,
}

#[derive(Default)]
pub(crate) struct Frames {
    revision: u64,
    minimum_input: u64,
    /// The newest retained raster surface that a painted frame carried.
    surface: u64,
    selection: SelectionStamp,
}

impl Frames {
    pub fn latest_revision(&self) -> u64 {
        self.revision
    }
    pub fn accepts(&self, frame: u64) -> bool {
        frame != 0 && frame >= self.minimum_input && frame <= self.revision
    }

    /// Besides every frame that [`Self::accepts`], Copy accepts any frame painted
    /// since the selection last changed, whatever invalidated its coordinates later.
    pub fn accepts_copy(&self, frame: u64) -> bool {
        self.accepts(frame)
            || (frame != 0 && frame >= self.selection.frame && frame <= self.revision)
    }

    /// [`Self::accepts_copy`] for a retained surface that a later context replaced.
    /// The current surface is accepted by the surface check of every raster input.
    pub fn accepts_surface_copy(&self, surface: u64) -> bool {
        surface != 0 && surface >= self.selection.surface && surface <= self.surface
    }

    pub fn invalidate(&mut self) {
        self.minimum_input = self.revision.saturating_add(1);
    }

    /// The selection that Copy returns changed. Frames and surfaces published so far
    /// showed another one, so Copy may name only later ones (or a frame that
    /// [`Self::accepts`], such as the one a pointer selection was made on).
    pub fn selection_changed(&mut self) {
        self.selection = SelectionStamp {
            frame: self.revision.saturating_add(1),
            surface: self.surface.saturating_add(1),
        };
    }

    pub fn painted(&mut self) -> Result<u64, String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Viewer frame identity exhausted")?;
        Ok(self.revision)
    }

    /// The frame just painted carries this retained raster surface.
    pub fn painted_surface(&mut self, surface: u64) {
        self.surface = self.surface.max(surface);
    }
}

/// What a selection covers, independent of the page and layout that show it: the
/// source offsets of its ends. Another page showing the same source range shows the
/// same selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Selected {
    Nothing,
    Source([u64; 2]),
    /// An end without a source anchor; never provably the same as another selection.
    Unmapped,
}

impl Selected {
    /// `anchors` are those of the page whose text offsets `selection` holds; they are
    /// strictly ordered by text offset and only listed boundaries are exact.
    pub fn on(anchors: &[Anchor], selection: Option<(usize, usize)>) -> Self {
        let Some((first, last)) = selection else {
            return Self::Nothing;
        };
        let source = |index: usize| {
            let at = anchors.partition_point(|anchor| (anchor.text_offset as usize) < index);
            anchors
                .get(at)
                .filter(|anchor| anchor.text_offset as usize == index)
                .map(|anchor| anchor.source_offset)
        };
        match (source(first), source(last)) {
            (Some(first), Some(last)) => Self::Source([first.min(last), first.max(last)]),
            _ => Self::Unmapped,
        }
    }

    pub fn same(self, other: Self) -> bool {
        self != Self::Unmapped && self == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_and_dragging_remain_valid_until_page_or_camera_changes() {
        let mut frames = Frames::default();
        assert!(!frames.accepts(0));
        let visible = frames.painted().unwrap();
        frames.painted().unwrap(); // Selection was painted; UI still shows `visible`.
        assert!(frames.accepts(visible));
        assert!(!frames.accepts(frames.revision + 1));
        frames.invalidate(); // A page, scroll origin or size changed.
        assert!(!frames.accepts(visible));
        let next = frames.painted().unwrap();
        assert!(frames.accepts(next));
        assert!(!frames.accepts(visible));
    }

    #[test]
    fn copy_names_any_frame_or_surface_that_showed_the_current_selection() {
        let mut frames = Frames::default();
        let before = frames.painted().unwrap();
        frames.painted_surface(1);
        // A find selects a match, retires the camera and repaints on a new surface.
        frames.selection_changed();
        frames.invalidate();
        let found = frames.painted().unwrap();
        frames.painted_surface(2);
        // A position refinement keeps that selection on another camera and surface.
        frames.invalidate();
        let refined = frames.painted().unwrap();
        frames.painted_surface(3);
        assert!(
            !frames.accepts(found),
            "coordinates of the find frame retired"
        );
        assert!(frames.accepts_copy(found) && frames.accepts_copy(refined));
        assert!(!frames.accepts_copy(before), "it showed no match");
        assert!(!frames.accepts_copy(0) && !frames.accepts_copy(refined + 1));
        assert!(frames.accepts_surface_copy(2) && frames.accepts_surface_copy(3));
        assert!(!frames.accepts_surface_copy(1), "it showed no match");
        assert!(!frames.accepts_surface_copy(4), "not published yet");

        // A pointer selection names the shown frame, whose coordinates stay valid.
        frames.selection_changed();
        assert!(frames.accepts_copy(refined));
        assert!(!frames.accepts_copy(found));
        assert!(
            !frames.accepts_surface_copy(3),
            "published before the selection"
        );
        let selected = frames.painted().unwrap();
        frames.painted_surface(3); // A selection-only repaint keeps its surface.
        frames.invalidate();
        frames.painted().unwrap();
        frames.painted_surface(4);
        assert!(frames.accepts_copy(selected));
        assert!(
            !frames.accepts_copy(refined),
            "it showed the match, not the selection"
        );
        assert!(frames.accepts_surface_copy(4));
        assert!(
            !frames.accepts_surface_copy(3),
            "an older frame of this surface showed the match"
        );
    }

    #[test]
    fn selections_compare_by_source_range_whatever_page_shows_them() {
        let page = |source: u64, gaps: bool| -> Vec<Anchor> {
            (0..=20)
                .filter(|at| !gaps || at % 5 == 0)
                .map(|at| Anchor {
                    text_offset: at,
                    source_offset: source + u64::from(at),
                })
                .collect()
        };
        let (first, later) = (page(100, false), page(90, false));
        let shown = Selected::on(&first, Some((2, 7)));
        assert_eq!(shown, Selected::Source([102, 107]));
        assert!(shown.same(Selected::on(&later, Some((17, 12)))));
        assert!(!shown.same(Selected::on(&later, Some((12, 16)))));
        assert!(!shown.same(Selected::on(&later, None)));
        assert!(Selected::on(&first, None).same(Selected::on(&later, None)));
        let unmapped = Selected::on(&page(100, true), Some((2, 5)));
        assert_eq!(unmapped, Selected::Unmapped);
        assert!(!unmapped.same(unmapped));
        assert!(!Selected::on(&[], Some((0, 0))).same(Selected::Nothing));
    }
}
