# Interactive web viewer API and SDK 0.2.0

This independent Rust 2024 / Rust 1.99 workspace has no third-party dependencies.
The SDK depends only on `midnight-web-viewer-api =0.2.0`. Providers consume released
registry packages; this workspace never depends on the application, Slint, Tokio,
CEF or a private host crate. Existing semantic and raster protocols are unchanged.

## Public Rust contract

All following API names are exported from `midnight_web_viewer_api`:

```rust,ignore
struct Header { session: u64, document: u64, sequence: u64 }
struct Command { header: Header, kind: CommandKind }
struct Event { header: Header, kind: EventKind }
enum Message { Command(Command), Event(Event) }
struct Viewport { revision: u64, width: u32, height: u32, scale: f64 }
struct Policy { network: bool, scripts: bool }
struct Resource { id: u64, length: u64, mime: String }
struct Frame {
    viewport_revision: u64, acknowledged_input: u64,
    width: u32, height: u32, rgba: Vec<u8>,
}
struct Modifiers { shift: bool, control: bool, alt: bool, meta: bool }
enum PointerKind { Move, Down, Up, Leave }
enum PointerButton { None, Left, Middle, Right, Back, Forward }
struct Pointer {
    kind: PointerKind, button: PointerButton, x: f64, y: f64,
    modifiers: Modifiers, click_count: u8,
}
struct Wheel { x: f64, y: f64, delta_x: f64, delta_y: f64, modifiers: Modifiers }
struct Key { key: String, code: String, pressed: bool, repeat: bool, modifiers: Modifiers }
enum Ime {
    Start, Update { text: String, selection_start: u32, selection_end: u32 },
    Commit(String), End,
}
struct Find { query: String, forward: bool, match_case: bool, find_next: bool }
enum ResourceReply {
    Resolved { request: u64, resource: Resource },
    Data { request: u64, resource: u64, offset: u64, bytes: Vec<u8> },
    Released { resource: u64 }, Denied { request: u64, reason: String },
}
enum CommandKind {
    Open { source: Resource, viewport: Viewport, policy: Policy },
    Resize(Viewport), Pointer(Pointer), Wheel(Wheel), Key(Key), Text(String), Ime(Ime),
    Focus(bool), Find(Find), Zoom(f64), Copy, Policy(Policy),
    ResourceReply(ResourceReply), FrameAck { frame_sequence: u64 }, Close,
}
struct State { title: String, url: String, loading: bool, editable: bool, zoom: f64 }
struct FindResult { request_sequence: u64, matches: u32, active: u32, complete: bool }
enum HostShortcut {
    Close, Find, FindNext, FindPrevious, CycleNext, CyclePrevious,
    ZoomIn, ZoomOut, ZoomReset, Copy, Backspace, AltLeft, AltRight,
}
enum ErrorCode {
    InvalidRequest, Denied, MissingResource, ResourceChanged, Unsupported, Runtime, BudgetExceeded,
}
enum EventKind {
    Ready, Frame(Frame), State(State), FindResult(FindResult), Selection(String),
    ResourceResolve { request: u64, base: u64, url: String },
    ResourceRead { request: u64, resource: u64, offset: u64, length: u32 },
    ResourceRelease { resource: u64 }, HostShortcut(HostShortcut),
    Error { code: ErrorCode, message: String }, Closed,
}
fn read_message(reader: &mut impl std::io::Read) -> std::io::Result<Message>;
fn write_message(writer: &mut impl std::io::Write, message: &Message) -> std::io::Result<()>;
fn rgba_length(width: u32, height: u32) -> std::io::Result<usize>;
fn validate_read(offset: u64, length: u32) -> std::io::Result<()>;
```

Fields are public. Header identities, viewport revisions, resource/request IDs,
acknowledged frame sequences and find request sequences are nonzero. Each direction
uses strictly increasing sequences within one document. Sequence gaps are valid;
wraparound is forbidden. `acknowledged_input = 0` means no input incorporated yet.
Each process owns one viewer. `Open` starts one captured document, and a new file
uses a new document generation and default permissions. Transport validation is
stateless; the SDK session helper enforces generation/sequence/surface freshness.

