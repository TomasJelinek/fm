//! Viewer messages contain immutable snapshots; all file work belongs to the backend.
use crate::{DirectoryIndex, Entry};
use std::{path::PathBuf, sync::Arc};
#[path = "viewer_tiles.rs"]
mod tiles;
pub use tiles::*;
#[path = "viewer_raster.rs"]
mod raster;
pub use raster::*;
#[path = "viewer_associations.rs"]
mod associations;
pub use associations::ViewerAssociations;
#[path = "viewer_names.rs"]
mod names;
pub use names::viewer_plugin_name;

#[derive(Clone, Debug)]
pub struct ViewerLaunch {
    /// Startup-captured routing overrides; manifests and capabilities remain authoritative.
    pub associations: Arc<ViewerAssociations>,
    /// Host-generated immutable text; never reopened by a display pathname.
    pub generated: Option<Arc<dyn crate::GeneratedTextSource>>,
    /// Accepted choices captured before the first content frame.
    pub preferences: Arc<crate::ViewerPreferences>,
    /// Provider-scoped focus for virtual content; `focused` is unused when present.
    pub archive: Option<ArchiveViewer>,
    /// Exact adopted inline tree; normalized only by the viewer backend.
    pub tree: Option<TreeViewer>,
    pub entries: Arc<[Entry]>,
    pub view: Arc<DirectoryIndex>,
    pub focused: PathBuf,
    pub basic: bool,
    /// Preferences captured at application startup. Changes take effect after restart.
    pub disabled_plugins: Arc<[String]>,
}

/// Immutable viewer scope for an adopted tree. Only visible regular root aliases are eligible.
/// The launch entries/index must be the exact allocations in this projection's root.
#[derive(Clone, Debug)]
pub struct TreeViewer {
    pub view: Arc<crate::TreeProjection>,
    pub focused: crate::TreeRowId,
}

#[derive(Clone, Debug)]
pub struct ArchiveViewer {
    pub mount: Arc<dyn crate::ArchiveMount>,
    pub source_version: Vec<u8>,
    pub focused: crate::VirtualEntryId,
}

#[derive(Clone, Debug)]
pub struct ViewerPluginInfo {
    pub extensions: Vec<String>,
    /// The original manifest declares a catch-all plain text viewer.
    pub fallback: bool,
    pub id: String,
    pub name: String,
    /// Functional details from metadata, separate from the short list title.
    pub description: String,
    pub version: String,
    pub status: String,
    pub location: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesystemPluginKind {
    Native,
    Archive,
    Network,
}

/// A manifest-only catalog record. Installed means metadata/artifacts were
/// validated; it does not assert that a library or worker has been loaded.
#[derive(Clone, Debug)]
pub struct FilesystemPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Diagnostics have no configurable provider identity or declared kind.
    pub kind: Option<FilesystemPluginKind>,
    pub location: Option<PathBuf>,
    pub status: String,
}

/// A thumbnail helper discovered from trusted metadata without starting its worker.
/// Installed means metadata/artifacts were validated, not that decoding succeeded.
#[derive(Clone, Debug)]
pub struct ThumbnailerPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Manifest routing hints; each worker still validates the source content.
    pub extensions: Vec<String>,
    /// Supports additional sampled frames for selected or hovered video previews.
    pub video: bool,
    /// The validated helper executable in the installation's thumbnailers directory.
    pub location: PathBuf,
    pub status: String,
}

