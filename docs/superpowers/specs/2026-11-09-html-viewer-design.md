# Embedded Chromium HTML viewer

## Intent and accepted context

Add HTML viewing inside the normal viewer window. The application remains Rust,
Slint and Tokio; viewers remain optional independent plugin projects sharing only
public contracts. Windows x64 and Linux are the initial platform targets; macOS
keeps a separate future implementation boundary. Preserve configurable extension
associations, Basic View, Escape-to-close, panel file cycling, search and zoom.

The user requested Chromium, rather than opening an external browser. Existing
Markdown continues to use its semantic renderer and never becomes active HTML.
This design is an extension of the accepted viewer/plugin design. The following
implementation choices are engineering decisions, not additional user requests.

## Approaches

- Selected: CEF in a separate process, painting into the Slint viewer through a
  public interactive-surface protocol. It gives Windows and Linux Chromium and
  keeps its native dependencies outside the main application binary.
- System webviews: smaller Windows installation with WebView2, but the usual
  Linux backend is WebKit. This does not meet the common Chromium requirement.
- Native child browser windows: efficient native composition, but cross-process
  focus, shortcut handling, DPI and Wayland embedding require separate contracts.
  Keep this as a future negotiated presentation optimization.

Start with bounded CPU frames and measure actual scrolling before choosing a
platform-specific GPU sharing mechanism. Chromium may use its own GPU process.

## Independent components

- `web-viewer-api/`: independent public API/SDK workspace, no application, Slint,
  Tokio or Chromium dependency. Existing semantic ABI and raster protocols remain
  unchanged. New framing has its own magic and version.
- `plugins/midnight-viewer-html/`: independently built/tested CEF provider. It
  depends only on the published public API/SDK and its own third-party libraries.
- `midnight-web-viewer-host`: backend package discovery, supervised process tree,
  input sequencing, source/resource broker and validated frame delivery.
- Existing viewer engine/UI: generic interactive-surface routing and presentation.
  Only the backend opens files, resolves assets, launches processes or performs
  large pixel conversion/copying. UI work is limited to input and presentation.

Discovery uses an independently installed `web-viewer.json` package beneath
`viewers/`. Missing packages remove the capability; missing/invalid runtime files
produce a package diagnostic. Files being viewed never determine executable code.
Default extensions are `html`, `htm` and `xhtml`; user associations may override
them. SVG keeps the existing image-viewer association. HTA is not auto-associated.

## Public contract

Use `Command`, `Event`, `Viewport`, `Policy`, `Resource` and `Frame` value types.
Every command/event carries a session, document generation and sequence. A frame
also identifies its viewport revision and last acknowledged input. There are no
Slint objects, Rust trait objects, CEF pointers or native filesystem paths on the
wire. Invalid tags, lengths, dimensions, non-finite values and stale generations
are rejected before allocation or presentation.

Commands cover Open, Resize, Pointer, Wheel, Key, Text/IME, Focus, Find, Zoom,
Copy, Policy, resource replies, frame acknowledgement and Close. Events cover
Ready, Frame, State, FindResult, Selection, ResourceResolve/Read/Release, host
shortcut requests, errors and Closed. Input coordinates are canvas-local physical
pixels; DPI is explicit. Text is UTF-8; platform key conversion stays in separate
platform files. The browser owns CSS layout and DOM scrolling/search.

Initial transport budgets are 64 KiB of control data, 256 KiB per source read,
16 million pixels/64 MiB per frame and an 8192-pixel edge. These bound individual
messages and viewports, not source file length. Source offsets/lengths use u64.
Queues are bounded. Latest resize, pointer movement and frames may supersede older
ones; key/button ordering and close/resource ownership must remain exact.

No borrowed OnPaint buffer escapes a CEF callback. Convert/composite BGRA and popup
paints in the provider/backend, then deliver owned premultiplied RGBA frames.
Host surface generation checks prevent old paints appearing after resize, file
cycling, provider replacement or close. One provider process owns one open viewer;
CEF renderer/GPU/network children belong to its supervised process tree.

## Source and relative assets

The main document is a captured read-only source. A separate host-owned resource
scope grants the containing directory or captured archive subtree. Never infer
this scope from a display name, temporary extraction path or `CapturedFile.path()`;
provider-backed captures can have no native path at all.

Serve content under a session-private synthetic HTTPS origin. The browser receives
only opaque resource IDs, MIME information, lengths and checked byte ranges.
Relative CSS, images, fonts and permitted documents resolve within the captured
scope. URL decoding, containment, link/reparse checks and revision checks belong
to the backend. Parent escapes, foreign authorities, device paths, NULs and
unapproved schemes are rejected. Archive assets use the same captured mount and
raw member identities; generated documents without a scope cannot read siblings.
Reads stream in bounded chunks; there is no artificial whole-file size ceiling.

