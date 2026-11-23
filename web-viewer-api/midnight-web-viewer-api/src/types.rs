//! Wire values carry opaque identities, never native paths or engine objects.

/// Maximum encoded control section in bytes, including the envelope identity.
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;
/// Maximum raw source data in a single reply.
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// Maximum edge of a physical viewport or frame.
pub const MAX_EDGE: u32 = 8192;
/// Maximum number of physical pixels in a frame.
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
/// Maximum raw RGBA8 frame bytes.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Protocol version; existing semantic and image protocols are unrelated.
pub const PROTOCOL_VERSION: u16 = 1;

/// Explicit host launch opt-in for the physical-key extension. Never inherit this value.
/// Providers must emit only baseline tags unless the launching host supplies the exact value.
pub const HOST_SHORTCUTS_ENV: &str = "MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS";
/// Enables Backspace/AltLeft/AltRight after the browser's live editable-focus arbitration.
pub const HOST_SHORTCUTS_PHYSICAL_V1: &str = "physical-v1";

/// Explicit host launch opt-in for link routing (API 0.3). Never inherit this value.
/// Providers emit `LinkHover`/`LinkActivated` only for the exact value below; otherwise
/// they keep every link navigation inside the browser, as API 0.2 providers do.
pub const LINKS_ENV: &str = "MIDNIGHT_WEB_VIEWER_LINKS";
/// Reports the hovered link and hands every followed link to the host instead of
/// navigating the browser.
pub const LINKS_HOST_V1: &str = "host-v1";
/// Longest link target in UTF-8 bytes.
pub const MAX_LINK_TARGET_BYTES: usize = 8192;

/// Explicit host launch opt-in for pointer cursor reports (API 0.4). Never inherit this
/// value. Providers emit `Cursor` events only for the exact value below.
pub const CURSOR_ENV: &str = "MIDNIGHT_WEB_VIEWER_CURSOR";
/// Reports the pointer cursor the page asks for whenever it changes.
pub const CURSOR_V1: &str = "cursor-v1";

/// Identities are scoped to one process; zero is reserved and invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub session: u64,
    pub document: u64,
    /// Strictly increasing per direction and document; wraparound is forbidden.
    pub sequence: u64,
}

/// Physical canvas dimensions and physical pixels per CSS pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// Explicit document permissions. Before API 0.4 scripts required network; since 0.4 a
/// policy may enable scripts offline, and the provider keeps them from any network.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub network: bool,
    pub scripts: bool,
}

/// Host-owned read capability, not a native file or source URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub id: u64,
    pub length: u64,
    pub mime: String,
}

/// Owned premultiplied sRGB RGBA8 pixels, packed rows with no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub viewport_revision: u64,
    /// Most recent command sequence incorporated into these pixels; zero means none.
    pub acknowledged_input: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Platform-neutral modifier state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Move,
    Down,
    Up,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    None,
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

/// Canvas-local physical pixels. Negative finite positions allow captured drags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub kind: PointerKind,
    pub button: PointerButton,
    pub x: f64,
    pub y: f64,
    pub modifiers: Modifiers,
    pub click_count: u8,
}

/// Wheel deltas are physical pixels; the backend converts native units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wheel {
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub modifiers: Modifiers,
}

/// W3C UI Events key/code names; native translation stays in platform adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub key: String,
    pub code: String,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: Modifiers,
}

/// IME selection offsets count UTF-8 bytes and must lie on character boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    Start,
    Update {
        text: String,
        selection_start: u32,
        selection_end: u32,
    },
    Commit(String),
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Find {
    pub query: String,
    pub forward: bool,
    pub match_case: bool,
    pub find_next: bool,
}