pub fn validate_disabled_viewers(ids: &[String]) -> Result<(), &'static str> {
    if ids.len() > 64 {
        return Err("At most 64 disabled viewer IDs can be stored.");
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err("Invalid viewer plugin ID.");
        }
        if !seen.insert(id) {
            return Err("Viewer plugin IDs must be unique.");
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ViewerAppearance {
    pub family: String,
    pub font_size: f32,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
    pub selection: [u8; 4],
    pub accent: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct ViewerGeometry {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Host-owned process image mode; independent from the native plugin ABI modes.
pub const VIEWER_MODE_IMAGE: u16 = 5;
/// Interactive browser surface; independent of semantic plugin ABI modes.
pub const VIEWER_MODE_HTML: u16 = 6;
/// Platform-neutral interactive surface contract shared by backend and presentation.
pub use midnight_web_viewer_api as viewer_web;

#[derive(Clone, Debug)]
pub enum ViewerImageInput {
    /// One atomic gesture anchored in the actually displayed source, independent
    /// of a prefetched backend camera. The enclosing frame scopes its coordinates.
    ViewZoom {
        zoom: crate::ViewerZoomPreference,
        source_anchor: [f64; 2],
        viewport_anchor: [f32; 2],
    },
    Fit,
    ActualPixels,
    Zoom {
        factor: f64,
        anchor: Option<[f32; 2]>,
    },
    Drag {
        x: f32,
        y: f32,
    },
    /// Latest absolute camera; older sequences and replaced surfaces are refused.
    PanTo {
        surface: ViewerSurfaceId,
        sequence: u64,
        origin: [f64; 2],
    },
}

#[derive(Clone, Debug)]
pub struct ViewerImageInfo {
    pub surface: ViewerSurfaceId,
    pub origin: [f64; 2],
    pub visible_source: [f64; 2],
    pub raster_scale: f64,
    pub pan_sequence: u64,
    pub width: u32,
    pub height: u32,
    pub zoom: f64,
    pub fitted: bool,
    /// One-based document page; zero for ordinary images.
    pub page: u32,
    pub page_count: u32,
}

#[derive(Clone, Debug)]
pub enum ViewerInput {
    /// Opt in to app-owned retained text pixels; legacy CPU consumers stay viewport-only.
    RetainedRaster(bool),
    Raster {
        surface: ViewerSurfaceId,
        input: ViewerRasterInput,
    },
    /// Reply to an automatic retained-context handoff with the camera actually
    /// displayed in the old surface's effective raster pixel coordinates.
    RasterRebase {
        transition: u64,
        origin: [f64; 2],
    },
    /// Browser input only; host validates authority-bearing commands separately.
    Web {
        frame: u64,
        input: viewer_web::CommandKind,
    },
    Resize(ViewerGeometry),
    Scroll {
        x: f32,
        y: f32,
    },
    /// Normalized file position. Actual anchors and offsets remain u64 on the backend.
    Seek(f64),
    Pan(f64),
    /// Absolute scrollbar intent scoped to the currently displayed text context.
    Position {
        generation: u64,
        sequence: u64,
        horizontal: bool,
        position: f64,
    },
    NextFile,
    Mode(u16),
    /// Reflow Text/Source within the viewport; ignored by fixed-column modes.
    Wrap(bool),
    Image {
        frame: u64,
        input: ViewerImageInput,
    },
    DocumentPage {
        frame: u64,
        delta: i32,
    },
    Search {
        query: String,
        backwards: bool,
        case_sensitive: bool,
    },
    Select {
        frame: u64,
        x: f32,
        y: f32,
        extend: bool,
    },
    SelectPage {
        frame: u64,
    },
    Copy {
        frame: u64,
    },
    Close,
}

#[derive(Clone, Debug)]
pub struct ViewerFrame {
    pub generation: u64,
    /// Vertical, horizontal; advanced only when matching content has been prepared.
    pub position_sequence: [u64; 2],
    pub revision: u64,
    pub mode: u16,
    pub modes: Vec<u16>,
    pub wrap_enabled: bool,
    pub wrap_available: bool,
    pub image: Option<ViewerImageInfo>,
    /// Text/source/Markdown retained camera; never changes image-mode UI semantics.
    pub raster: Option<ViewerRasterInfo>,
    /// Sticky acknowledgement of a completed displayed-camera handoff. Retained
    /// until accepted subsequent input or an explicit context invalidation.
    pub raster_rebase: Option<u64>,
    pub width: u32,
    pub height: u32,
    /// Bounded premultiplied RGBA prepared by backend text or image workers.
    pub pixels: Arc<[u8]>,
    /// Reusable immutable tiles; single-raster pixels is empty when present.
    pub tiles: Option<Arc<[ViewerTile]>>,
    pub accessible_text: Arc<str>,
    pub position: f64,
    pub vertical_extent: f64,
    pub horizontal_position: f64,
    pub horizontal_extent: f64,
    pub caption: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub enum ViewerOutput {
    WebState {
        generation: u64,
        policy: viewer_web::Policy,
        zoom: f64,
    },
    /// Host-owned commands from an interactive viewer, after browser arbitration.
    WebShortcut(viewer_web::HostShortcut),
    Frame(Box<ViewerFrame>),
    /// Keep the current picture until the UI replies with its displayed camera.
    RasterRebase {
        surface: ViewerSurfaceId,
        transition: u64,
    },
    RasterUnavailable {
        surface: ViewerSurfaceId,
        message: String,
    },
    ImageUnavailable {
        generation: u64,
        surface: ViewerSurfaceId,
        message: String,
    },
    Status(String),
    CopyText(String),
    Closed,
}
