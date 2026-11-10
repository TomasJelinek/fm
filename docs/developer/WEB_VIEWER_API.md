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
fallback. The Linux launcher tests pass, but native Linux Chromium rendering has
not been qualified; Windows rendering results do not establish Linux support.

Viewer controls reuse Escape, Ctrl+F, Find next/previous and zoom. Pointer, keyboard
and IME input target the displayed content generation. Space cycling is arbitrated
inside Chromium using its actual editable focus, so typing into an HTML input does
not change the viewed file. Clipboard text returns through the host's explicit
clipboard command.

Viewer shortcuts use the existing user key resolver before browser forwarding.
Browser-arbitrated Space is resolved again against the configured action; an
unbound Space scrolls instead of cycling files. Copy preserves an explicit
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