/// Correlates exactly with a provider request; denial never permits fallback loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceReply {
    Resolved {
        request: u64,
        resource: Resource,
    },
    Data {
        request: u64,
        resource: u64,
        offset: u64,
        bytes: Vec<u8>,
    },
    Released {
        resource: u64,
    },
    Denied {
        request: u64,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub header: Header,
    pub kind: CommandKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandKind {
    Open {
        source: Resource,
        viewport: Viewport,
        policy: Policy,
    },
    Resize(Viewport),
    Pointer(Pointer),
    Wheel(Wheel),
    Key(Key),
    Text(String),
    Ime(Ime),
    Focus(bool),
    Find(Find),
    /// Absolute CSS zoom factor, finite and in 0.1..=10.0.
    Zoom(f64),
    Copy,
    Policy(Policy),
    ResourceReply(ResourceReply),
    FrameAck {
        frame_sequence: u64,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub title: String,
    /// Display-only URL; this never grants resource authority.
    pub url: String,
    pub loading: bool,
    pub editable: bool,
    pub zoom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindResult {
    pub request_sequence: u64,
    pub matches: u32,
    /// One-based active match, or zero when there is no active match.
    pub active: u32,
    pub complete: bool,
}

/// The pointer cursor a page asks for (API 0.4, `Cursor` event). Cursors without a
/// counterpart here (custom images included) are reported as `Default`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Default,
    /// The link hand.
    Pointer,
    Text,
    VerticalText,
    Crosshair,
    Wait,
    Progress,
    Help,
    Move,
    AllScroll,
    NotAllowed,
    Grab,
    Grabbing,
    ColResize,
    RowResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ZoomIn,
    ZoomOut,
    Copy,
    Alias,
    ContextMenu,
    Cell,
    /// The page hides the cursor.
    None,
}

impl Cursor {
    /// Every cursor in wire order: a cursor's discriminant is its index.
    pub const ALL: [Cursor; 26] = [
        Cursor::Default,
        Cursor::Pointer,
        Cursor::Text,
        Cursor::VerticalText,
        Cursor::Crosshair,
        Cursor::Wait,
        Cursor::Progress,
        Cursor::Help,
        Cursor::Move,
        Cursor::AllScroll,
        Cursor::NotAllowed,
        Cursor::Grab,
        Cursor::Grabbing,
        Cursor::ColResize,
        Cursor::RowResize,
        Cursor::EwResize,
        Cursor::NsResize,
        Cursor::NeswResize,
        Cursor::NwseResize,
        Cursor::ZoomIn,
        Cursor::ZoomOut,
        Cursor::Copy,
        Cursor::Alias,
        Cursor::ContextMenu,
        Cursor::Cell,
        Cursor::None,
    ];
}

/// Explicit host actions, arbitrated by the browser's current editable focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostShortcut {
    Close,
    Find,
    FindNext,
    FindPrevious,
    CycleNext,
    CyclePrevious,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Copy,
    /// Plain Backspace; requires the explicit physical-v1 host launch opt-in.
    Backspace,
    /// Alt+Left; requires the explicit physical-v1 host launch opt-in.
    AltLeft,
    /// Alt+Right; requires the explicit physical-v1 host launch opt-in.
    AltRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidRequest,
    Denied,
    MissingResource,
    ResourceChanged,
    Unsupported,
    Runtime,
    BudgetExceeded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub header: Header,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Ready,
    Frame(Frame),
    State(State),
    FindResult(FindResult),
    /// Selection text only, copied through the host after explicit Copy.
    Selection(String),
    ResourceResolve {
        request: u64,
        base: u64,
        url: String,
    },
    ResourceRead {
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    },
    ResourceRelease {
        resource: u64,
    },
    HostShortcut(HostShortcut),
    Error {
        code: ErrorCode,
        message: String,
    },
    Closed,
    /// The target of the link under the pointer (the browser's status link, which is the
    /// keyboard-focused link while the pointer is over none); empty once there is none.
    /// Sent only when it changes and only under the host-v1 link-routing launch opt-in.
    /// A nonempty target has the format of `LinkActivated`.
    LinkHover {
        target: String,
    },
    /// A link the user followed (click, middle or modified click, Enter, a new window or
    /// tab) to another document. The browser cancelled that navigation; only the host
    /// decides what happens. Sent only under the host-v1 link-routing launch opt-in.
    ///
    /// Outside the provider's private document origin the target is the absolute URL
    /// exactly as the browser resolved it, which starts with a scheme. On that origin it
    /// is a reference relative to the document's folder, without a scheme and keeping
    /// `?query#fragment`, such as `sub/other.html#part` or `../README.md`. Targets are
    /// nonempty, at most [`MAX_LINK_TARGET_BYTES`] of UTF-8 and free of control characters.
    LinkActivated {
        target: String,
    },
    /// The pointer cursor the page asks for, sent when it changes (API 0.4). Sent only
    /// under the cursor-v1 launch opt-in.
    Cursor(Cursor),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Command(Command),
    Event(Event),
}
