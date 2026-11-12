# Viewer implementation and verification

The implementation follows the [accepted extension](../superpowers/specs/2026-11-05-viewer-implementation-extension.md)
and [implementation plan](../superpowers/plans/2026-11-05-viewer-plugins.md).
Salamander's `src/viewer3.cpp` Space/next-file behavior and
`src/plugins/demoview/viewer.cpp` Escape/source enumeration were consulted as behavioral
references. No Salamander code or resources were copied.

## Boundaries

- `midnight-plugin-host`: bounded discovery, native trust checks/loading, C ABI
  lifetime, content leases and requests. Platform loader, native file identity and
  installation permission checks live in separate Windows/Linux files.
- `midnight-viewer-engine`: Tokio backend workers, captured file order, navigation,
  search, semantic page conversion, selection and shared Parley/Swash rasterization.
- `midnight-viewer-ui`: small independent Slint window and message adapters. It owns
  rendering resources, focus, input and presentation; no filesystem reads or parsing.
- `plugins/midnight-viewer-{basic,source,markdown}`: independent Cargo workspaces,
  locks, fixtures, tests, exported builds and licensing. Dependencies on this project
  use the public registry API/SDK only. They do not import host crates or each other.
- F4 is described in [external-editor.md](external-editor.md).

The large existing Slint binding compilation moved into `midnight-ui-bindings`, with
tests/doctests disabled for generated code and dev debug information disabled for
that crate. Before the split, the generated main binding was 18,609,423 bytes and
its UI check took 33.6 seconds. Afterwards binding checking took 28.5 seconds and
the handwritten UI crate took 1.0 second. These local warm-check measurements justify
avoiding repeated compilation of generated code in handwritten UI tests; they do
**not** establish a reduction in generated source size or a runtime speedup.

## Runtime limits and semantics

Four live windows/sessions are allowed. Each window uses a bounded 64-command queue
and a latest-frame mailbox. Copy requests retain the latest clipboard intent.
Closing has a reserved atomic signal, so a full queue cannot prevent cancellation.
The main window closes its viewers during application shutdown.

Captured source order is limited to an estimated 8 MiB of path/identity descriptors.
The focused identity is prepared first; a separate bounded backend producer prepares
the remaining regular files in next-file order. This is not an atomic directory
snapshot. Identity/revision checks prevent silently opening a changed object later.
Changing panel sorting/navigation or closing its tab cannot change the captured order.

Content reads are at most 256 KiB. Plugins use smaller bounded windows, with exact
u64 source offsets distinct from UTF-8 text ranges. The host caps decoded page memory
at 16 MiB/session and 64 MiB globally; wire queues, parser/source-map items, service
replies and indexes have separate caps. The worker normally requests 12 KiB text
or 4096 binary bytes, at most 512 lines. A raster is limited to 4096 pixels per axis
and 4,194,304 pixels (16 MiB RGBA); exceptionally large windows reduce raster scale.
Raster buffers and UI image copies are additional to the decoded-page cap.

Scrollbars use normalized positions while backend anchors stay u64. Seeking to EOF
requests a page ending at EOF. Text/source pages do not wrap; horizontal scrolling
handles long lines. Source styling covers the package's published catalogue, with
plain text fallback when syntax work exceeds its budget. Markdown renders semantic
headings, paragraphs, lists, quotes, code and inline emphasis, with inert link text.
It does not implement a web browser or fetch resources.
Automatic host routing currently uses filename extensions. The source plugin's own
shebang detector is tested, but an extensionless executable is routed to the basic
fallback; automatic content-probe routing is not part of this initial host interface.

Text selection and Ctrl+A/Ctrl+C are bounded to the loaded page. Copy returns displayed
Unicode text; binary mode copies the displayed hex/text representation. Search scans
on a cancellable backend task. Markdown search follows rendered text; switching to
Text mode searches literal markup. Searches stop at the end rather than wrapping.
Space cycles files only while content is focused; search widgets retain ordinary
spaces, IME and editing keys. Escape closes from either context. Modes and viewer
shortcuts remain separate from main-window actions.

Options → Plugins lists discovery diagnostics and startup status. Enable/disable
changes persist in preference schema 10 and take effect on restart. Native code is
cached for process lifetime; closing the last instance does not unload its library.

## Native installation and test commands

All compiler/generated/test output stays below the repository's `_build`. Native
loading tests additionally require a deployed installation with safe native ownership
and ancestors. The shared Windows D: checkout and WSL drvfs mount did not meet that
condition: the loader correctly refused them. Windows QA copies were deployed under
`C:\Users\tjelinek\_build\MidnightExplorerViewerQA\windows`, protected for the current
user, SYSTEM and Administrators. Linux QA uses a private native filesystem directory
below `/tmp/fm-interaction/_build` with normal `0755` directories and `0644` files.
These are deployment copies, not additional compiler caches. A protected child under
a replaceable parent is intentionally rejected.

`HOST_TEST_INSTALL_ROOT` points host tests at a safe native temporary installation
parent. `HOST_TEST_PLUGIN` supplies the separately built basic DLL/SO. The engine's
opt-in test uses `MIDNIGHT_VIEWER_TEST_INSTALL`, an already installed directory with
all three packages. These variables are test-only and never change application
discovery, which remains relative to the actual executable.

