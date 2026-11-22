# Independent Chromium HTML viewer

This required runtime plugin is its own Rust workspace and lockfile. It consumes only
the released `midnight-web-viewer-api` 0.3.0 registry package. Removing its
installed viewer directory removes HTML capability without changing the host, but
normal builds and distribution validation require the complete package.
Use `bash scripts/build-html-viewer.sh debug` or
`scripts/build-html-viewer.ps1 -Profile debug` from the repository for a standalone
rebuild with the verified pinned CEF cache. The normal build includes it automatically.

The host launches `midnight-viewer-html.exe` without arguments and provides MWV1
framed binary commands on standard input; standard output contains events only.
The executable is the matching official CEF sandbox bootstrap. Its same-basename
`midnight-viewer-html.dll` exports the pinned five-argument C `RunWinMain` and
passes the supplied sandbox pointer to both CEF execution and initialization.
Never substitute the Cargo Windows binary for this bootstrap. Both files must
be unsigned or carry matching valid primary signatures. Windows builds use a
static CRT and the exact CEF distribution's delayed-import list.

Set `MIDNIGHT_WEB_VIEWER_PROFILE` to an absolute, owned private session directory.
The trusted host creates and pins it, restricts permissions, supervises the full
Chromium process tree, and removes it only after every child exits. It is runtime
storage, never a viewed-document resource path. Each document uses a fresh
off-record request context in which cookies are disabled entirely: no cookie is
loaded or saved, session cookies included, so online sites that require cookies
do not work. Windows 10 or later
supplies the OS shader compiler; the package does not redistribute Microsoft's
`d3dcompiler_47.dll`.

## Build and package on Windows x64

Obtain the exact official minimal CEF archive recorded in [THIRD_PARTY.md](THIRD_PARTY.md),
verify its published SHA-1 and recorded SHA-256, then extract beneath `_build`.
Use the repository's Rust/MSVC environment, CMake and Ninja. Build with the
immutable public API offline registry bundle generated from `web-viewer-api`:

```powershell
./plugins/midnight-viewer-html/scripts/build-windows.ps1 -SdkConfig <absolute-registry-config>
```

