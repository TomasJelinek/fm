//! Viewer messages contain immutable snapshots; all file work belongs to the backend.
use crate::{DirectoryIndex, Entry};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct ViewerLaunch {
    /// Provider-scoped focus for virtual content; `focused` is unused when present.
    pub archive: Option<ArchiveViewer>,
    pub entries: Arc<[Entry]>,
    pub view: Arc<DirectoryIndex>,
    pub focused: PathBuf,
    pub basic: bool,
    /// Preferences captured at application startup. Changes take effect after restart.
    pub disabled_plugins: Arc<[String]>,
}

#[derive(Clone, Debug)]
pub struct ArchiveViewer {
    pub mount: Arc<dyn crate::ArchiveMount>,
    pub source_version: Vec<u8>,
    pub focused: crate::VirtualEntryId,
}

#[derive(Clone, Debug)]
pub struct ViewerPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub location: PathBuf,
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

#[derive(Clone, Debug)]
pub enum ViewerImageInput {
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
}

#[derive(Clone, Debug)]
pub struct ViewerImageInfo {
    pub width: u32,
    pub height: u32,
    pub zoom: f64,
    pub fitted: bool,
}

#[derive(Clone, Debug)]
pub enum ViewerInput {
    Resize(ViewerGeometry),
    Scroll {
        x: f32,
        y: f32,
    },
    /// Normalized file position. Actual anchors and offsets remain u64 on the backend.
    Seek(f64),
    Pan(f64),
    NextFile,
    Mode(u16),
    /// Reflow Text/Source within the viewport; ignored by fixed-column modes.
    Wrap(bool),
    Image {
        frame: u64,
        input: ViewerImageInput,
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
    pub revision: u64,
    pub mode: u16,
    pub modes: Vec<u16>,
    pub wrap_enabled: bool,
    pub wrap_available: bool,
    pub image: Option<ViewerImageInfo>,
    pub width: u32,
    pub height: u32,
    /// Bounded premultiplied RGBA prepared by backend text or image workers.
    pub pixels: Arc<[u8]>,
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
    Frame(ViewerFrame),
    Status(String),
    CopyText(String),
    Closed,
}
