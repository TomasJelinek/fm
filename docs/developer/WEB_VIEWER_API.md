# Interactive HTML viewer

The optional HTML provider uses Chromium Embedded Framework in a separate process.
It renders owned premultiplied RGBA frames into the Slint viewer window;
it does not open an external browser or embed CEF types in the application's API.
The authoritative protocol is the independent
[web-viewer API/SDK](../../web-viewer-api/README.md). The implementation is an
[independent Cargo project](../../plugins/midnight-viewer-html/Cargo.toml), with its
own dependencies, lockfile and tests and no private application crate dependency.

Install `viewers/<package>/web-viewer.json` and the complete matching runtime next
to the executable in its validated private installation. Schema/protocol 1 includes
`id`, `name`, `version`, `target`, a simple executable basename and default
`extensions`. HTML, HTM and XHTML are defaults. Viewer associations in
Options can override suffixes and priority. Removing the package removes this
provider; Alt+F3 still selects the basic text viewer. SVG retains image routing.

`midnight-web-viewer-host` validates package identity and dependencies, owns the
browser supervisor and brokers reads on the session's backend actor thread, the
same thread that forwards input and acknowledges frames (never a UI thread). Source
lengths and offsets are `u64`; reads are at most 256 KiB. Resource IDs grant access
only to captured content. Native source labels, archive paths and temporary
filenames do not grant authority. The optional `ContentResources` capability
captures relative assets against pinned directories or an archive mount. Relative
URLs resolve lexically like a browser: `../img/logo.png` from `css/site.css` names
`img/logo.png`, while any path that would leave the captured scope root is denied.
Native descendants reject links and revalidate held ancestors/source revisions.
Archive descendants use mount/node identities and raw names. Generated documents
have no implicit scope.

Every message has a session, document generation and sequence. Resize and mouse
move coalesce only when adjacent; key/button transitions preserve ordering. The
input queue holds 64 commands. Rather than drop a key or button release and leave
the page with stuck input, a command that does not fit stops the browser session:
`Session::send` refuses it and every later command, and pending output is replaced by
one terminal error (*Browser input overflowed while the page was busy; reopen the
document.*) that `Session::drain` delivers. The viewer engine treats a refused send as
a stopped session and handles it once. Unless the newest browser output it drained is
an error, which stays visible as the reason, it reports *Browser closed; reopen the
document.* through its error path (an opening without a frame fails with it). The
terminal error the host publishes for the refused command is drained in the same loop
pass and replaces that report; later refusals are silent. A browser that closes the
session itself is reported the same way without waiting for input: the host wakes the
viewer and `Session::drain` delivers the protocol's Closed event after the pending
output, while a session the viewer closes publishes no Closed event. `Session::drain`
returns the queued events and the one retained frame in the browser's order (document,
then sequence), so an error the browser sent after a frame is drained after that frame
and stays the newest output and the reported reason. The host retains at most one
pending painted frame. Replaced document generations and
viewport revisions cannot publish stale pixels. Transport budgets bound messages
and viewport memory, not complete file sizes. Frame acknowledgements and close/EOF
release ownership. The host acknowledges and drops a frame of an older viewport
revision: a Resize it sends can cross a frame the browser has already painted. The SDK
`Session` validator checks messages in the provider's transport order, where frames that
follow a Resize carry its revision. No UI callback performs file reads, discovery or
process waits. The host session actor and its watchdog block until a command, browser
output, a stop or a watchdog deadline (5 s after the latest write to the browser
started, which ends a write still blocked then, and 30 s until Ready), so an idle open
document causes no periodic host wakeups.

The application's opt-in `Service::open_with_layout` path uses app-private
`midnight_core::viewer` messages. The UI admits the backend session before creating
the native window so capture can start immediately. If creating or showing the window
fails, dropping the session cancels its worker and retires its capture.
`ViewerOutput::LayoutSelected` announces a
nonzero token, selected mode and `content_generation` before the backend starts
a native plugin session or HTML browser session. An image or PDF provider starts
decoding at selection instead, overlapping native window creation, because decoding
needs no viewport; its first viewport is still requested only after the
acknowledgement. A selection that is invalidated retires that provider like a
closing image. The UI applies the selected mode/chrome,
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
explicit-initial-geometry behavior.

