# Interactive HTML viewer

The optional HTML provider uses Chromium Embedded Framework in a separate process.
It renders owned premultiplied RGBA frames into the existing Slint viewer window;
it does not open an external browser or embed CEF types in the application's API.
The authoritative protocol is the independent
[web-viewer API/SDK](../../web-viewer-api/README.md). The implementation is an
[independent Cargo project](../../plugins/midnight-viewer-html/Cargo.toml), with its
own dependencies, lockfile and tests and no private application crate dependency.

Install `viewers/<package>/web-viewer.json` and the complete matching runtime next
to the executable in its validated private installation. Schema/protocol 1 includes
`id`, `name`, `version`, `target`, a simple executable basename and default
`extensions`. HTML, HTM and XHTML are defaults. Existing viewer associations in
Options can override suffixes and priority. Removing the package removes this
provider; Alt+F3 still selects the basic text viewer. SVG retains image routing.

`midnight-web-viewer-host` validates package identity and dependencies, owns the
browser supervisor and brokers reads on background workers. Source lengths and
offsets are `u64`; reads are at most 256 KiB. Resource IDs grant access only to
captured content. Native source labels, archive paths and temporary filenames do
not grant authority. The optional `ContentResources` capability captures relative
assets against pinned directories or an archive mount. Native descendants reject
links and revalidate held ancestors/source revisions. Archive descendants use
mount/node identities and raw names. Generated documents have no implicit scope.

Every message has a session, document generation and sequence. Resize and mouse
move coalesce only when adjacent; key/button transitions preserve ordering. The
host retains at most one pending painted frame. Replaced document generations and
viewport revisions cannot publish stale pixels. Transport budgets bound messages
and viewport memory, not complete file sizes. Frame acknowledgements and close/EOF
release ownership. No UI callback performs file reads, discovery or process waits.

The application's opt-in `Service::open_with_layout` path uses app-private
`midnight_core::viewer` messages. `ViewerOutput::LayoutSelected` announces a
nonzero token, selected mode and `content_generation` before the backend starts
the viewer transport or HTML browser session. The UI applies the selected mode/chrome,
settles layout, measures the actual canvas and sends `ViewerInput::LayoutReady`
with that token and `ViewerGeometry`. It acknowledges unchanged dimensions too,
retries refused enqueues and suppresses redundant Resize until acceptance.

The content generation fences presentation frames; it is separate from the native
plugin session generation. Resize preserves the pending token; Close, file cycling,
mode replacement or fallback invalidate it. Stale/duplicate acknowledgements
cannot start content, and the UI rejects frames below the announced generation
floor before they can rewrite chrome. Layout metadata and its acknowledgement do
not make opening ready: a real frame is required. The public plugin API/wire
protocol is unchanged; fixed-viewport `Service::open` consumers retain their
existing explicit-initial-geometry behavior.

HTML starts offline with JavaScript disabled. Explicit online mode grants only
the captured main document, embedded data and remote resources; relative local
assets are unavailable. Enabling JavaScript requires online mode and the UI warns
that scripts can read/transmit the main document. Each transition revokes resource
IDs, clears pending presentation, and recreates the Chromium browser/context and
origin. Permissions reset on a different file. File URLs, external protocols,
downloads, popups, file pickers, device permissions and extension loading are
denied. There is no renderer-facing application RPC or remote debugging port.
Browser request policy is not represented as an OS-level network firewall.

Windows runs the matching official CEF bootstrap with the same-basename client
DLL and the exact five-argument `RunWinMain` sandbox ABI. It passes the sandbox
state to subprocess dispatch and initialization. A host kill-on-close job owns
up to 32 processes and 2 GiB aggregate committed memory. Browser profile files live
in a fresh private session directory and are removed after process cleanup; the
user's normal browser profile is never used. Linux has its own launcher and
requires Linux 5.14+ with cgroup v2 and an existing writable user delegation with
`memory` and `pids` already enabled. It creates an owned child cgroup with 2 GiB
aggregate memory, no swap, and at most 256 tasks (threads count on Linux), joining
it before executing Chromium. An eligible ancestor is detected automatically;
`MIDNIGHT_WEB_VIEWER_CGROUP_PARENT` may specify an explicit delegated parent.
The application does not enable controllers or change its own cgroup membership.
Tree-wide cancellation uses `cgroup.kill`. These limits supplement Chromium's
sandbox. Failure to establish containment is an error, never an unsandboxed
fallback. Native Linux rendering and containment were qualified on Ubuntu 24.04
under WSLg XWayland; see the [Linux report](../verification/2026-11-10-html-linux.md).
Physical IME, native Chromium Wayland and other display/system policies still need
their own qualification. Windows measurements do not establish Linux frame pacing.

Viewer controls reuse Escape, Ctrl+F, Find next/previous and zoom. Pointer, keyboard
and IME input target the displayed content generation. Space, Backspace and
Alt+Left/Right are arbitrated inside Chromium using its actual editable focus, so
typing into an HTML input does not change the viewed file. Clipboard text returns through the host's explicit
clipboard command.

Find, Copy and zoom use the existing user key resolver before browser forwarding.
HTML file cycling is restricted to browser-arbitrated Space/Shift+Space, Backspace
and Alt+Left/Right. Assigning Previous/Next file to another chord (for example Q
or Ctrl+J) works in other viewer modes; in HTML that chord remains page input.
This includes native digit/bracket shortcuts and the software keyboard fallback.
Supporting arbitrary HTML file-navigation remaps requires a separate negotiated,
bounded generic-key candidate/receipt contract; cached editable state is unsafe.
Browser-arbitrated keys are resolved again against the current configured action;
an unbound Space scrolls instead of cycling files. Backspace/Alt arrows require
the explicit `physical-v1` host launch opt-in from public API/SDK 0.2. The host
adds it to a cleared environment, and the provider never emits new tags without
it. Existing MWV1 packets and manifests remain compatible. New physical-key
replies are ignored after viewer content loses focus; unbound keys are not replayed. Copy preserves an explicit
overflow state: a selection exceeding 48 KiB of UTF-8 reports a budget error
without publishing a partial or empty clipboard replacement. A later smaller
selection remains copyable.

The provider's own dev package uses `opt-level = 2` for its image conversion and
composition loops. Other dev dependencies and the host's iteration profile are
unchanged. Native test measurements and platform limitations are recorded in the
[verification report](../verification/2026-11-10-html-viewer.md).

See the [design](../superpowers/specs/2026-11-09-html-viewer-design.md),
[implementation plan](../superpowers/plans/2026-11-09-html-viewer.md), and the
provider's [dependency record](../../plugins/midnight-viewer-html/THIRD_PARTY.md).

The provider permits up to 60 painted frames per second while the host still keeps
only the newest pending frame. This does not require a GPU or guarantee 60 fps.
Measured Windows native and software latency and remaining limitations are in the
[frame-pacing report](../verification/2026-11-12-html-frame-pacing.md).