Defaults derive from the checkout root; `-TargetDirectory` selects output beneath
`_build`, and `-Release` selects an optimized build. The normal build
(`scripts/build-html-viewer.ps1`) passes `-CefSdk` with the flattened SDK that
`scripts/prepare-html-cef.py` verified: Cargo compiles against it in place and the
package copies its runtime files from it, checking them against its `.verified.json`
receipt. That SDK lies below the checkout's own `_build`, however the checkout is
spelled (it may lie below a link or hidden `AppData`), with no link from `_build` down;
the download cache must be a plain folder inside the checkout. On Windows, renaming
the fresh SDK and package into place waits for about 2 s while endpoint protection
holds a file just written. With `-CefDistribution` (default: the extracted distribution
beneath `_build/html-viewer/cef`) the helper flattens the layout required by `cef-dll-sys`
once into `<target>\cef-sdk` and records a `.staged.json` receipt, which the package
checks the same way. The helper builds the Rust DLL and creates
`_build/html-viewer/provider-target/package`.
Copy the whole resulting package beneath the application-private
`viewers/midnight-viewer-html/` directory. Do not register it from an opened folder.
Retain `LICENSE` (this provider's MIT licence), CEF's `LICENSE.txt`, `CREDITS.html`,
`THIRD_PARTY.md`, `notices/` and `runtime-files.json`; the latter lists every package
file's SHA-256. Release packaging (`scripts/package-release.py`) refuses an HTML viewer
package without its own `LICENSE`.

## Verification

Use the same `--manifest-path`, `--config` registry bundle, Windows static CRT
config `plugins/midnight-viewer-html/.cargo/config.toml`, `CEF_PATH` flattened SDK,
and dedicated `--target-dir _build/html-viewer/provider-target` for `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `cargo fmt -- --check`.

The opt-in native test requires `MIDNIGHT_HTML_TEST_EXECUTABLE` pointing at the
packaged bootstrap and a fresh `MIDNIGHT_WEB_VIEWER_PROFILE`. Run
`cargo test ... --test native_runtime -- --ignored --nocapture` on an interactive
Windows desktop. It exercises real CEF, captured HTML and nested CSS streaming,
inline styling, script denial, a denied loopback HTTP image request, browser
Find and Find Next, select-control popup composition/hide, browser-arbitrated
Space input, editable-field Space and explicit selection copy, textarea
Enter/Shift+Enter, punctuation keys and a synthetic IME commit, layout zoom,
fractional-DPI resize, and clean process exit; `chromium_applies_each_wheel_event_at_once`
checks that one wheel event scrolls its whole distance in the next frame, stops at the
top edge, and that a notch the host glides in 25 frames of 2.4 px scrolls its 60 px. It
reads the machine's wheel setting as CEF does and expects what Chromium can scroll there:
the closest distance not past 120 px for the single event, and for the glide 60 px or
less by under one wheel unit's scroll (both exact at three lines and on Linux); with a
notch set to scroll a page it does not run. On Windows 11 (21 November 2026, debug
package of this checkout, three wheel lines) it first measured 100 px for a 120 px event;
with the unit conversion below alone, the glide moved 47 of 60 px; with whole quanta it
passed at three lines. With the per-event conversion below it passed again (21 November
2026, Windows 11, three wheel lines, a debug package of this checkout's provider: 120 px
and 60 px, 2.44 s); other wheel settings are covered by the provider's unit tests, not
measured. It earlier ran on Linux X11. Editable
steps wait for the copied selection rather than fixed delays, because frames carry no
input acknowledgment.
The final fixture passed in 2.29 seconds on the development Windows environment;
on a loaded development machine CefShutdown alone took 10-12 seconds, so the
fixture allows 30 seconds for the clean exit. Tests
inside Codex's additional process sandbox failed child DLL startup; external
qualification keeps Chromium's own sandbox enabled and succeeds. No unsandboxed
fallback exists.

Policy changes require a new Open document generation and fresh source/resource
IDs. The old browser must complete OnBeforeClose before a new context is created;
a five-second close timeout exits the provider instead of weakening isolation.
Offline permits only the current synthetic origin, main document and bounded
host resource capabilities. The captured document is served at its folder's URL, 16
synthetic folder levels (`mewv-level-1/` … `mewv-level-16/`, see `src/policy.rs`) below
`https://mewv-<generation>.invalid/mewv-document/`, so that links with up to 16 `../`
segments stay apart after Chromium resolves them; relative assets, and without host
link routing offline relative navigation to other captured files in scope, resolve
below that folder through host capabilities. A reference that climbs above it (`../x`)
or is root-relative (`/x`) is denied, never mapped onto a same-named file of the
document directory. `State.url` always reports
`https://mewv-<generation>.invalid/mewv-document/` (not the page shown after an
in-scope navigation), and `State.editable` is always false. Online exposes
only the captured main document and permits HTTP(S); relative native/archive assets
are revoked. Scripts require explicit online mode. Navigation, redirects, downloads,
popups, file dialogs, permissions and external protocols cannot grant native
authority. Select-control popup surfaces are separately bounded and composited into
owned RGBA frames; without a visible popup each converted paint moves into its
frame without a second copy. No OnPaint or resource-output pointer survives its callback.

With the host's exact `MIDNIGHT_WEB_VIEWER_LINKS=host-v1` launch value (web-viewer API
0.3), the provider routes links to the host instead (`src/links.rs`, routes in
`src/navigation_route.rs`): Chromium's status URL of the hovered link becomes
`LinkHover` when it changes, and a navigation of the main frame to another document
that the user started (a click of any kind, Enter on a link, `target=_blank`,
`window.open`) is cancelled and becomes `LinkActivated`, at most one per 500 ms.
The first load, reloads and fragment navigations within the document stay in the
browser; navigations without a user gesture, form submissions and targets that cannot
be expressed are refused with one `Denied` notice per second at most. Targets are
absolute URLs off the private origin and folder-relative references (up to 16 `../`)
on it; see the [web viewer API](../../docs/developer/WEB_VIEWER_API.md#link-routing-apisdk-03).
Without that value, links load in the browser as before.

At most 32 host resource requests are in flight; further browser requests wait in
a FIFO of at most 4096 instead of failing, and a relative URL over 8 KiB is denied
locally. Provider events are validated before they are queued, so an over-budget
event becomes a bounded runtime error instead of stopping the transport. Paints and
browser state keep only their latest value, resource releases use their own bounded
lane, and the remaining controls use a FIFO of 256. The run loop handles up to 64 queued
commands per CEF pump and wakes as soon as a command arrives. Without commands it waits
at most 2 ms before the next CEF pump, and no external message pump is used. On Linux
that is about every 2 ms (not measured). On an idle Windows document it is about 64
pumps per second, because the wait rounds up to the timer period.

Windows virtual keys never come from a character's ASCII value: punctuation uses
the US-layout OEM key of its physical key, letters follow the produced Latin letter
(QWERTZ/AZERTY), numpad keys follow NumLock, and unknown keys are 0, so typed
punctuation never acts as Delete, arrow or page commands. Mouse Back/Forward buttons are not forwarded as clicks, and
focus loss ends a held button's capture. Chromium runs with `disable-smooth-scrolling`:
the host glides wheel notches and coasts touchpad flicks itself, one delta per frame, so
Chromium applies each wheel event at once instead of easing it again over about 170 ms.
The host's deltas are the distance to scroll, a notch as far as in the text viewer: 100 px
at three lines since 21 November 2026, 60 px when the figures below were measured.
Chromium scrolls whole pixels of whole units. On Linux CEF reads a delta as pixels, so
a unit is a pixel. On Windows CEF reads it as `WHEEL_DELTA` units (120 per notch) and
Chromium scrolls the system's wheel lines (characters across) of 100/3 px each, computed
in `f32`, and drops the fraction of a pixel each event scrolls: a unit is 5/18 px per
line. The Windows adapter reads the current `SPI_GETWHEELSCROLLLINES`/
`SPI_GETWHEELSCROLLCHARS` setting for every event, as CEF does, and models that
arithmetic. Each event sends the fewest units that scroll the most whole pixels not past
those due, one or more as soon as one pixel fits; what Chromium will not scroll for them,
and the fraction of a pixel, is carried to the gesture's next event. A gesture ends when no wheel
event follows for 250 ms (longer than a notch's 200 ms glide), and its rest, less than
one unit's scroll, is dropped instead of moving the next gesture; a reversed direction
drops it too. At three lines or fewer a unit is at most 5/6 px and every whole pixel is
reachable, so a notch scrolls exactly 60 px. Above three lines not every distance is a
whole number of units, and a gesture comes within one unit's scroll of its distance: a
notch scrolls 59–60 px at five or seven lines and 58–60 px at ten (before, the adapter
sent only units that scroll whole pixels, so 50, 35 and 50 px, with the rest waiting for
the next gesture, and touchpad glides jumped in 25–35 px steps). CEF on Windows scrolls
one axis per event, so a delta on both axes goes out as two events. A "one screen at a
time" setting cannot scroll by pixels; the adapter then uses the default three lines, and
Chromium still scrolls by pages.
IME composition and commit pass CEF's required invalid replacement range.

Unit tests (`cargo test ... --lib --bins --test policy_surface --test native_runtime`)
passed on Windows on 21 November 2026: 110 passed, 1 ignored in the library, the
platform tests included (after the per-event wheel conversion; 109 before it), and 5 in
`policy_surface`. Four resource and navigation tests had failed on Windows as on
Linux because they read `ResourceRelease` from the control lane, while releases have
their own lane since the output lanes were split; they now read the provider's output
in order. This was a test defect, not an environment limit. Linux was not rerun on this
host: its WSL has no Linux CEF SDK.

## Linux status

Linux has a separate process entry, window/input adapter, and CEF lifecycle.
The launcher requires Chromium namespace/seccomp isolation or a correctly
installed sandbox helper; unsupported policy must fail. The exact Linux x64 pair
was qualified on Ubuntu 24.04 under WSLg XWayland with namespace/seccomp enabled
and delegated memory/pid cgroup supervision. Actual installed host and Slint
software-renderer tests cover source-relative CSS through explicit native directory
authority, a 350-byte Unicode source path, Find, zoom, fractional viewport resize,
Escape, policy-context replacement, provider crash and complete tree/state cleanup.
Independent native CEF protocol tests additionally cover offline script/network
denial, Find/Next, selection/Copy and editable Space/Enter/Shift+Enter.
The launcher passes its arguments to CEF as raw bytes, so a non-UTF-8 installation
path does not abort startup. Before CEF starts any child, the browser process keeps
a private close-on-exec duplicate of the MWV1 standard output for its writer and
points descriptor 1 at standard error, so Chromium children and native libraries
cannot write into the protocol stream. These two Linux adapter changes have not yet
been qualified with the native Linux package.
Physical IME input and native Chromium Wayland remain unqualified. Other display/system
policies require their own native qualification. macOS has no implementation here.

The launcher links `libcef.so` through a literal `$ORIGIN` DT_RPATH; CEF's other shared
libraries come from the distribution (NSS/NSPR, ALSA, ATK/AT-SPI, CUPS, D-Bus, DRM/GBM,
Expat, GLib, Pango/Cairo, X11/XCB and its extensions; `scripts/setup-linux.sh --install`
installs them). When one is missing, the dynamic loader stops the launcher with status
127 before its first message, and the host names the missing libraries and the command
that installs them instead of a stopped transport, from the ELF dynamic sections of the
launcher and its libraries ([web viewer API](../../docs/developer/WEB_VIEWER_API.md)).

The host keeps the full profile path and separately owns a short private socket
directory, retained through whole-tree reaping. This avoids Chromium's AF_UNIX path
limit without truncating source/profile identities. Existing user delegation and
sandbox policy remain required; no root cgroup configuration or sandbox bypass is
performed. Linux qualification artifacts are retained under `_build/qa/html-linux`.

`scripts/build-linux.sh /absolute/public-sdk-config.toml` can stage an independent
Linux x64 package when `CEF_PATH` names the exact verified flattened Linux SDK.
The helper requires Python 3.11 or later for the standard-library SHA-256 streaming API.
It writes beneath `_build/html-viewer/provider-target/linux`, validates all 929
flattened SDK inputs against `scripts/cef-linux-pinned.json` before invoking Cargo,
and rejects unexpected SDK files/directories or environment overrides that could
trigger cef-dll-sys fallback downloads. Flatten the verified official archive's
Release and Resources contents plus include, cmake, libcef_dll, CMakeLists.txt and
original LICENSE/CREDITS into CEF_PATH; archive.json must identify the exact archive,
type and published SHA-1. The immutable public registry config must use Linux paths.
The marker itself is byte-pinned along with the SDK. Create its exact UTF-8/LF form:

```bash
python3 - "$CEF_PATH" <<'PY'
import json, sys
from pathlib import Path
marker = {
    "type": "minimal",
    "name": "cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_linux64_minimal.tar.bz2",
    "sha1": "cacfbb7d7261fea682fb67c108de7e4347519ff7",
}
(Path(sys.argv[1]) / "archive.json").write_text(json.dumps(marker) + "\n", encoding="utf-8")
PY
```

The launcher uses literal `$ORIGIN` ELF DT_RPATH so the supervisor's cleared
environment resolves the package's sibling runtime libraries. Packaging requires a
fresh package destination to prevent stale components and preserves target-filtered
dependency metadata, all Rust/native notices and `runtime-files.json` SHA-256 entries.
Dependency notices are found and read without following links: a link below a
dependency's root fails packaging, while links above it (a linked cargo home) are
accepted. Optional private native-library input requires `MIDNIGHT_HTML_NATIVE_MANIFEST` with
explicit source/path/SHA-256 entries and matching license/source distribution data;
see THIRD_PARTY.md. No install, root action or setuid configuration is performed.
Each newly built package still needs real supervised rendering/input/cleanup
qualification in its intended environment. Run packaging safety tests independently with
`python3 plugins/midnight-viewer-html/tests/test_linux_package.py` on Linux.

## Native source availability

The exact CEF source commit is `8ea0c73` on Chromium branch `8078`, matching
`156.0.26+g8ea0c73+chromium-156.0.8078.88`; Chromium is version
`156.0.8078.88`. Use the official CEF automated build setup and the exact CEF
commit, not the current branch head. Its pinned `DEPS` and Chromium revision
select the bundled component sources, including FFmpeg/WebKit/MPL components
identified in the shipped CREDITS. Preserve the original archive metadata and
build coordinates when redistributing this runtime. Corresponding sources and
rebuild instructions are available through:

- [CEF exact source tree](https://github.com/chromiumembedded/cef/tree/8ea0c73)
- [CEF automated build setup](https://chromiumembedded.github.io/cef/automated_build_setup)
- [CEF Chromium compatibility coordinates](https://github.com/chromiumembedded/cef/blob/8ea0c73/CHROMIUM_BUILD_COMPATIBILITY.txt)
- [Chromium source and DEPS](https://chromium.googlesource.com/chromium/src/+/156.0.8078.88/)
- [Exact cef-rs source](https://github.com/tauri-apps/cef-rs/tree/9aa78c450489eddb828eb982eaee6c104a89d348)

For a reproducible native source checkout, obtain Google's depot_tools at
`132599ebbd8d22d1b1bd5ea703a6d3bc493edd71`, and the
[automation script at the exact CEF commit](https://raw.githubusercontent.com/chromiumembedded/cef/8ea0c73/tools/automate/automate-git.py).
Run that script with `--download-dir=<owned-build-directory> --branch=8078
--checkout=8ea0c73 --no-build --no-distrib`; retain its exact Chromium
`refs/tags/156.0.8078.88` checkout and run depot_tools `gclient sync` using that
tree's pinned DEPS. Follow the pinned script's documented platform/toolchain
steps and GN settings to rebuild CEF, bootstrap and the minimal distribution;
do not silently enable proprietary codecs or Widevine. Include these coordinates,
the full original notices, and an accessible corresponding-source offer/source
archive with a published runtime. Local testing and inventory generation do not
constitute final distribution/legal approval.

CEF remains dynamically linked; users may replace compatible rebuilt `libcef.dll`
and its matching runtime components. If the CEF ABI/version changes, rebuild this
independent client against the matching bindings and official bootstrap, and
update the explicit version check. Signature requirements and sandbox ABI
compatibility still apply. Do not remove copyright or corresponding-source
obligations when repackaging the official binaries.