```powershell
scripts/cargo.ps1 test -p midnight-viewer-engine --lib --offline
# Set MIDNIGHT_VIEWER_TEST_INSTALL to the private deployment first:
scripts/cargo.ps1 test -p midnight-viewer-engine --lib --offline '--' --ignored --nocapture
scripts/cargo.ps1 test -p midnight-viewer-ui --test keyboard --offline
scripts/cargo.ps1 test -p midnight-viewer-render --offline
```

`midnight-viewer-ui/examples/native_viewer.rs` exercises the real native window,
captures text/source/Markdown screenshots and verifies Space plus Escape from search.
Deploy its executable beside `viewers/` and pass three fixture paths followed by a
screenshot directory. Filesystem work and screenshot encoding remain backend work.

## Recorded results

Full workspace tests passed on Windows (966 tests, 16 opt-in skips) and Linux
(926 tests, 22 opt-in skips). Strict workspace/all-target Clippy passed on both;
workspace formatting and the 100000-byte source-size guard also passed. The
separate host native suites passed 24 tests on Windows and 25 on Linux, including
real DLL/SO loading and permission/ancestor rejection. Opt-in viewer-engine tests
were run separately with installed native packages. The actual Slint search-widget
test passed Unicode/Space entry and Escape propagation.
The final Linux native host rerun initially lacked `HOST_BAD_PLUGIN`; supplying the
separately built wrong-ABI fixture made all five opt-in tests pass. Both logs are
retained; this was a test-environment omission, with no production-code change.

Each exported plugin project was tested without application sources against the
released public SDK. Windows and Linux both passed 25 basic, 21 source and 34
Markdown tests, strict Clippy, formatting and native release packaging. Plugin-owned
tests cover malformed bytes, UTF-8/UTF-16 boundaries, huge-line/work limits, source
anchors, active Markdown content, cancellation and the declared syntax catalogue.

The optimized Windows engine suite passed all 12 tests, including native opening,
Unicode copy, cross-page search, EOF navigation, source/Markdown routing, mode
switching, captured-order cycling, changed-file rejection and close. The streamed
fixture exceeds 20 MiB. First text viewport was 110 ms with a 720,000-byte 600×300
surface; full-file search took 6.90 s, cancelling search and cycling took 154 ms,
and backend close took 5.5 ms. Peak process working set was 29,700,096 bytes.
This measures the backend test process, not the full GPU application. It is one
local run, not a percentile target or a network-storage cancellation guarantee.
Evidence: `windows-native-release{,-errors}.log` and
`windows-native-release-metrics.json` below `_build/qa/viewer-implementation`.

Independent review confirmed and reran fixes for selection-only repaint discarding
input from the still-visible frame, Page Down skipping unseen page content, and
empty/BOM-only Markdown Text mode emitting completion without a page. Final visual
QA exposed false previous-page flags from hidden Markdown prefixes/BOMs. The scoped
re-review additionally caught a skipped multibyte scalar at a full page boundary,
including a false-negative search at the production 8192-byte budget. Eleven
independent Markdown probes and the source BOM regression passed after correction.
Original failing and corrected probes remain under
`_build/qa/viewer-implementation/final-review`; no actionable finding remained.

The final optimized Windows GUI probe rendered all three viewers, exercised Space
and Escape from the actual search field, and saved inspected screenshots under
`_build/qa/viewer-implementation/windows-final-ui`. Initial text content appeared at
630 ms; source and Markdown at 834 ms and 1.034 s from launch. These include native
window/GPU startup and timer sampling. The Markdown first page now has correct
navigation bounds and no spurious scrollbar. Main-window release smoke checks also
passed; the bottom function bar was visible in the captured image.

Distribution checks passed 12 Windows tests (two OS-specific skips) and all 14 Linux
tests. Windows PowerShell 5 build/deployment checks passed, including streamed
SHA-256 readback, refusal of existing/unsafe destinations and unchanged ancestor
permissions. CI separates SDK publication artifacts, sparse independent plugin
builds and native host integration; its remote GitHub Actions run is not claimed
by these local checks.

The Linux optimized engine suite passed all 12 tests against the final `.so`
packages. First viewport was 77 ms, the >20 MiB search took 6.02 s, cancelling and
cycling took 41 ms, and close took 5.1 ms. `/usr/bin/time -v` recorded a maximum
resident set of 24,480 KiB for the backend test process. This is separate from
compilation and the full graphical application, and is not directly equivalent
to Windows working-set accounting. Logs are under
`_build/qa/viewer-implementation/editor/linux-final`.

Both optimized Linux executables built successfully. The actual native window
probe passed on Ubuntu 24.04 in WSL with an isolated Xvfb display and CPU Vulkan
renderer. Text/source/Markdown screenshots were inspected under
`_build/qa/viewer-implementation/linux-ui`; Space, spaces in the search field and
Escape worked. Viewports appeared at 1.516 s, 2.875 s and 4.473 s from launch in
that environment. The temporary native installation had to be recreated after a
WSL process ended; final preparation and probes ran in one WSL process. This did
not require production-code changes or another full test run.

macOS, a native Wayland viewer smoke check and native Fedora/openSUSE qualification
remain outside this Windows/Ubuntu verification run.
