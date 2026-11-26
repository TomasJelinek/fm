//! A panel whose drive leaves the drive list (a USB stick pulled out, a `subst` or
//! network letter removed, a mount unmounted) reads its folder again at once, so the
//! usual recovery of a folder that no longer opens (the nearest parent that does, then
//! the system drive) follows the drive list instead of the next periodic check.
use super::*;

impl Presenter {
    /// The panes that show a native folder on a volume that `event`'s drive list no
    /// longer has. Read before the event replaces the list.
    pub(super) fn panes_on_removed_volumes(&self, event: &Event) -> Vec<PaneId> {
        let Event::VolumesUpdated(volumes) = event else {
            return Vec::new();
        };
        [PaneId::Left, PaneId::Right]
            .into_iter()
            .filter(|pane| {
                let p = &self.panes[pane.index()];
                p.native_capabilities()
                    && self
                        .volumes
                        .iter()
                        .filter(|volume| p.path.starts_with(&volume.root))
                        .max_by_key(|volume| volume.root.components().count())
                        .is_some_and(|volume| {
                            !volumes.iter().any(|listed| listed.root == volume.root)
                        })
            })
            .collect()
    }

    /// Read the folder of a pane whose volume was removed again, as a watch hint would.
    /// A pane that is loading already has a listing that will fail or succeed on its
    /// own; one whose re-read waits (a drag or comparison) keeps the hint for later.
    pub(super) fn refresh_removed_volumes(
        &mut self,
        panes: Vec<PaneId>,
        send: &dyn Fn(Command) -> bool,
    ) {
        for pane in panes {
            if let Some(navigation) = self.panes[pane.index()].successful_generation {
                self.refresh_if_changed(pane, navigation, send);
            }
        }
    }
}
