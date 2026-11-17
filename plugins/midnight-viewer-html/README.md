# Independent Chromium HTML viewer

This required runtime plugin is its own Rust workspace and lockfile. It consumes only
the released `midnight-web-viewer-api` 0.2.0 registry package. Removing its
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
receipt. With `-CefDistribution` (default: the extracted distribution beneath
`_build/html-viewer/cef`) the helper flattens the layout required by `cef-dll-sys`
once into `<target>\cef-sdk` and records a `.staged.json` receipt, which the package
checks the same way. The helper builds the Rust DLL and creates
`_build/html-viewer/provider-target/package`.
Copy the whole resulting package beneath the application-private
`viewers/midnight-viewer-html/` directory. Do not register it from an opened folder.
Retain `CREDITS.html`, `LICENSE.txt`, `THIRD_PARTY.md`, `notices/` and
`runtime-files.json`; the latter lists every package file's SHA-256.

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
fractional-DPI resize, and clean process exit. Editable steps wait for the copied
selection rather than fixed delays, because frames carry no input acknowledgment.
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
host resource capabilities. The captured document is served at
`https://mewv-<generation>.invalid/mewv-document/`; relative assets, and offline
relative navigation to other captured files in scope, resolve below that directory
through host capabilities. A reference that climbs above it (`../x`) or is
root-relative (`/x`) is denied, never mapped onto a same-named file of the document
directory. `State.url` always reports that main document URL (not the page shown
after an in-scope navigation), and `State.editable` is always false. Online exposes
only the captured main document and permits HTTP(S); relative native/archive assets
are revoked. Scripts require explicit online mode. Navigation, redirects, downloads,
popups, file dialogs, permissions and external protocols cannot grant native
authority. Select-control popup surfaces are separately bounded and composited into
owned RGBA frames; without a visible popup each converted paint moves into its
frame without a second copy. No OnPaint or resource-output pointer survives its callback.

At most 32 host resource requests are in flight; further browser requests wait in
a FIFO of at most 4096 instead of failing, and a relative URL over 8 KiB is denied
locally. Provider events are validated before they are queued, so an over-budget
event becomes a bounded runtime error instead of stopping the transport. Paints and
browser state keep only their latest value, resource releases use their own bounded
lane, and the remaining controls use a FIFO of 256. The run loop handles up to 64 queued
commands per CEF pump and wakes as soon as a command arrives; without commands it
still pumps CEF about every 2 ms (no external message pump is used).

Windows virtual keys never come from a character's ASCII value: punctuation uses
the US-layout OEM key of its physical key, letters follow the produced Latin letter
(QWERTZ/AZERTY), numpad keys follow NumLock, and unknown keys are 0, so typed
punctuation never acts as Delete, arrow or page commands. Mouse Back/Forward buttons are not forwarded as clicks, and
focus loss ends a held button's capture. Fractional wheel deltas accumulate per
document. IME composition and commit pass CEF's required invalid replacement range.

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
Optional private native-library input requires `MIDNIGHT_HTML_NATIVE_MANIFEST` with
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