Viewport dimensions and pointer coordinates use canvas-local physical pixels.
`scale` is physical pixels per CSS pixel, finite in 0.25..=8.0. Pointer coordinates
and wheel deltas must be finite; negative positions permit captured drags. Wheel
deltas use physical pixels. Pointer button transitions require a non-None button;
click counts are 0..=3. `Key.key` and `Key.code` use nonempty W3C UI Events names.
Native key/wheel conversion belongs in separate platform adapters. IME selection
offsets count UTF-8 bytes, must be ordered, and must be character boundaries.
Empty Find clears matches. Zoom is absolute, finite and in 0.1..=10.0. Active find
matches are one-based; zero denotes no active match. Escape/Find/file-cycle
requests are explicit `HostShortcut` values; editable-focus arbitration belongs
inside the browser. `State.editable` is informational.

Frames own tightly packed premultiplied sRGB RGBA8 bytes, no borrowed paint buffer.
Frame dimensions must match the current viewport and `viewport_revision`; their
header sequence identifies the frame for `FrameAck`. Acknowledgement transfers
permission to release that retained frame, including a frame dropped by the host.
An old generation/revision must never be presented. Each endpoint releases all
remaining owned resources/frames on Close, Closed, EOF or protocol/I/O error.

`Resource` is a read-only opaque capability. Lengths/offsets are u64 with no
whole-source ceiling. MIME text is nonempty and contains no control characters.
Reads request 1..=256 KiB, reject offset overflow and are checked against the
captured resource length by its owner. Data may be shorter than requested at EOF,
including empty. Every reply correlates with a pending request; the host checks
resource/range ownership. Release removes the capability after outstanding reads
settle; the host acknowledges with Released. URL strings request broker resolution
only; they never grant native paths or authority. The host validates scheme,
synthetic origin, scope containment, encoding, links and source revision.

Policy defaults to offline and scripts disabled. Version one rejects scripts
without network. Switching offline must disable scripts in the same policy.
Permissions reset for each different file. No native paths, handles, UI objects,
CEF values or executable callbacks cross this API. This protocol is not a process
sandbox: the provider/host must enforce browser sandboxing and request policy.

## Optional physical-key host arbitration (API/SDK 0.2)

The Rust package version is 0.2 because adding enum variants changes exhaustive
source matches. The wire remains MWV1 version 1, and every pre-existing command,
event and shortcut encoding is unchanged. New HostShortcut discriminants are
Backspace=10, AltLeft=11 and AltRight=12.

A host that can decode these tags explicitly launches the provider with
`MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS=physical-v1` in its cleared process environment.
The API exposes `HOST_SHORTCUTS_ENV` and `HOST_SHORTCUTS_PHYSICAL_V1` for that
contract. This value must never be copied from an ambient parent environment.
Providers emit the new tags only for this exact opt-in; absence or an unknown
value keeps baseline v1 traffic. Thus old hosts can run new providers, and old
providers can run under new hosts, with their existing keyboard behavior.
Manifest schema/protocol 1 remains unchanged and needs no extra field.

The signals describe physical keys, not authorized file operations. Chromium
arbitrates its live editable focus first. The host resolves the current configured
binding only while viewer content still owns focus; delayed replies cannot escape
into search editing. Unbound new keys perform no file action and are not replayed
into the browser arbitration loop. Plain/Shift+Space retain their existing tags
and unbound page-scroll behavior. Repeated new navigation keys are consumed
without repeating file changes; editable fields retain normal browser key repeat.

## MWV1 version 1 wire

