# Platform and host crates review — 24 November 2026

Scope: `midnight-platform-windows`, `midnight-platform-linux`, `midnight-platform-common`,
`midnight-plugin-host`, `midnight-worker-platform`, `midnight-filesystem-host`,
`midnight-native-filesystem-host`, `midnight-viewer-engine`, `midnight-compare-engine`,
`midnight-media-host`, `midnight-thumbnail-host`, `midnight-web-viewer-host`,
`midnight-image-viewer-host`, `midnight-office-preview`, `midnight-viewer-render`,
`midnight-elevation`, `midnight-update` (about 280,000 lines of Rust, about 1,200
`unsafe` occurrences), reviewed against main `ac525b6f`. Priorities: `unsafe` and FFI
boundaries, child processes and job objects, IPC framing with plugin workers, viewer
worker threading, and the open design item on document zoom keys.

Coverage is partial. The 17 crates are too large to read completely in one pass; the
lists below say what was read and how.

## Coverage

| Area | How it was read |
| --- | --- |
| `midnight-update` | Every file in `src/` read in full; the `apply`, `client`, `loopback`, `trust` and `archive` tests read. |
| `midnight-elevation` | `main.rs`, `lib.rs`, `helper.rs`, `protocol.rs`, `linux.rs`, `sandbox_profile_linux.rs`, `windows.rs`, `windows_channel.rs`, `windows_trust.rs`, `windows_children.rs` read in full. |
| `midnight-filesystem-host` | `session.rs`, `session_close.rs`, `staging.rs`, `staging_memory.rs`, `staging_cancel.rs`, `nested.rs` read in full; staging space and roots not read. |
| `midnight-worker-platform` | `lib.rs`, `windows.rs`, `linux.rs`, `browser_windows.rs`, `browser_linux.rs`, `spool*.rs` read in full; browser cgroup, libraries and store files in part; browser profile and sandbox files not read. |
| `midnight-plugin-host` | `lib.rs`, `loaded.rs`, `trusted_executable_windows.rs`, `trust_repair_windows.rs`, `native_linux_files.rs` read in full; `callbacks.rs`, `dispatch.rs`, `session.rs`, `content.rs`, `native_windows.rs`, `trust_windows.rs` in part. |
| `midnight-native-filesystem-host` | `ffi.rs` in full, `admission.rs` in part. |
| `midnight-platform-windows` | Drag and drop, clipboard, shell menu and its host, desktop properties, path and icon pickers, executable icons and icon lists, user command output read in full; the `unsafe` sections of icons, watch service, directory records, desktop, transfer preservation/recovery/finalize/namespace parent/source links/link stage/destination, mutation, open, sort keys, locale, OneDrive locations, storage kind, volume details. Other files: `unsafe` sites only or not read (tests, `watch/`). |
| `midnight-platform-linux` | The `unsafe` and libc sections of transfer leases, locale, open files, user commands and their output, sort keys, metadata `chmod_pinned`, the XFS case-insensitivity ioctl, and the Wayland drag-and-drop source (lines 1–200). X11 code not read (it uses the safe x11rb bindings and has no `unsafe`). |
| `midnight-viewer-engine` | `image_input.rs`, `worker_image.rs`, `document_view.rs`, `image_view.rs`, `input_queue.rs` (1–200), `worker_loop.rs` (1–200), `image_worker.rs` (1–140) and the scene's surface tracking read; the rest not read. |
| `midnight-image-viewer-host` | `raster_reply.rs` read in full. |
| `midnight-thumbnail-host` | `scheduler.rs` (1–220) read. |
| `midnight-compare-engine`, `midnight-media-host`, `midnight-web-viewer-host`, `midnight-office-preview`, `midnight-viewer-render`, `midnight-platform-common` | Not reviewed in this pass. |

## Findings and fixes

