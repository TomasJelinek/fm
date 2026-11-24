//! When the panels' free space is read again (`Command::RefreshSpace`). A listing
//! reads it with each navigation and re-read; besides, a job that writes to or
//! removes from a panel's volume asks for it while it runs and once it ends, and so
//! do a tree refresh and the main window gaining the focus. The backend coalesces and
//! throttles the reads, and reads a fixed local disk periodically while the window is
//! visible (`Command::SetWindowVisible`).
use super::*;

pub(crate) struct State {
    /// A job changed the pane's volume; sent once the pane shows its folder.
    owed: [bool; 2],
    /// The visibility the backend last accepted.
    visible: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owed: [false; 2],
            visible: true,
        }
    }
}

/// The volume that holds `path`, by its text: the longest listed volume root that
/// contains it, else its drive, UNC share or filesystem root. The Home location is a
/// folder, not a volume. A link or mount that the list does not name can hide another
/// volume; that panel then shows the change with its next listing, the next focus of
/// the window or, on a fixed local disk, its next periodic read.
fn volume_of<'a>(volumes: &'a [Volume], path: &'a Path) -> &'a Path {
    volumes
        .iter()
        .filter(|volume| volume.id != "home" && path.starts_with(&volume.root))
        .map(|volume| volume.root.as_path())
        .max_by_key(|root| root.components().count())
        .unwrap_or_else(|| path.ancestors().last().unwrap_or(path))
}

/// A job wrote to or removed from `changed`: each panel whose folder is on the volume
/// of one of them reads its free space again. Takes the fields it needs so callers
/// can hold other parts of the presenter meanwhile.
pub(super) fn job_changed<'a>(
    state: &mut State,
    volumes: &[Volume],
    panes: &[Pane; 2],
    changed: impl IntoIterator<Item = &'a Path>,
) {
    let shown = panes.each_ref().map(|pane| {
        pane.native_capabilities()
            .then(|| volume_of(volumes, &pane.path))
    });
    for path in changed {
        let volume = volume_of(volumes, path);
        for (owed, shown) in state.owed.iter_mut().zip(shown) {
            *owed |= shown == Some(volume);
        }
    }
}

impl Presenter {
    /// Asks the backend to read `pane`'s free space again. False only while that
    /// must wait: a loading pane reads it with its listing, which may have started
    /// before the change, and a full command queue refused it.
    fn request_space(&self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let p = &self.panes[pane.index()];
        if !p.native_capabilities() {
            return true;
        }
        if p.loading {
            return false;
        }
        match p.successful_generation {
            Some(navigation) => send(Command::RefreshSpace { pane, navigation }),
            None => true,
        }
    }

    /// Sends the reads that jobs asked for and that can start now.
    pub(super) fn flush_space_refreshes(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if self.space_refresh.owed[pane.index()] && self.request_space(pane, send) {
                self.space_refresh.owed[pane.index()] = false;
            }
        }
    }

    /// A tree refresh may follow changes anywhere below the tree's root.
    pub(super) fn tree_changed_space(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.space_refresh.owed[pane.index()] = true;
        self.flush_space_refreshes(send);
    }

    /// The pane shows another location: what a job owed the previous one is moot.
    pub(super) fn forget_space_refresh(&mut self, pane: PaneId) {
        self.space_refresh.owed[pane.index()] = false;
    }

    /// The main window's visibility changed, or it gained the focus (`focused`):
    /// another program may have changed the volumes meanwhile.
    pub fn window_presence(
        &mut self,
        visible: bool,
        focused: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if visible != self.space_refresh.visible && send(Command::SetWindowVisible { visible }) {
            self.space_refresh.visible = visible;
        }
        if focused {
            self.space_refresh.owed = [true; 2];
        }
        self.flush_space_refreshes(send);
    }
}
