//! Whether a pane in Thumbnails view may read files for previews, and what its status
//! row says when a setting withholds them or the preview service reports a failure.
//! Both are presentation only: a status row never opens a dialog or takes the keyboard.
use crate::{Presenter, presenter::Pane};
use midnight_core::{PaneId, PanelViewMode, StorageKind};

/// The storage policy of one pane's Thumbnails view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelPreviews {
    /// Not a Thumbnails view, previews switched off, or a location without previews.
    Off,
    /// The location's storage is not classified yet. Nothing is read before it is,
    /// unless network previews are allowed: a late network answer must not find a read
    /// already started.
    Pending,
    /// A setting withholds previews of this location.
    Withheld(Withheld),
    Allowed,
}

/// Why a setting withholds a pane's previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Withheld {
    /// Network storage, and network previews are not allowed.
    Network,
    /// Storage not known to be local (`StorageKind::Unknown`), and network previews,
    /// which also cover such storage, are not allowed.
    Unknown,
    /// An archive's members, and previews inside archives are off.
    Archives,
}

/// A pane's status-row text about its previews.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThumbnailStatus {
    pub(crate) text: String,
    /// The tooltip's further explanation, or empty.
    pub(crate) details: String,
}

const NETWORK_SETTING: &str = "Options › Previews › Allow previews on network locations";
const ARCHIVE_SETTING: &str = "Options › Previews › Allow previews inside archives";

impl Withheld {
    fn status(self) -> ThumbnailStatus {
        match self {
            Self::Network => ThumbnailStatus {
                text: format!("No previews of network locations: turn on {NETWORK_SETTING}."),
                details: "Reading files over the network for previews is off by default.".into(),
            },
            Self::Unknown => ThumbnailStatus {
                text: format!(
                    "No previews: this folder's storage is not known to be local. \
                     {NETWORK_SETTING} turns them on."
                ),
                details: "The system does not show which disk holds this folder, so it \
                          may be on another computer."
                    .into(),
            },
            Self::Archives => ThumbnailStatus {
                text: format!("No previews inside archives: turn on {ARCHIVE_SETTING}."),
                details: String::new(),
            },
        }
    }
}

/// The pane's policy for its own location, independent of the preview switch and view.
fn location_previews(pane: &Pane, network_allowed: bool, archives_allowed: bool) -> PanelPreviews {
    if let Some(location) = pane.thumbnail_archive_location() {
        return match (location.mount.is_some(), archives_allowed) {
            (false, _) => PanelPreviews::Off,
            (true, false) => PanelPreviews::Withheld(Withheld::Archives),
            (true, true) => PanelPreviews::Allowed,
        };
    }
    if !pane.native_capabilities() {
        return PanelPreviews::Off;
    }
    if network_allowed {
        return PanelPreviews::Allowed;
    }
    match pane.visible_storage_classification() {
        None => PanelPreviews::Pending,
        Some(kind) if kind.is_local() => PanelPreviews::Allowed,
        Some(StorageKind::Network) => PanelPreviews::Withheld(Withheld::Network),
        Some(_) => PanelPreviews::Withheld(Withheld::Unknown),
    }
}

impl Presenter {
    pub(crate) fn panel_previews(&self, side: PaneId) -> PanelPreviews {
        let values = &self.preferences.values;
        if !values.thumbnails_enabled
            || values.panel_view_modes[side.index()] != PanelViewMode::Thumbnails
        {
            return PanelPreviews::Off;
        }
        location_previews(
            &self.panes[side.index()],
            values.thumbnails_network,
            values.thumbnails_archives,
        )
    }

    /// What the pane's status row says about its previews: the setting that withholds
    /// them, else the preview service's latest notice while previews are shown.
    pub(crate) fn thumbnail_status(&self, side: PaneId) -> Option<ThumbnailStatus> {
        match self.panel_previews(side) {
            PanelPreviews::Withheld(reason) => Some(reason.status()),
            PanelPreviews::Allowed => self.thumbnails.notice.clone().map(|text| ThumbnailStatus {
                text,
                details: String::new(),
            }),
            PanelPreviews::Off | PanelPreviews::Pending => None,
        }
    }

    /// A failure the preview service reports (its cache, a thumbnailer that cannot
    /// start): shown in the status row of panes in Thumbnails view, never as a notice
    /// that takes the keyboard. It stays until another replaces it or the previews are
    /// cleared (`clear_thumbnail_images`).
    pub fn report_thumbnail_error(&mut self, error: String) {
        self.thumbnails.notice = Some(error);
    }
}
