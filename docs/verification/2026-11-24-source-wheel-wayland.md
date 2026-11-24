# Source wheel scrolling on Wayland

Linux, NVIDIA GPU, native Wayland, Slint 1.18.1 / FemtoVG-WGPU. The read-only
reported Python file was 13,098 bytes, 355 CRLF lines, with tabs and syntax
highlighting. Its contents are not included in the repository. The running
installation used the same viewer sources as the initial `ff6880b4` checkout.

## Diagnosis

The GPU redraw blocked the UI thread for roughly one second at a time. The
camera advanced correctly whenever rendering returned; the pauses occurred
inside rendering, independently of source-page refills. X11 GPU and Wayland
software controls did not reproduce these long pauses.

The pinned Slint FemtoVG-WGPU adapter does not call Winit's
`pre_present_notify`, unlike Slint's software, Skia, Vello and FemtoVG-GL
adapters. Winit uses that notification to request Wayland frame callbacks.
The pinned wgpu surface-acquisition timeout is 1,000 ms. Supplying the missing
notification removed the repeated one-second waits in the same wheel trial.

The application now installs a weak rendering notifier once per Wayland window,
calling `pre_present_notify` only at `AfterRendering` for WGPU. This is after
successful surface acquisition and immediately before presentation. Sending it
before rendering would risk waiting for a commit after an acquisition that
skips. X11 and non-Linux implementations do nothing; an existing custom notifier
retains its slot. The performance probe uses the same helper.

A separate trace showed an upward notch reaching the document start and then
snapping one pixel back down. Whole-pixel resting alignment now preserves a
reached document boundary, including a fractional world coordinate after a
page rebase.

## Native trials

These are native rendered windows and real plugin reads, driven through the
existing native wheel observer with 96 notches, reversing after 48, nominally
50 ms apart. The native compositor supplies redraws; the acceptance seam's
headless redraw pump is disabled for a Winit-backed window. This is repeatable
observer input, not OS mouse injection or compositor readback. No pixel
snapshots were taken during these timing runs.

The baseline used a temporary adapter to that same observer, before the
presentation fix. The retained camera edge fix was already present. Reports
remain under `_build/`; elapsed time includes opening and the probe's setup.

| Trial | Inputs submitted | Completed | Redraw p95 | Longest redraw | Elapsed |
| --- | ---: | --- | ---: | ---: | ---: |
| Wayland GPU, before notification | 80 / 96 | Timeout | 1,023.5 ms | 1,050.1 ms | 45.64 s |
| Wayland GPU, final, run 1 | 96 / 96 | Yes | 29.6 ms | 46.7 ms | 7.86 s |
| Wayland GPU, final, run 2 | 96 / 96 | Yes | 29.8 ms | 52.5 ms | 7.90 s |
| Wayland GPU, final, run 3 | 96 / 96 | Yes | 29.1 ms | 52.9 ms | 7.75 s |
| Wayland GPU, final, 2x scale | 96 / 96 | Yes | 28.9 ms | 52.0 ms | 7.97 s |
| Wayland software, final | 96 / 96 | Yes | 5.6 ms | 8.2 ms | 6.12 s |

Every final trial reached both document ends. The three ordinary GPU trials
retained gaps between moving frames of at most 55.7 ms, rather than the
one-second freezes. These measurements establish removal of the reproduced
stall on this workstation, not universal 60 Hz presentation.

Reproduction, with the source file named by `SOURCE_FILE` and an installed
source plugin beside the example executable:

```sh
cargo build -p midnight-viewer-ui --example performance_probe \
  --features acceptance-seams --locked --offline
MIDNIGHT_VIEWER_PROBE_UI_SIZE=14 MIDNIGHT_VIEWER_PROBE_WHEEL_MS=50 \
  _build/debug/examples/performance_probe _build _build/source-wheel.log \
  --file="$SOURCE_FILE" --observer-wheel --sustained --wheel-trace
```

Repeat with `SLINT_SCALE_FACTOR=2` for the high-DPI case, `--software` for the
software control, or `env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11` for X11.
The source plugin used for these trials was built from an isolated copy below
`_build` against the unchanged public API/SDK sources; the ordinary SDK
packaging command could not resolve uncached `cbindgen` offline. No third-party
dependency version, feature, license or distribution notice changed.

## Regression evidence

`prepared_scroll_reaches_fractional_document_edges_without_bouncing_inward`
failed before the camera change with `0.00010000169277191162 -> 1`. After the
change, all 26 retained-camera tests passed. The new test covers both ends,
eased wheel and direct input, monotonic travel and repeated outward input.

Final library validation compiled once with:

```sh
cargo test -p midnight-ui-input -p midnight-viewer-ui --lib \
  --locked --offline --no-run -j 4
```

The reported library executables then ran from their package directories with
`--test-threads=2`: `midnight-ui-input` **64 passed**, `midnight-viewer-ui`
**245 passed**, zero failed or ignored. `scripts/check-source-size.sh` checked
3,499 source files, all within 100,000 bytes. The local build used the installed
Fontconfig runtime and existing `_build` pkg-config/linker aliases because the
system's Fontconfig development metadata was absent.

The application integration test build also passed:

```sh
cargo test -p midnight-explorer --bin midnight-explorer \
  --locked --offline --no-run -j 4
```

Its reported test executable ran from the package directory with
`--test-threads=2`: **140 passed, 17 ignored, zero failed**. Ignored native and
measurement cases were not run. Formatting checks passed for
`midnight-explorer`, `midnight-ui-input` and `midnight-viewer-ui`.

Strict targeted linting passed with no warnings:

```sh
cargo clippy -p midnight-explorer -p midnight-ui-input -p midnight-viewer-ui \
  --no-deps --locked --offline -j 4 -- -D warnings
```

The raw `source_scroll` example also reported a pixel mismatch on returning to
75%. Inspection showed identical text-body pixels and asynchronous line-number
indexing in the gutter; this was not evidence of blank source content. The
named-file co-simulation normalizes CRLF to LF, so its green result alone did
not validate the reported file's real plugin path.
