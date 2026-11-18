//! Recolor existing semantic pages without changing typography or source/camera authority.
use super::*;

impl Worker {
    pub(super) fn sync_palette(&mut self, output: &Mailbox) {
        let (revision, colors) = self.palette_state.snapshot();
        if revision == self.palette_revision {
            return;
        }
        self.palette_revision = revision;
        let changed = colors != crate::palette::Colors::from(&self.appearance);
        colors.apply(&mut self.appearance);
        if changed {
            if let Some((_, page)) = self.position_page.as_mut() {
                page.recolor(&self.appearance);
            }
            if let Some(mut page) = self.page.take() {
                page.recolor(&self.appearance);
                let layout = self.layout_page(&page);
                self.page = Some(page);
                match layout {
                    Ok(shaped) => self.shaped = Some(shaped),
                    Err(error) => {
                        // Retain the decoded page and the already displayed surface. No
                        // successful new text fence is possible without a shaped page.
                        self.shaped = None;
                        self.status(format!("Cannot recolor this page: {error:?}"), output);
                    }
                }
            }
            self.text.gutter = None;
            self.raster.palette_changed();
        }
        // A->B->A must produce a new real fence even if B was never applied.
        // Correctly colored retained pixels may be reused for that new frame.
        self.dirty = true;
    }

    pub(super) fn announce_content_palette(&mut self, output: &Mailbox) {
        if self.image.is_some() || self.web.is_some() {
            self.acknowledge_palette(output, None);
        }
    }

    pub(super) fn acknowledge_palette(&mut self, output: &Mailbox, floor: Option<u64>) {
        let key = (
            self.palette_revision,
            self.positions.generation,
            floor.is_some(),
        );
        if self.palette_ack == Some(key) {
            return;
        }
        output.put(ViewerOutput::PaletteApplied {
            revision: self.palette_revision,
            generation: self.positions.generation,
            minimum_frame_revision: floor,
        });
        self.palette_ack = Some(key);
    }
}