| # | Severity | Finding | Status |
| --- | --- | --- | --- |
| 1 | Medium | The update archive's SHA-256 was computed on one handle and the archive was reopened by path for extraction (`controller.rs`, `trust::check_file`, `archive::extract`). On Windows std opens files with full sharing, so another program could change or replace the file between the check and the extraction. A copy run as administrator from an administrators-only folder downloads to the user's `%LOCALAPPDATA%`, which the user's unelevated programs can write: content they choose would be staged into the protected installation. | Fixed: `SignedSums::open_checked` replaces `check_file`; it checks the digest on a handle that denies write, rename and delete sharing on Windows, rewinds it, and `archive::extract` takes that `File`. |
| 2 | Low–medium | After `Outcome::Unconfirmed` (no confirmation within 180 s while the new version runs) `pending.json` stayed. Every later start took the confirm branch: the previous version's backup was never removed and "Updated to version Y." showed at every launch. `updates.md` already promised that "the next start after a confirmation removes it". | Fixed: the updater marks `pending.json` unattended; the first start that finds the new version's confirmation removes the backup, `confirmed` and `pending.json`. |
| 3 | Medium | WinHTTP `header()` read every header into a fixed 1,024-unit buffer and gave `None` on `ERROR_INSUFFICIENT_BUFFER`. A longer `Location` (GitHub's signed download redirects are about that long) failed as "GitHub sent an invalid redirect." | Fixed: one retry at the size WinHTTP reports. |
| 4 | Low | `now + Retry-After` and `start + Content-Length` use server values: overflow panic in debug builds, wrap-around in release builds. | Fixed: both saturate. |
| 5 | Design item | Document zoom keys could lose a final press: after the engine published a zoomed frame (a new surface), a `ViewZoom` naming the frame the window still showed was refused by `ImageFrames::accepts_document`'s exact-frame rule. | Fixed, see below. |
| 6 | Medium (open) | `midnight-filesystem-host/src/session.rs`: a protocol 2+ archive worker can keep one exchange alive forever. Every `SourceRead` renews the 20 s idle watchdog, the total broker-byte budget applies only to protocol 1, and `list` and `unlock` exchange with no cancellation token (`exchange_read(None, None, …)`). Endless valid `SourceRead { length: 1 }` replies to `List` block the calling thread and, through the channel lock, every other request on that mount, and keep one of the two native slots. | Open: needs a decision on a per-exchange budget for protocol 2+ (legitimate workers re-read sources) and on whether cancelling a listing may stop the worker. |
| 7 | Medium (open) | `Mount::list` appends worker-supplied pages without a total limit while `more` is true (each page is bounded: 128 nodes, 64 KiB names, 1 MiB frames, and re-arms the watchdog). A hostile worker grows host memory at pipe speed until the user cancels. | Open: a total cap per listing needs a product decision; the design says a folder of any size lists completely. |
| 8 | Low (open) | Linux: after the fix for finding 1, a process of the same user can still write the checked archive through its own descriptor (Linux has no deny-write sharing). This matters only for a root copy whose download folder another user can write, such as `sudo` keeping `HOME`. | Open: would need a copy into the protected staging folder before the check. |
| 9 | Low, uncertain (open) | `transfer_preservation.rs` `security_parts` and `transfer_recovery.rs` `trusted_recovery_parent` walk security descriptors from `GetKernelObjectSecurity` using their own `AclSize`, `AceSize` and SID lengths, without checking them against the buffer. Only a malformed descriptor from a remote server could make that an out-of-bounds read, and whether the SMB redirector validates descriptors first was not established. | Open: hardening candidate (`GetSecurityDescriptorLength` ≤ buffer and an `IsValidSecurityDescriptor` check before walking). |
| 10 | Low, uncertain (open) | `file_drag_drop_shell.rs`: the non-agile drag `IDataObject` is wrapped by `SHCreateDataObject`. If that Shell wrapper aggregates the free-threaded marshaler, an in-process drop target on another STA (the path picker, property sheets) would call it directly. | Open: not verified; a drag onto the app's own path-picker dialog that records the thread inside `GetData` would settle it. |
| 11 | Low, uncertain (open) | `midnight-platform-linux` `file_drag_drop_wayland.rs` borrows winit's `wl_display`/`wl_surface`; the UI only sets a cancel flag and does not wait for the actor, so a window destroyed first could leave the actor using a dead surface. Opt-in (`MIDNIGHT_WAYLAND_DND=1`). | Open: shutdown order not traced. |
| 12 | Low (open) | `semver` ordering also compares build metadata, so a tag such as `v0.5.0+r2` would be offered as newer than 0.5.0. Not exploitable: the signature binds the exact version. | Open. |

| 13 | Test time | The three thumbnail service tests that wait for real retries (`RETRY_DELAYS`, 2 s then 8 s) were the critical path of `midnight-thumbnail-host`'s lib tests. | Fixed: under `cfg(test)` the same backoff is 200 ms and 800 ms; 2.53–2.59 s before, 0.75–0.78 s after (three runs each), 15 more runs passed. |

Checked and found sound (no change): image-viewer reply framing (`raster_reply.rs`
bounds the length before reading); filesystem-api framing (frame, batch and blob limits
before allocation, checked offset arithmetic); Windows worker spawn (job assigned at
creation, kill on job close, active-process limit 1, every error after creation kills
and waits); Linux workers reaped on drop; the elevation pipe and socket identity checks
in both directions; update extraction refuses links, `..`, escapes and duplicates;
Windows clipboard, drag-and-drop media, PIDL, icon and overlapped-I/O ownership; the
viewer input channel's reserved wake slot.

## Document zoom keys (finding 5)

A `ViewZoom` carries a source point and a viewport point. The engine resolves the source
point with the published snapshot's layout and, in the Paged layout, its page; the
camera's origin and scale are not used. Those stay the same across every frame from the
first frame of the layout shown last: a page turn resets `ImageFrames` (`page_turned`)
and a viewport change retires coordinates (`resized`). `ImageFrames` now records the
first frame of each published layout, and `accepts_document_image` accepts a `ViewZoom`
from any frame since then. Camera-anchored inputs keep the exact-frame rule: a layout
switch (its anchor is the published camera's centre) and a pointer `Zoom` by a factor
with an anchor. This matches ordinary images, whose `ViewZoom` was already accepted from
any frame of the image.

## Test evidence

Commands ran from the export root with the project's cargo environment, a private target
folder, `CARGO_INCREMENTAL=0`, `--locked --offline`.

- Red, before the fix: `midnight_viewer_engine` lib tests, filter
  `source_anchored_zoom`: `source_anchored_zoom_from_the_frame_before_a_published_zoom_applies`
  FAILED ("Paged: the final zoom press was lost", scale 1.25 instead of 1.5625); the
  layout-switch refusal test passed (it guards the boundary). Green after: all 656
  viewer-engine lib tests pass (20 ignored), 15.0 s.
- `midnight-update` integration tests, red before each fix:
  `a_late_confirmation_finishes_the_update_at_the_next_start` (assertion at the
  `confirm` check), `the_system_transport_reports_a_long_redirect_location` (Location
  `None`), `a_rate_limit_answer_reports_when_to_retry` (overflow panic at
  `client.rs:327`), `a_checked_archive_is_read_through_the_handle_that_was_checked`
  (with the share mode removed: the archive could be opened for writing). Green after:
  66 passed, 0.8 s.

## Test times

Each test executable of the 17 crates (built once with `cargo test --no-run`, 49
executables) was run alone, one after another, from the export root. Wall time includes
process start (about 1–1.5 s per executable on this machine); "in" is libtest's own time.

| Executable | Tests | Wall | In |
| --- | --- | --- | --- |
| `midnight-viewer-engine` lib | 656 passed, 20 ignored | 22.0 s | 18.1 s |
| `midnight-platform-windows` lib | 435 run, 26 ignored | 19.3 s | 7.7 s |
| `midnight-thumbnail-host` lib | 51 passed, 5 ignored | 13.2 s | 2.5 s (0.8 s after finding 13) |
| `midnight-platform-windows` `create_file` | 9 passed | 10.3 s | 0.03 s |
| `midnight-platform-common` lib | 184 passed | 7.3 s | 1.0 s |
| `midnight-compare-engine` lib | 179 passed, 4 ignored | 5.4 s | 2.0 s |
| `midnight-viewer-render` lib | 46 passed | 5.2 s | 2.9 s |
| every other executable | | 0.9–5.2 s | at most 1.6 s |

The slowest single tests (with `--report-time`, in parallel, so inflated by contention):
`worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::{source,text}`
19.4 s and 18.6 s, `image_worker::document_band_tests::a_fast_drag_publishes_the_cameras_it_passes`
14.7 s, the 2400-byte `long_rows` cases 7–11 s. These are the viewer engine's critical
path; shortening them needs the viewer owner to decide which travel cases still prove
the behaviour, and was not done here.

Run outside cargo, three executables failed for the environment, not the code:
`midnight-plugin-host` lib (54 tests need the CI-prepared `HOST_TEST_INSTALL_ROOT`), and
`midnight-platform-windows` lib (6) and `user_commands` (1), which need `rustc` on `PATH`
to build their probe. With the cargo environment the latter two pass (17 and 3 tests).

## Gates

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --locked --offline --features
  midnight-acceptance/acceptance -- -D warnings`: passed (6 min 28 s). The build script
  of `midnight-viewer-ui` prints Slint warnings: a binding loop on `layoutinfo-h`
  (`viewer.slint` lines 85, 327, 334, 351), which Slint calls deprecated and able to
  panic at run time. That crate is outside this review.
- `clippy -p midnight-thumbnail-host --all-targets -- -D warnings` after finding 13: passed.
- Linux: cross-clippy for `x86_64-unknown-linux-gnu` is not possible on this machine
  (`ring` needs `x86_64-linux-gnu-gcc`, `yeslogic-fontconfig-sys` a cross pkg-config). The
  Linux side of the changes is plain std code; the long-redirect test was not run on Linux.