All integers and IEEE-754 f64 values use little endian. The 16-byte fixed prefix is
`MWV1`, u16 version 1, u8 direction (0 command, 1 event), u8 kind tag, u32 control
length, u32 raw length. The control section begins with Header's three u64 values,
then fields in the declaration order above. Strings are u32 byte length then UTF-8
bytes. Booleans are exactly 0 or 1. Modifiers are one u8: Shift bit0, Control bit1,
Alt bit2, Meta bit3; other bits are invalid. Nested enums use one u8 discriminant
in declaration order starting at zero. All enums reject unknown tags.

Command tags 1..=15 and event tags 1..=11 follow their declaration order above.
ResourceReply Data's `bytes` and Frame's `rgba` are the raw section; no vector
length is duplicated in control. All other variants require zero raw length.
Control (including Header) is 24..=65,536 bytes. Data raw length is at most
262,144 bytes. Frame raw length must exactly equal validated width*height*4, at
most 67,108,864 bytes, 16,777,216 pixels, and each edge 1..=8192. Source read
offset+length must not overflow. Control is validated before allocating raw data;
invalid direction/tag/raw budgets are rejected from the fixed prefix alone.
Trailing control bytes, nonfinite numbers, malformed UTF-8, invalid nested tags,
dimensions and truncated messages fail. Multiple messages may follow directly.

`write_message` validates before writing any byte, streams raw bytes directly and
flushes. I/O failures may write a partial packet. All protocol/I/O errors are
terminal: close the stream and release session ownership; do not resynchronize.
EOF is reported as UnexpectedEof. Neither endpoint should log document/selection
text or resource URL secrets.

## SDK ownership helpers

`midnight_web_viewer_sdk::Transport<R: Read, W: Write>` takes ownership with
`new(reader, writer)`, sends with `send(&Message)`, receives with `receive()`, and
returns streams with `into_inner()`. Every error poisons the transport; later
operations return BrokenPipe. Run blocking I/O on backend workers.

`Session::new(session, document)` and `accept(&Message)` validate both directions
transactionally. Open precedes normal traffic; Ready precedes frames. Resize
revisions increase, frame revision/dimensions match the current viewport, frame
input acknowledgements cannot refer to future commands, and closing rejects new
frames/input. `is_closed()` becomes true on Closed. Each new document gets a new
Session. Failed transitions leave state unchanged. The broker must separately
validate pending resource requests, scope and range ownership.

`CommandQueue::new(capacity)` accepts 1..=64 commands. `push(Command)` returns
`Result<Option<Command>, Command>`: displaced ownership on adjacent Resize/Move
coalescing, or original ownership on invalid/full input. Only adjacent commands
from the same document with increasing sequence coalesce. `pop()` transfers the
oldest command. Keys, buttons, resource replies and Close remain FIFO.

`FrameWindow::new(capacity)` accepts 1..=2 frames (at most 128 MiB raw retained).
`retain(Event)` returns `Result<(), Event>`, retaining only valid Frame events in
increasing sequence for the same document. `acknowledge(sequence)` returns exactly
the matching owned event; unknown/duplicate acknowledgements change nothing.
`clear()` transfers all remaining events for backend cleanup. These helpers do
not create threads, start processes, open files, interpret URLs or isolate CEF.

## Build and offline release

Set CARGO_HOME and RUSTUP_HOME beneath the repository's `.tools`, and use
`cargo test --manifest-path web-viewer-api/Cargo.toml --workspace --target-dir
_build/html-viewer/api-target` (also strict Clippy and rustfmt). No root workspace
build is required. `pwsh web-viewer-api/scripts/package.ps1` on Windows or
`python web-viewer-api/scripts/package.py` on Linux creates a fresh
immutable `_build/html-viewer/sdk/release-*/` registry bundle. It prints its Cargo
config path; pass that path as `cargo --config PATH` when building an independent
provider that declares `version = "=0.2.0", registry = "midnight"`. The bundle
contains normalized Cargo packages, SHA-256 index entries and artifacts.json.
Source path dependencies never enter the released manifests. This only packages
the public API/SDK and is not evidence of native Chromium execution.