Resources never grant writes, shell actions, credentials or native operation
paths. A failed resource is reported as missing/denied content without retargeting
the request or replacing the main source authority.

## Content policy and isolation

Default is offline, with JavaScript disabled. This is the stated default while
the optional preference question is unanswered. Online requests require explicit
viewer control. Interactive scripts additionally require online mode in this
first implementation: do not present script-enabled mode as fully offline without
qualifying browser channels such as WebRTC. Turning online mode off also disables
scripts. Permissions reset when a different file is opened.

Online mode has document-only authority. Enabling it closes the old browser and
request context, cancels pending requests and revokes all relative-resource IDs,
grants and caches. A fresh origin/generation exposes only the captured main
document. Embedded and remote resources may then work, but relative native/archive
resources are unavailable. State/tooltip text explains this restriction and that
enabled scripts can read and transmit the main document. No online folder-access
permission is part of this slice. Remote scripts included by a local page therefore
cannot use its origin to read arbitrary siblings. Returning offline also creates
a fresh context, with network/scripts disabled before restoring the asset scope.
This is a backend authority transition, not just a frontend checkbox change.

Apply policy to navigation, subresources and redirects, including a restrictive
response CSP and browser request interception. Explicit denials must not fall back
to Chromium's normal network/file loaders. No `file://` loading, external protocol
launches, downloads, popups, permission prompts, browser extensions, user browser
profile or renderer-facing file-manager bridge. Remote content cannot obtain a
local resource capability. Chromium's sandbox stays enabled; fail closed if it
cannot initialize. Request policy is not advertised as an OS-level network sandbox.

The existing single-process image/PDF decoder sandbox remains unchanged. Add an
explicit multiprocess browser profile rather than weakening it. Validate the
complete private runtime package, inherit only intended IPC handles, supervise
the owned process tree and release source/resource leases on termination. The
trusted Chromium broker is not described as an unprivileged decoder sandbox.

Windows uses the matching official `bootstrap.exe` and same-basename Rust cdylib.
The pinned five-argument C `RunWinMain` entry receives sandbox and version data.
Dispatch subprocess execution before application IPC, pass the sandbox pointer
to both CEF execute/initialize, and use the same bootstrap for all CEF processes.
Do not use `--no-sandbox`, a null-pointer fallback or single-process mode. Bootstrap
and client DLL must both be unsigned or have matching valid primary signatures.

Linux has its own launcher and CEF lifecycle files. Require supported Chromium
namespace/seccomp isolation or a correctly installed helper; incompatible system
policy must yield an actionable error. Qualify Linux display/session behavior
separately, including X11 and Wayland, rather than inferring it from Windows tests.

## Viewer behavior

Keep the normal viewer size, resize behavior, toolbar and search field. Escape
always closes, even when page content has focus. Ctrl+F opens Find, F3/Shift+F3
advance matches, zoom affects Chromium layout, and copy remains an explicit user
action mediated by the host clipboard service. Pointer, keyboard and IME input
must not leak into the file panels.

Space cycles panel files when browser content is not editing a form. The browser
must arbitrate this against its current editable focus; a delayed frontend focus
flag is not sufficient. Search and toolbar controls keep their normal editing
behavior. Navigation and resize never block the main UI thread.

## Pinned runtime and licenses

Candidate exact pair verified from the official stable artifact metadata:

- `cef` / `cef-dll-sys` `156.4.0+156.0.26`, using MIT from MIT OR Apache-2.0.
- CEF `156.0.26+g8ea0c73+chromium-156.0.8078.88`, BSD-style CEF terms plus
  Chromium/bundled-component licenses. Preserve and audit `CREDITS.html`; the
  CEF license alone is insufficient.
- Keep the `sandbox` feature; disable unrelated optional Rust features. Pin native
  archives and record SHA-256 after checking published archive metadata.
- No proprietary codecs, Widevine or paid/trial-only components are added.

Windows minimal archive is about 175 MB compressed; Linux about 333 MB. They are
optional runtime packages, not additions to `me.exe`. All local downloads,
compilation, tests and staging remain under `_build`; dependency caches use
`.tools`. Update the dependency inventory and distributable notices before shipping.

## Acceptance

Test real protocol boundaries, hostile input/lengths, stale paints, queue pressure,
source mutation, cancellation, Unicode/long paths, asset traversal/link escapes,
archive resources, absent providers/runtime, renderer crashes and process cleanup.
Native CEF tests must prove actual HTML/CSS rendering, scripts denied by default,
denied remote requests, working search/selection/zoom, and sandboxed children.
Exercise the installed viewer through F3 and user association settings, scrolling,
resize, Escape and file cycling; inspect its visible output and record latency.
Keep independent plugin tests in its own project. No successful mock-only test
is evidence that native Chromium launched or that Linux is qualified.
