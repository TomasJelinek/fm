# Viewer review items, 25 November 2026

What each of the ten viewer items recorded on 20 November ended as, with its evidence.
Measurements ran on one Linux desktop host: Ubuntu 26.04 on GNOME Wayland, 32 cores and a
discrete GPU. The GPU numbers come from after a restart onto its driver 615. Builds used the
`dev` profile of main, unless a row says otherwise. Acceptance ran at smoke scale with
software rendering.

| Item | Outcome | Evidence |
| --- | --- | --- |
| 1. Raster frontier travel loss | Fixed before this pass (`6cd20942`): a frame restyled by lookahead counts as uncovered | `raster_frontier_tests`, `worker_frontier_tests`; acceptance R5 and R7 pass |
| 2a. Rows of 8000 bytes | Fixed (`dfd80cb7`): such a page grows at rest until its rows fill the window, Source scrollbar positions after a PageDown no longer fail, and the thumb stays where a position left it | `worker_long_row_tests` (`rows_of_8000_bytes_fill_the_window_at_rest`, `rows_of_8000_bytes_take_scrollbar_positions_after_page_down`); 661 engine library tests; the installed Source position tests on staged plugins |
| 2b. Chained replies | Partial: up to 16 replies join a page; a fast spin over rows wider than the window still stalls a few frames under load | [Backlog](../implementation-backlog.md) SCROLL-02 |
| 2c. Plugin page size | Decided, not implemented: run anchors in plugin API/SDK 0.3.0, so text plugins can reply with up to 64 KiB | [Design](../superpowers/specs/2026-11-25-viewer-paging-browser-open-design.md) |
| 3. PDF drags, image Home/End, HTML Find | Pass on Linux | [Viewer records](../developer/acceptance/viewers.md) (C8, O6, R8 and others) |
| 4. Syntax selection contrast | Not a defect: the readings were a measuring error (`a56d0a4b`); selected text has per-palette colours (`aac18026`) | Those commits |
| 5. Source Lines check box in small windows | Not a defect: a probe artifact | `ui_polish` |
| 6a. Archive copy-out at stress sizes (A13) | Pass on Linux, see [Archive stress](#archive-stress-a13); Windows (WIN-8) still open | `viewers_archives --stress`, run `20261125-190420` |
| 6f. AppArmor Install… offer | Checked by a real staged child (O20), which never presses Install… | Runs `20261125-180528` and `20261125-183655` |
| 6g. Archive members from Find feeds | Not applicable by design (FIND-01) | Viewer records X10 |
| 6h. Make File List viewer output | Covered by panels check L-08 | Viewer records X9 |
| 7a. HTML paint trace on Windows | Done before this pass | — |
| 7b. Short HTML wheel glides | Fixed (`b079a2a6`): a glide pauses through a page stall and ends only after 600 ms without a frame. A page slow to paint its first scroll gets the rest at once | 253 viewer UI library tests; `viewers_scroll` run `20261125-184840` passed with no review flags |
| 7c. Kept-ready HTML browser | Decided, not implemented (VIEW-08) | Design |
| 7d. HTML key distances | Decided: Chromium's own arrow and Page Down distances stay | [Web viewer API](../developer/WEB_VIEWER_API.md), [Scrolling](../developer/scrolling.md) |
| 7e | Done before this pass | — |
| 8. Text viewer opening under 200 ms | Met on Linux without a window pool; see [Text viewer opening](#text-viewer-opening). Windows took 1.1 s ([record](2026-11-25-windows-viewer-opening.md)), so the planned pool remains for Windows | `open_latency` |
| 9a. Steps deferred under a seam request | Fixed (`d6465f32`): they wait for the page past the edge instead of being dropped | `worker_seam_tests` |
| 9b–9e. Semicolon CSV, PDF thumbnail retry, video thumbnailer build, AVIF thumbnails | Done before this pass | Earlier records |
| 10. About animation | Drawing is cheap, so no layer cache is needed. On native Wayland with the GPU, the driver's FIFO presentation stalled frames ([record](2026-11-25-wayland-fifo-presentation.md)); with Mailbox presentation (`6f37db8c`) it holds 60 fps, see [About animation](#about-animation). The About box shows the application icon as set, without a halo (`2ebb1aff`) | `native_about` |

## Text viewer opening

`open_latency` opened a one-line text file three times per process and recorded the first
content of each opening. It was run twice, alternating the three setups. Times are in ms.

| Setup | First opening | Later openings |
| --- | ---: | ---: |
| Native Wayland, GPU | 120.0–121.0 | 102.6–105.3 |
| XWayland, GPU | 120.5–122.8 | 101.2–103.6 |
| Software renderer | 62.3–62.6 | 57.7–58.2 |

On 20 November a release probe on X11 with the previous driver took 268–322 ms
([record](2026-11-20-viewer-speed.md)). Since then capture and decoders start before the
window (`4e3e64c6`). The profiles differ, so the two sets are not a paired comparison.

## About animation

`crates/midnight-ui/examples/native_about.rs` shows the main window with 10,000 rows per
pane. It opens About after 1.5 s and measures the redraws of its sky for 5.5 s, after half
a second of settling. It feeds the application's Wayland presentation notifier, as the
application does. Two rounds each:

| Setup | Frames per second | Redraw span median / p95 | Gap median / p95 / max |
| --- | ---: | --- | --- |
| Native Wayland, GPU | 9.1, 8.0 | 4.35 / 5.43 ms | 33.0 / 967 / 986 ms |
| XWayland, GPU | 60.3, 60.4 | 16.6 / 17.0 ms (waiting for vsync in present) | 16.6 / 17.1 / 18.4 ms |
| Native Wayland, software | 59.8, 60.0 | 5.3 / 5.9 ms | 16.7 / 16.8 / 27.0 ms |

Drawing the whole window takes about 4–5 ms per frame on the GPU, so caching the main
window as a layer while About animates would save little. The native Wayland stalls come
from presentation. The driver's FIFO mode waits on presentation timing barriers, which the
compositor honoured up to a second late. Mailbox presentation on native Wayland was
measured with this probe before it landed: 60.0 fps in both rounds, with XWayland
unchanged. The idle window still drew 2 frames in 1 s, so Mailbox draws only when
something changes, as FIFO does ([record](2026-11-25-wayland-fifo-presentation.md)).

Measured again on main once Mailbox had landed (`6f37db8c`), two rounds each:

| Setup | Frames per second | Redraw span median / p95 | Gap median / p95 / max |
| --- | ---: | --- | --- |
| Native Wayland, GPU, Mailbox | 60.0, 60.0 | 3.8 / 4.5–4.7 ms | 16.7 / 16.7–16.8 / 17.0 ms |
| XWayland, GPU | 60.4, 60.4 | 16.6 / 17.1–17.2 ms | 16.6 / 17.2 / 18.3 ms |

The idle window before About drew 2 frames in 1 s on native Wayland, as before.

To repeat the measurements:
- `cargo build --locked -p midnight-ui --example native_about`, then run it:
  - as built;
  - with `WAYLAND_DISPLAY` unset;
  - with `MIDNIGHT_PROBE_RENDERER=software`.
- `cargo build --locked -p midnight-viewer-ui --example open_latency`, then:
  - place it beside the build's `viewers` folder;
  - run it with `MIDNIGHT_PROBE_SHARED_WGPU=1` and a one-line text file.

Run both with private XDG folders.

## Archive stress (A13)

`viewers_archives` at the stress sizes passed in run `20261125-190420`, all 38 checks
in 3,236 s. F5 out of `flat.tar` copied every one of its 100,001 files twice:
- the folder, in 1,480.7 s;
- a Ctrl+A selection, in 1,653.6 s.

That is 60–68 files per second, within the 40-minute budget for each (`26c18895`).
Builds of another task ran on the host meanwhile. Windows (WIN-8) has not run.

## Still open

- **SCROLL-02:** fast spins over wide rows under load.
- **2c, 7c and 8:** the designs are not yet implemented (the window pool is for Windows only).
- **WIN-8:** the archive stress check on Windows.