HTML starts offline with JavaScript running (web viewer API 0.4). The host opens the
first document with the consumer's policy (`Session::open_with_options`,
`OpenOptions::policy`); the viewer engine opens each page offline with scripts unless the
user turned JavaScript off in that window, and a Reload keeps the page's choices.
Offline with scripts the provider serves the document sandboxed without
`allow-same-origin` (`sandbox allow-scripts allow-popups`, an opaque origin) under a CSP
that allows assets only from its private origin, `data:` and `blob:`, with
`connect-src 'none'` and no worker, frame, child, object, manifest or form. Every asset
is cross-origin to the page, so its scripts read none; fonts and module scripts get
`Access-Control-Allow-Origin: *` to load. Every document's request context has network
prediction off (`net.network_prediction_options` 2: no preconnect, DNS prefetch,
prefetch or prerender, which bypass request interception), every response carries
`X-DNS-Prefetch-Control: off`, and before any script of an offline page runs, the
renderer replaces WebRTC and pixel readback (canvas `getImageData`, `toDataURL`, `toBlob`,
`captureStream`, `OffscreenCanvas`, WebCodecs) in each of its contexts with fixed values a
script cannot redefine: Chromium marks no intercepted response cross-origin, so a canvas
would not be tainted by a local image (`plugins/midnight-viewer-html/src/native/handlers.rs`).
The opt-in native test `tests/native_browser.rs` checks that fetch, XHR, WebSocket,
EventSource, beacons, images, module imports, workers, service workers, preconnect,
prefetch, speculation rules, a script navigation and WebRTC reach neither a loopback TCP
nor a UDP listener, that `file://` is refused and that no window opens. Explicit online
mode grants only the captured main document, embedded data and remote resources;
relative local assets are unavailable, and the UI warns that scripts can read and
transmit the main document. Each transition revokes resource IDs, clears pending
presentation, and recreates the Chromium browser/context and origin. Online permission
resets on a different file. The provider serves the captured document at its folder's
URL, 16 synthetic folder levels below `https://mewv-<generation>.invalid/mewv-document/`
(see [link routing](#link-routing-apisdk-03) for why); offline relative assets and,
without host link routing, relative navigation to other captured files in scope resolve
below it. An asset reference that climbs above it (`../css/site.css`, as rustdoc, MkDocs,
Sphinx and Javadoc pages use) is requested as that `../` reference (at most 16), which the
host resolves within the folders its scope reaches (`Resources::ancestor_levels`; the
application's directory scope reaches up to 16 folders above the document's, each pinned
by its native path when an asset first names it); the browser itself navigates to no
document above it, and a root-relative reference is denied rather than substituted. Asset
MIME types follow the extension, also before a saved page's `.download` suffix, and a
stylesheet of unknown type is served as CSS. `State.url` reports `https://mewv-<generation>.invalid/mewv-document/`,
as before the synthetic levels existed, and `State.editable` is always false. Cookies
are disabled entirely. File URLs, external protocols, downloads, popups, file pickers,
device permissions and extension loading are denied; with host link routing a popup
the user opened becomes a `LinkActivated` event instead, and no window opens either
way. There is no renderer-facing application RPC or remote debugging port.
Browser request policy is not represented as an OS-level network firewall.

Windows runs the matching official CEF bootstrap with the same-basename client
DLL and the exact five-argument `RunWinMain` sandbox ABI. It passes the sandbox
state to subprocess dispatch and initialization. A host kill-on-close job owns
up to 32 processes and 2 GiB aggregate committed memory. Browser profile files live
in a fresh private session directory. After the browser tree is terminated, the host
waits up to two seconds for every browser process to exit (Windows until the job
reports no active process, Linux until the session's cgroup is empty) and then removes
the directory. A directory that could not be removed then, or one left by a crashed
session, is removed when a later application process starts its first browser. The
user's normal browser profile is never used. Linux has its own launcher and
requires Linux 5.14+ with cgroup v2 and an existing writable user delegation with
`memory` and `pids` already enabled. It creates an owned child cgroup with 2 GiB
aggregate memory, no swap, and at most 256 tasks (threads count on Linux), joining
it before executing Chromium. An eligible ancestor is detected automatically;
`MIDNIGHT_WEB_VIEWER_CGROUP_PARENT` may specify an explicit delegated parent.
The application does not enable controllers or change its own cgroup membership.
Tree-wide cancellation uses `cgroup.kill`. These limits supplement Chromium's
sandbox. Failure to establish containment is an error, never an unsandboxed
fallback. Native Linux rendering and containment were verified on Ubuntu 24.04
under WSLg XWayland ([Linux record](../verification/2026-11-10-html-linux.md)). That
does not cover AppArmor's user-namespace restriction, which WSLg does not apply: on Ubuntu 23.10+ desktops the browser needs the
`midnight-explorer-html-viewer` profile that `scripts/install-html-viewer-apparmor.sh`
installs, or a setuid-root `chrome-sandbox` (see
[system requirements](../SYSTEM_REQUIREMENTS.md)). The Linux launcher checks for this
before the browser can use its sandbox. When `kernel.apparmor_restrict_unprivileged_userns`
is 1 and `chrome-sandbox` is not setuid root, it reads the started browser's AppArmor
label; a browser without any profile, where an `unshare --user --map-root-user` probe
in a child process is refused too, is stopped at once. `browser::spawn` then returns
`browser::SandboxUnavailable`, whose message names the installer of that installation
when the script and every folder above it are owned by root and not writable by group
or others (otherwise it asks for an administrator and points to the system
requirements), and the host shows that message instead of a transport error. It also
carries the typed `browser::SandboxRefusal::UserNamespaces { executable }`: the host
session records it before it publishes that terminal error, and
`Session::sandbox_refusal` returns it to the viewer worker that drained the error, so
nothing interprets the message text. The worker then publishes
`ViewerOutput::SandboxNotice` (shown in the page's place) and, where
`midnight_elevation::SandboxProfile::for_browser` finds a root-owned `/usr/bin/pkexec`
and an installation path AppArmor can name, offers Install…. `ViewerInput::InstallSandboxPermission`
runs `SandboxProfile::install` on its own thread (polkit, one constant root command; see
[security](../../SECURITY.md)), shows "Waiting for authorization…", and opens the
document again after success. A dismissed or refused authorization (pkexec 126/127) and
other failures keep Install… available; a pkexec that became untrusted leaves only the
manual instructions; closing the window cancels a pending prompt. A profile the browser
gets from the installed policy or inherits from the application allows the namespaces,
so it is never reported. Other launch failures keep the generic error and offer nothing.
A browser that stops before it reports Ready may not have started at all: once the
watchdog has waited for its process tree, the host session asks the platform why
(`browser::Process::startup_failure`). On Linux, when the browser ended with the dynamic
loader's status 127, that reads the ELF dynamic sections of the viewer executable and of
every library it needs, looked up as glibc's loader looks them up (DT_RPATH chain or
DT_RUNPATH, `/etc/ld.so.cache`, default folders), without executing anything, and returns
`browser::LibrariesMissing`, which names the missing libraries and the command that
installs them for the running distribution family (apt package names for
Debian/Ubuntu, soname capabilities for dnf and zypper; in a source checkout also
`scripts/setup-linux.sh --install`). The host shows that message instead of "Chromium
transport stopped: failed to fill whole buffer"; it is no sandbox refusal and offers
nothing. A browser that reported Ready keeps its own error, and Windows reports no such
reason.
IME composition/commit pass CEF's required invalid replacement range (a synthetic
commit is covered by the opt-in native fixture); physical IME input, native Chromium
Wayland and other display/system policies are not verified, and Windows measurements do
not establish Linux frame pacing. The host session is event-driven (verified with real
Chromium on Windows only). While a document is open, the provider still waits at most 2 ms for a command
before it pumps CEF again. On Linux that is about every 2 ms (not measured). On an idle
Windows document it is about 64 times per second, because the wait rounds up to the
timer period (15.6 ms when measured). CEF's external message pump measured only about 3x
fewer idle pumps on Windows and is not used
([idle-wakeup record](../verification/2026-11-18-html-idle-wakeups.md)).

Viewer controls reuse Escape, Ctrl+F, Find next/previous and zoom. Every search goes
to Chromium as a follow-up (`Find.find_next`): a new native session only highlights
the matches and reports none active, while a follow-up, also with another query or
case, activates the nearest match on from the active one, as the text viewer searches
on from where it stands. From Chromium's reply on, the status line follows the
page's state with "Match 2 of 3", "1 match" or "3 matches" while no match is active,
or "No matches" ("Page title · Offline · JavaScript on · Match 2 of 3", or for an Office
preview "Office preview · Offline · JavaScript off · Match 2 of 3"), whether the page's next
frame comes before or after the reply. Pointer, keyboard
and IME input target the displayed content generation. Space, Backspace and
Alt+Left/Right are arbitrated inside Chromium using its actual editable focus, so
typing into an HTML input does not change the viewed file (nor go Back or Forward). Clipboard text returns through the host's explicit
clipboard command.

Find, Copy and zoom use the user key resolver before browser forwarding.
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
it. MWV1 packets and manifests stay compatible. New physical-key
replies are ignored after viewer content loses focus; unbound keys are not replayed. Copy preserves an explicit
overflow state: a selection exceeding 48 KiB of UTF-8 reports a budget error
without publishing a partial or empty clipboard replacement. A later smaller
selection remains copyable.

Wheel input scrolls a page as far and as fast as a text document. The viewer glides a
wheel notch through the shared wheel motion (the text viewer's notch distance, 100
logical px at Windows' default three lines, over 200 ms) and sends one physical-pixel
`Wheel` per presented frame; touchpad pixels, X11 finger lines (`LINE_PIXELS`, 100 px
per line) and Linux momentum go through at once, one `Wheel` per report or frame. The provider starts Chromium with `disable-smooth-scrolling`, so it applies
each wheel event at once instead of animating it again (CEF's windowless wheel events
are not precise deltas, which Chromium otherwise eases over about 170 ms); keyboard
scrolling jumps at once too. Chromium reports no scroll position: once the page has
painted since a glide or touchpad stream began (or a second has passed), the glide or
momentum stops 120 ms after the page stops painting new frames, at its top or bottom
edge. A wheel with
Ctrl, Alt or Super acts once, unglided, with a 40 px line, so Chromium's Ctrl+wheel
zoom keeps its rate. See [HTML pages](SCROLLING.md#html-pages); the timing was measured
on Linux only.

The provider's own dev package uses `opt-level = 2` for its image conversion and
composition loops. Native test measurements and platform limitations are recorded in
the [HTML viewer record](../verification/2026-11-10-html-viewer.md).

See the [design](../superpowers/specs/2026-11-09-html-viewer-design.md),
[implementation plan](../superpowers/plans/2026-11-09-html-viewer.md), and the
provider's [dependency record](../../plugins/midnight-viewer-html/THIRD_PARTY.md).

The provider permits up to 60 painted frames per second while the host still keeps
only the newest pending frame. This does not require a GPU or guarantee 60 fps.
Measured Windows native and software latency and remaining limitations are in the
[frame-pacing report](../verification/2026-11-12-html-frame-pacing.md).

## Versions and compatibility

The wire stays MWV1 version 1 (`PROTOCOL_VERSION` 1) and the manifest stays schema 1 /
protocol 1. Each API/SDK release adds Rust enum variants, so its Rust version changes,
and each new tag is sent only after an exact launch opt-in that the host adds to the
browser's cleared environment.

| API/SDK | Adds | Launch opt-in |
| --- | --- | --- |
| 0.1 | Commands 1–15, events 1–11, host shortcuts 0–9 | — |
| 0.2 | `HostShortcut` Backspace = 10, AltLeft = 11, AltRight = 12 | `MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS=physical-v1` |
| 0.3 | Events `LinkHover` = 12 and `LinkActivated` = 13 | `MIDNIGHT_WEB_VIEWER_LINKS=host-v1` |
| 0.4 | Event `Cursor` = 14; `Policy { network: false, scripts: true }` is valid | `MIDNIGHT_WEB_VIEWER_CURSOR=cursor-v1` (the cursor only) |

A provider of API 0.3 or earlier refuses an offline policy with scripts as an invalid
message and stops, so the 0.4 host sends one only to the bundled 0.4 provider it ships
with; a consumer that may face an older provider opens with the default policy. The
application host always adds the cursor opt-in; a 0.3 provider ignores it, and the window
then shows the hand over a link from `LinkHover` alone.

| Host | 0.2 provider | 0.3 provider |
| --- | --- | --- |
| 0.2 host | Links load in the browser | No link opt-in: links load in the browser, no new tags |
| 0.3 host, `LinkRouting::Browser` | Links load in the browser | No link opt-in: as with a 0.2 provider |
| 0.3 host, `LinkRouting::Host` | The unknown variable is ignored: links load in the browser, no link events | Link events; the browser never leaves the document by itself |

The application host always adds the physical-v1 opt-in. Every host built against
0.3 decodes tags 12 and 13; a host built against 0.2 would refuse them, which is why
the provider sends them only for the exact link opt-in. Every host built against 0.4
decodes tag 14 too; the provider sends it only for the exact cursor opt-in.

## Link routing (API/SDK 0.3)

`midnight_web_viewer_host::LinkRouting` selects who follows a document's links when
a session opens: `Session::open_with_links(provider, source, resources, viewport,
wake, ceiling, links)`. `Session::open_with_policy_ceiling` is
`open_with_links(…, LinkRouting::Browser)`. With `LinkRouting::Browser` the host
launches the provider as before and links inside the captured document's folder load
in the browser through the provider's navigation preparation, which the Office
preview's generated `office/v1/…` paging links rely on: a main-frame GET target of at
most 1 MiB is admitted completely through the captured resource broker before the
browser replaces the document, within a 30-second deadline (`BODY_BYTES`, `DEADLINE` in
the provider's `navigation.rs`). A provider that sends a
link event anyway ends the session ("Browser sent a link event without the
link-routing launch opt-in").

With `LinkRouting::Host` the host adds `MIDNIGHT_WEB_VIEWER_LINKS=host-v1` to the
browser's cleared environment (both platforms' launch allowlists carry it) and passes
`LinkHover` and `LinkActivated` events to the consumer unchanged, in the browser's
order, like the other events of the current document. Their targets were already
validated when the host read them: at most 8,192 bytes of UTF-8, no control characters,
and nonempty for `LinkActivated`; anything else is a protocol error that ends the
session. The host's output queue holds 64 events and a full queue ends the session, so
the provider sends a hover only when its target changes.

Under the opt-in the provider cancels every navigation of the main frame to another
document and decides in CEF's callbacks:

- `OnStatusMessage` gives Chromium's status URL: the link under the pointer (without
  one, the keyboard-focused link), empty when there is none; a changed target becomes
  `LinkHover`. A link whose target cannot be expressed clears the hover target and sends
  a `Denied` notice.
- With Chromium's user-gesture flag, `OnBeforeBrowse` (a click or Enter on a focused
  link, neither a redirect nor a client redirect), `OnOpenURLFromTab` (a middle, Ctrl or
  Shift click) and `OnBeforePopup` (`target=_blank`, `window.open`) become
  `LinkActivated`. One activation is sent per 500 ms, so a double click or a script
  repeating one gesture's navigation reaches the host once.
- The browser's first load of the document, reloads (F5, Ctrl+R) and fragment
  navigations within the current document stay inside the browser.
- Navigations without a user gesture (scripts), server redirects, client redirects
  (Chromium's `TT_CLIENT_REDIRECT_FLAG`: a `<meta http-equiv="refresh">` or a script
  redirect, which Chromium may give the gesture of a recent click or key press), form
  submissions (`TT_FORM_SUBMIT`, GET or POST, also when the user sends the form), popups
  without a gesture and links whose target cannot be expressed are refused with an
  `Error { code: Denied }` notice, at most one per second.
- Limit: with scripts enabled, a navigation a script starts inside a click or key
  handler carries that gesture. Unless Chromium marks it as a client redirect, the
  provider cannot tell it from a followed link, and it becomes `LinkActivated`.

A target outside the private origin is the absolute URL exactly as Chromium resolved
it (it starts with a scheme, whatever the scheme). A target on the private origin is a
reference relative to the document's folder without a scheme, keeping `?query#fragment`:
`sub/other.html#part`, `../README.md`, `x.html?q=1`. The document is served at its
folder's URL, so a link to the document itself or to its folder is `./`; a first
segment with a colon or an empty first segment is also written after `./`, so a
relative target never reads as a scheme or a root path. Another generation's private
origin is no folder of this document; its URLs are passed on like any other URL.

Chromium resolves `../` itself and stops at the origin's root. Below one
`mewv-document` folder, `../../a.html` and `../../../a.html` would both become the
origin root's `a.html`, and the host could not tell which file a link names. The
provider therefore serves the document 16 synthetic folder levels deeper
(`mewv-level-1/` … `mewv-level-16/`, `plugins/midnight-viewer-html/src/policy.rs`): up
to 16 `../` segments stay apart and are reported as `../` segments. A link with 17 or
more cannot be told apart from one with more and is refused with a notice that it
leads above the 16 folder levels the viewer can tell apart; so is a root-relative link.
The level names are distinctive, not reserved: a real folder of the document's parent
named exactly like the innermost level would alias the document's own folder. Assets
keep their folder-relative `ResourceResolve` URLs, and assets above the document's
folder are denied.

The application's viewer engine (`crates/midnight-viewer-engine/src/worker_web.rs`)
opens ordinary HTML documents with `LinkRouting::Host` and the Office preview with
`LinkRouting::Browser`. A `LinkHover` target is described on the status line, with the
pointer cursor, and a `LinkActivated` target is followed by the same host link policy
as a Markdown link ([links](VIEWER_PREFERENCES.md#links)): a relative target resolves
against the document's folder, and another generation's private origin (a `.invalid`
host) is refused. Offline with scripts, an activated target with a scheme (a web or mail
address) is followed only when it equals the newest `LinkHover` target: a navigation a
script starts in a click or key handler carries the gesture, and without this check it
would carry the page's data to the user's browser. A refusal or result stays on the
status line as the browser's notice until the next user action.

## The small browser (application)

The HTML row of the viewer window has Back, Forward and Reload buttons before the Online
and JavaScript boxes; a narrow window leaves the three buttons out and keeps the boxes and
the zoom, while the keys and mouse buttons below still work. Back and Forward
(`viewer.link-back`, Alt+Shift+Left; `viewer.link-forward`, Alt+Shift+Right) go through
the documents opened from links in that window (`ViewerLinkInput::Back`/`Forward`); the
engine reports whether each has a document to go to (`ViewerOutput::LinkHistory`), which
enables the buttons. Reload (`ViewerLinkInput::Reload`, `viewer.reload`, F5 and Ctrl+R)
captures the shown file again and reopens it, keeping the history and an HTML page's
Online and JavaScript choices for that file only. A text page reopens where it was; an
HTML page starts at its top, because Chromium reports no scroll position
(`worker_links.rs`, `reload`). After Chromium's editable-focus arbitration, Alt+Left and
Alt+Right go Back and Forward while there is a page in that direction and otherwise keep
their bound file action (`web_input::browser_history`); the mouse's Back and Forward
buttons go through the history. The status line shows the page's title (the file name
until it has one) or "Loading…", then the policy, a notice and the Find result; the row
shows "Loading…" too (`ViewerOutput::WebState::loading`). The page's cursor
(`EventKind::Cursor`, from CEF's `OnCursorChange`) reaches the window as
`ViewerOutput::WebCursor` and is set on the native window through winit while the pointer
is over the page, on Windows and Linux alike
(`crates/midnight-viewer-ui/src/web_cursor.rs`): Slint never sees the pointer there,
because those events go to the browser. Leaving the page restores the window's default
cursor.
