# Viewer and archive opening speed — 19–20 November 2026

The user reported ~1200 ms to open a one-line text file, slow HTML scrolling, very
slow image loading, an HTML file that did not open at all, and slow browsing of
`.tar.gz` archives in the debug application. The debug application loaded viewer
plugins and an archive worker built on 13 November with the unoptimized `dev`
profile. Findings from the acceptance suite's runs on that code each have their own
section: the HTML teardown failure (V1), the opening floor and stalls, broken images (V4),
a text file named `.zip` (V5) and the archive watchdog stall (V11). The HTML viewer's
failure under AppArmor's user-namespace restriction has its own section too.

## Method

All numbers are from one Linux desktop (32 cores, a discrete GPU with its vendor driver, Xorg with
GNOME Shell, X11 session) and are individual runs, not confidence intervals.

- `crates/midnight-viewer-ui/examples/open_latency.rs` opens each file three times
  in one process through `ViewerWindows::open_with_ready` (the application's path)
  and reports the time to the accepted first content frame. It selects the
  application's Linux renderer (winit + femtovg-wgpu), prepares an image spare like
  the application, and can share one wgpu device (`MIDNIGHT_PROBE_SHARED_WGPU=1`) or
  use the software renderer (`MIDNIGHT_PROBE_RENDERER=software`). "Debug probe" means
  the workspace `dev` profile (the debug application's profile); "release probe" the
  release profile. The executable sat beside a `viewers/` directory holding either
  the stale 13 November debug plugins or optimized plugins built on 19 November.
- `plugins/midnight-viewer-image/examples/decode_timing.rs` times the image
  provider's decode and first fitted render (1000×700) without any host.
- Phase timestamps came from temporary, uncommitted instrumentation of the engine,
  UI and image actor, from `MIDNIGHT_VIEWER_OPEN_TRACE=1`, and from the host's
  `startup-diagnostics` feature. `perf` is not permitted on this machine, so stack
  samples were taken under `gdb` and syscall timelines with `strace`.
- Fixtures: a 12-byte one-line text file, a generated 2000-paragraph script-free HTML
  page, and the system wallpapers in `/usr/share/backgrounds` (3840×2160 progressive
  JPEG, PNG, WebP and baseline JPEG).

From about 23:35 the monitor was off (DPMS). Every Vulkan present then blocked for
about one second in the driver (`vkQueuePresentKHR` → `nanosleep`), so GPU
presentation timings after that time are invalid and are not reported. Later runs
use backend timestamps or the software renderer, as stated.

## Causes

1. **Unoptimized plugins.** The image provider's decoders and its area filter ran
   at opt-level 0. The same plugin source decoded 3840×2160 images 12–60× slower and
   rendered the first fitted view 34× slower in the old `dev` profile:

   | Image provider, `decode_timing` | old `dev` decode / fit | new `dev` decode / fit | release decode / fit |
   | --- | ---: | ---: | ---: |
   | Progressive JPEG | 7133 / 1307 ms | 304 / 40 ms | 294 / 37 ms |
   | PNG | 1029 / 1316 ms | 86 / 39 ms | 74 / 35 ms |
   | WebP | 5410 / 1303 ms | 397 / 38 ms | 383 / 47 ms |
   | Baseline JPEG | 5861 / 1303 ms | 97 / 37 ms | 100 ms / — |

2. **A new wgpu instance, adapter and device for every window.** Without a
   configured device, Slint enumerates adapters while constructing each window
   (`ViewerWindow::new` took 88–136 ms in the debug probe) and creates an instance,
   adapter and device when it is shown. Native window creation then still costs
   about 250 ms: creating the Vulkan swapchain makes about twenty requests of the
   GPU driver's GLX extension to the X server, each answered after about one
   compositor frame (~14 ms).
3. **Image decoding waited for the window.** The engine started the image provider
   only after the frontend acknowledged its canvas, i.e. after native window
   creation and the first frame, although decoding needs no viewport.
4. **Per-pixel fit filter.** The provider's area filter recomputed coverage and used
   bounds-checked pixel access for every source pixel of every output pixel.

Not causes: the text plugin, file reading and text layout together took 5 ms after
the canvas acknowledgement. Font discovery (30 ms, once per process) and the text
renderer's font context (27 ms per window) run in parallel with window creation.

## Fixes

- Every plugin workspace's `dev` profile optimizes all dependencies at level 3 and
  the plugin crate at level 1 (image and PDF: 2). Debug assertions, overflow checks
  and debug info stay; release profiles are unchanged. Building optimized plugins
  from `build.sh debug` was rejected: it would remove debug checks and symbols from
  plugin development and add fat-LTO link times to every debug build. The rationale
  is in the README's build section.
- The Linux application creates one wgpu instance, adapter and device before
  selecting the backend and passes it to Slint (`WGPUConfiguration::Manual`), using
  Slint's own backend masking, adapter environment variables and default device
  descriptor. A CPU-only adapter keeps Slint's existing software fallback, an
  explicit renderer or `--software` is unchanged, and `MIDNIGHT_SHARED_WGPU=0`
  restores per-window devices. One reference is kept for the process lifetime: Slint
  drops its copy in a thread-local destructor at exit, where releasing the device
  aborted the probe. Windows still uses its explicit DX12 automatic configuration.
- The engine starts the image or PDF provider when it selects the layout. The worker
  is held with the pending opening, adopted on acknowledgement, and retired through
  the existing closing-image path (never joined on the viewer worker) when the
  negotiation is abandoned. Viewport requests still wait for the acknowledgement.
- The provider's area filter is separable with precomputed column and row coverage.
  Its unit test compares every output pixel with the previous direct filter across
  four images and six viewports (fractional origins, panned edges, transparency).

## Results

### Text (one-line file), femtovg-wgpu, display on

| Configuration | First content, three openings |
| --- | --- |
| Debug probe, stale debug plugins, per-window device | 907 ms; another run 731 / 545 / 580 ms |
| Debug probe, release plugins, per-window device | 600 / 560 / 542 ms; 642 / 570 / 566 ms |
| Debug probe, release plugins, shared device | 292 / 252 / 289 ms; 318 / 249 / 282 ms |
| Release probe, release plugins, per-window device | 959 / 519 / 540 ms |
| Release probe, release plugins, shared device | 301 / 268 / 322 ms |

With the shared device `ViewerWindow::new` takes 4 ms. The release-probe timeline
of an opening is: window constructed 4 ms, shown 5 ms, renderer ready (native
window and swapchain) 250–330 ms, first frame 11 ms, canvas acknowledgement,
plugin session and layout 6 ms, content frame 11 ms. The 200 ms target is **not**
met on this desktop: the remaining time is native X11 and GPU-driver window and swapchain
creation, which no viewer code runs. Reusing a hidden viewer window (X11 keeps the
surface while unmapped) is the remaining option and needs a separate design.

### Images, femtovg-wgpu, display on (debug probe, shared device)

| Image | Stale debug plugins | 19 Nov release plugins |
| --- | ---: | ---: |
| Progressive JPEG | 9084 / 8567 / 8518 ms | 827 / 799 / 743 ms |
| PNG | 2361 / 2369 / 2370 ms | 473 / 510 / 493 ms |
| WebP | 6849 / 6778 / 6931 ms | 809 / 853 / 885 ms |

With the release probe and release plugins: progressive JPEG 957–960 ms with
per-window devices and 593–607 ms (979 ms cold) with the shared device; PNG
718–755 ms and 397–447 ms.

### Images, engine publication of the complete first frame (display off)

Time from opening to the engine publishing the complete first fitted frame, before
(19 November engine and provider) and after (early provider start and separable
filter), femtovg-wgpu, three openings each. The canvas acknowledgement arrived
after 88–134 ms in both.

| Image | Before | After |
| --- | ---: | ---: |
| PNG | 347 / 309 / 306 ms | 262 / 231 / 226 ms |
| Baseline JPEG | 391 / 353 / 351 ms | 329 / 265 / 279 ms |

In an "after" progressive-JPEG trace the provider had decoded at 194 ms, before the
UI acknowledged the canvas; all twelve 256×256 tiles followed within 60 ms. Long
decodes (progressive JPEG 300 ms, WebP 380 ms) now overlap window creation instead
of following it. The display-off state prevented an end-to-end after measurement.

### Final code, software renderer (display-independent)

With the software renderer, no Vulkan swapchain is created, so these runs show the
viewer's own opening cost with the final engine, the new image provider and the
19 November release plugins (debug probe, three openings, while other sessions'
builds loaded the machine):

| File | First content |
| --- | --- |
| One-line text | 79 / 43 / 44 ms |
| Progressive JPEG | 444 / 447 / 435 ms |
| PNG | 228 / 227 / 235 ms |
| WebP | 526 / 535 / 529 ms |
| Baseline JPEG | 268 / 266 / 278 ms |

Images are now bounded by their decoders. Text needs 43 ms once the window exists;
the remaining text latency with femtovg-wgpu is the native swapchain creation above.

### HTML scrolling (software renderer, 972×592 content)

Page Down to the first changed frame accepted by the UI took 20–35 ms with both the
stale debug and the release HTML plugin. Each Page Down produced about ten frames,
17–21 ms apart, finishing after ~170 ms. No transport, conversion or UI preparation
bottleneck appeared; the UI copies each frame once with an optimized `memcpy`. With
the GPU renderer, frames are additionally paced by presentation, which could not be
measured with the monitor off. One wheel notch scrolls Chromium by 40 px × scale
(`web_input.rs`), well below desktop browsers; the wheel step belongs to the scroll
input layer and is reported to its owner rather than changed here.

## HTML file that did not open

The coordinator's acceptance suite then reported that F3 on every local `.html` and
`.htm` ends with "Content resource teardown is unconfirmed" (`docs/page.html` with
local assets and `docs/plain.htm`, after 227–243 ms), while every other viewer kind
opened.

Cause: the relative-resource capabilities that the application attaches to every
local capture (`captured_directory_resources`: `Scope` and the `Asset` sources it
creates) and the archive member capture (`capture_archive`: `Resources` and `Member`)
did not implement `release_on_worker`. The trait defaults deliberately fail closed:
they leak the reference and return "Content resource teardown is unconfirmed" or
"Content source teardown is unconfirmed". The HTML host releases every served asset
and finally its main document capture through that method, and any failure fails the
HTML session; the engine then keeps the failed retirement for the viewer window, so
every later file opened in that window (Space and Backspace step through the 27
CMake release-note pages in the reported folder) fails with the same message. The
reported page's stylesheets and scripts are in `../_static`, outside the resource
scope, so it needed no asset release of its own. Earlier `open_latency` runs did not
see the failure: they closed each window after its first frame and did not read the
status line.

Fix: each of the four implementations now gives up a shared reference immediately;
the last reference, which `Arc::into_inner` grants to exactly one of any concurrent
callers, drops its retained native handles on the calling backend worker and releases
its nested main-document capture (`Scope`, `Asset`) or its mount reference through the
mount's own protocol (`Member`, `Resources`). Every other production implementor
already had a release (native filesystem `Content` and `Snapshot`); `Generated`
(file lists) keeps the fail-closed default because its source has no confirmation
protocol and only the text viewer, which drops captures, consumes it.

Earlier checks: the page opened in every configuration tried with `open_latency`
(13 November debug and 19 November release HTML packages, built-in and
application capture, software and femtovg-wgpu renderers), neither package failed
trust checks (608 entries each), and the user's preferences disable no viewer plugin.
Separately, `Presenter::viewer_launch` used to decline silently while viewer settings
were being read, while the panel was still listing, while the quick search did not
match the focused entry, and for links, special files and members of a closed archive;
`viewer_unavailable_reason` now gives a visible reason for each.

Final acceptance run (coordinator, host desktop session, on the integration tree with
this work up to the spool-staging fix): 50 scenarios passed, 3 warned, 0 failed. HTML
opens and renders with its local stylesheet and in-folder image. The suite flagged
both HTML openings as slow against its 500 ms budget: `docs/page.html` 985 ms and
`docs/plain.htm` 966 ms from F3 to content (debug test build, software renderer); the
coordinator attributes this to Chromium startup. In this work's `open_latency` runs
(software renderer, debug or release HTML package), every HTML opening took about as
long, not only the first: six openings in one process took 0.84–0.93 s each from the
start of the opening to the first frame, 37 ms of it provider validation. Each HTML
session then starts its own Chromium process with a private profile
(`midnight-web-viewer-host/src/session_actor.rs`) and waits for its process tree when
the session ends; how the rest splits between that start and the page itself was not
measured. Starting one Chromium ahead at startup would therefore help only the first
opening. Later openings would need a started browser kept ready for the next session,
a design question for the web viewer host (memory, idle shutdown, revalidation, profile
isolation) that was not attempted here.

## Text opening checks from the acceptance suite

The suite's first run (01:36 UTC, debug build, software renderer) took 6729 ms for
`text/huge.log` (1 MiB, 9430 lines) and 1765 ms for the first source file,
`code/main.rs`; its 01:54 run took 225 ms and 193 ms for the same files.
`open_latency` (software renderer, three openings) with the basic and source plugins
built in the old `dev` profile, the new one, and release:

| File | Old `dev` profile | New `dev` profile | Release |
| --- | --- | --- | --- |
| Suite's `text/huge.log` | 128 / 91 / 91 ms | — | 112 / 81 / 84 ms |
| Generated 1 MiB log (8384 lines) | 140 / 99 / 105 ms | 127 / 86 / 86 ms | 127 / 86 / 86 ms |
| `main.rs` (43 KB), first then later openings | 944 / 56 / 56 ms | 70 / 55 / 53 ms | 58 / 54 / 51 ms |
| One-line text | 40 / 39 / 39 ms | 41 / 41 / 41 ms | 41 / 41 / 40 ms |

The first source opening paid the unoptimized Source plugin's one-time
initialization, which the new profile removes (944 → 70 ms). The 1 MiB log does not
take seconds with any build; the suite's 6.7 s happened once, for a fixture written
seconds before the run, and was not reproduced.

## Opening floor and intermittent stalls

The acceptance suite (debug build, software renderer, load average about 8 from parallel
builds with heavy disk writes) opened every file in 192–260 ms, with occasional stalls
of 0.6–6.7 s on different small files in different runs.

Cause: every viewer opening, of any kind, discovered the installed video support
(`VideoInstallation::discover`), which validates every file of the `thumbnailers`
package (726 in the development installation): an `lstat` of each ancestor and a full
canonicalization per file, about 15,000 metadata system calls. Every viewer window also
built its own fontconfig-backed font context. `open_latency` with the application's
capture path, with and without the `thumbnailers` package beside it (three openings):

| File | Without `thumbnailers` | With `thumbnailers` (before) | With `thumbnailers` (after) |
| --- | --- | --- | --- |
| One-line text | 82 / 55 / 45 ms | 183 / 138 / 141 ms | 143 / 20 / 19 ms |
| 4 KiB binary | 76 / 76 / 85 ms | 169 / 159 / 167 ms | 47 / 46 / 45 ms |
| `main.rs` (66 bytes) | 48 / 42 / 42 ms | 144 / 137 / 137 ms | 22 / 20 / 18 ms |
| `config.json` | 42 / 41 / 52 ms | 134 / 134 / 133 ms | 17 / 17 / 18 ms |
| `README.md` | 48 / 44 / 58 ms | 141 / 129 / 131 ms | 22 / 19 / 19 ms |

Under a writer that repeatedly wrote 768 MB with `fsync` (traced with `strace`, which
adds its own cost), opening a freshly written 4 KiB file took 1.9–6.9 s on every
opening before the change. After it, only the first opening in a process took
2.5–2.9 s; later openings took 61–71 ms (one 568 ms). No single system call took more
than 0.2 s: the time was spread over the metadata calls of the package walk. No viewer
opening path calls `fsync` or `fdatasync`; the remaining syncs in the host are the
thumbnail cache, settings saves and file transfers.

Fix: the engine discovers the video installation once per service, in the same
preparation as the plugin catalogs (launching still revalidates the package and pins
its code), and text renderers share one system font database. The application's
startup preparation (`prepare_image_workers`) now also prepares both and the installed
font catalog, so the first viewer opening does not pay them: in two fresh processes the
first opening of the one-line file took 27–29 ms instead of 143 ms. The native filesystem
host still admits seven provider calls at a time for the whole application; a viewer
capture waits when seven slow calls are in flight. That limit was not changed.

### Against the acceptance suite's numbers

The suite's runs on the old code are the baseline. Its timings run from F3 to the
viewer's accessible "File content" value; the figures here come from `open_latency` and
`measure_open`, so they are not the same measurement. A suite run on this code gives
the like-for-like number. The coordinator's final full run passed 50 scenarios with 3
warnings and 0 failures; its HTML timings are under "HTML file that did not open", and
its timings for these rows were not passed on to this work.

| Suite, old code | Cause found | After |
| --- | --- | --- |
| Floor: median 193 ms over 15 openings of a 9-byte file, the same for the text, source and Markdown plugins | The video-support walk and the per-window font context run on every opening, whatever the plugin; that is why the plugins did not differ | 17–22 ms for small text, source, JSON and Markdown files (129–183 ms before, same probe); first opening in a new process 27–29 ms |
| Stalls of 0.6–6.7 s in 3 of 5 runs | The same walk, about 15,000 metadata calls, waiting behind other programs' disk writes | 61–71 ms per opening (one 568 ms) under a writer that `fsync`s 768 MB in a loop; 1.9–6.9 s per opening before |
| `source.tar.gz` listed in 3.9 s once | The fixture is a few KB (8 members, the largest 4 KiB), so decoding is not the cost. Creating a spool file waits for ext4 journal commits that other programs' `fsync` calls force (see "Archive browsing"); the old code also flushed each sealed spool | A copy of the fixture: mount 11–14 ms, listing 0.3 ms. Next to a 1.5 GB `fsync` writer, one process mounted it 105 times in 11.0–12.3 ms. The spool root is now checked in the background at startup (45 checks, about 0.2 ms each); 24 of 25 first mounts took 11.2–11.9 ms, and one waited 14.1 s while the worker process started (see "Archive browsing") |

The startup preparation does the walk once, in the background. If F3 comes before it
finishes on a busy disk, that first opening waits for the walk to finish.

## Broken images and files that are not ZIP archives (V4, V5)

V4: a damaged image (the suite's cut PNG and zero-byte PNG) reported "Image decoder
rejected the source [image cleanup: Confirmed]". The bracket came from the image host's
cleanup error formatting (`worker_lifetime.rs`). The error text is now the original
failure only. The cleanup state stays typed (`cleanup_status`), and the engine already
uses it to keep the capacity of a worker whose exit is unconfirmed. The refusal now reads:

- Images: "This image is damaged or incomplete, or its format is not supported."
- PDF documents: "This document is damaged or incomplete, or its format is not supported."

A view that cannot be drawn after a successful open reads "This image could not be
drawn." or "This page could not be drawn; the document may be damaged."

The decoder's own short reason is not available. The image worker protocol's `Failure`
message has no text, and the SDK drops the decoder's error. Passing it on needs an
image-viewer API change: bounded text derived from untrusted content. That is a
decision for the Architect.

V5: a text file named `.zip` reported the ZIP reader's "invalid Zip archive: Could not
find EOCD". The archive worker now reports "This file is not a valid ZIP archive." on
the first line, with the reader's wording on the second. The application shows the
first line as the reason and the rest in the notice details. Other ZIP errors keep
their messages, including unsupported features and I/O failures. Through the host
(`measure_open` with the staged worker, a text file named `notes.zip`), the failure
arrives as operation `archive`, kind `InvalidData`, with the message's first line "This
file is not a valid ZIP archive." and its second "invalid Zip archive: Could not find
EOCD".

## V11: archive member read stalled by the archive watchdog

The suite's old build stalled 23.7 s on F3 of a 19-byte member of `bundle.zip` and
failed with "Archive worker made no progress for 20 seconds and was stopped" while the
disk was busy. Sealing each staged member and scratch stream flushed it with
`fdatasync` (0.45–10.7 s measured above); the spool change removes that wait. Nothing
else on the archive staging and member-read path syncs: the worker has no file access,
the host's staging appends with ordinary writes, and reads verify chunk CRC-32 values.
Creating a spool file could also wait for seconds on a busy disk (see the end of
"Archive browsing"). Members of up to 16 MiB are now staged in memory and create no
file. A larger member still creates one, and so does the spool-root check, which the
engine now runs in the background at startup, so on a disk as busy as the one measured
there the watchdog can still stop a worker reading a large member.

## Archive browsing (`.tar.gz`)

`crates/midnight-filesystem-host/examples/measure_open.rs` mounts an archive through
the installed worker exactly as the application does (debug host profile, three new
mounts each). Fixtures: `usr-include.tar.gz` (16 MB, 91 MB decoded, 7504 entries,
from `/usr/include`) and `python3.tar.gz` (64 MB, 283 MB decoded, 16,358 entries,
from `/usr/lib/python3`).

Causes, in order of cost:

1. Sealing the decoded scratch stream called `fdatasync`: 0.45–0.64 s for 91 MB and
   1.37–1.65 s for 283 MB on a quiet disk, up to 10.7 s while other builds wrote.
   The file is a temporary, delete-on-close spool; the flush existed to make write-
   back errors observable. Member stages were flushed the same way on first read.
2. After staging, the worker indexed the TAR by reading the staged stream back over
   IPC (its read window grows to 256 KiB, so practically all of it): 0.26 s / 0.79 s
   against 0.02 s / 0.04 s for the same index in memory.
3. The debug worker's unoptimized decompressors: decoding plus index in memory took
   1.35 s / 4.63 s instead of 0.23 s / 0.78 s.

Fixes: spool reads verify a CRC-32 recorded for each appended chunk (at least
256 KiB) instead of flushing at seal, so lost or failed write-back is a read error and
never content; the worker indexes compressed TAR headers while staging each decoded
block once, skipping member data instead of reading it back; and the archive
workspace's `dev` profile optimizes its dependencies like the viewer plugins.
Within an open archive, navigation already reused the mount; leaving the archive and
entering it again still decodes it again.

| Configuration | usr-include.tar.gz open | python3.tar.gz open |
| --- | ---: | ---: |
| Before: unoptimized worker (the user's setup) | 2.62–2.77 s | 8.37–9.22 s |
| Before: release worker | 1.60–1.65 s | 5.08–5.52 s |
| After: debug worker with the new `dev` profile | 0.51–1.03 s | 1.70–1.73 s |
| After: release worker | 0.49–0.52 s | 1.65–1.67 s |

The rows ran on a loaded machine (load average 6–12), "after" twice; one opening per
run of the smaller archive took about 1 s (2.0 s once) instead of 0.5 s. Remaining
time is decoding (0.21 s / 0.74 s in memory with miniz_oxide) and staging traffic.

**Stall in creating a spool file (fixed for content up to 16 MiB).** A copy of the suite's `source.tar.gz`
(4547 bytes) through the staged worker mounted in 11.2–11.5 ms and listed in
0.27–0.31 ms on a quiet disk. Next to a writer that wrote 1.5 GB with `fsync` in a loop
(up to 1.3 GB in write-back), 3 of 45 mounts took 3.3–7.8 s and the rest 11–14 ms. In
60 more mounts traced with `strace` (output on tmpfs), 6 took 0.49–11.9 s. Every system
call over 0.3 s, 0.32–11.8 s each and one per slow mount, was the host creating a spool file in
`~/.cache/midnight-explorer/archive-spool`, `openat(dir, ".", O_TMPFILE | O_EXCL | O_RDWR)`.
On ext4, creating that file needs a journal handle, which waits while the journal commits
the other program's `fsync`, including its data. Each mount created one such file to
check that the root can hold anonymous files (`Staging::open_first`), and one more for
each staged stream or member (for a `.tar.gz`, its decoded stream). In a first traced attempt,
with `strace` writing its log to that volume, a 10-byte `write` to a new file also blocked
for 4.8 s.

Fix, as the coordinator decided:
- A stream or member of at most 16 MiB is staged in memory while a 128 MiB budget,
  shared by every archive mount of the process, has room.
- Larger content, and content arriving once the budget is used up, goes to a spool file
  as before. A stage that outgrows a limit moves its bytes there, and those bytes count
  against the volume's reserve.
- The process checks the spool root once and keeps it until a spool fails in it.
- Follow-up the user approved: the engine makes that check on a background thread right
  after plugin discovery (`Catalog::prepare_staging`), so the first archive opened does
  not wait for it. A mount that starts during the check waits for its result.

Same fixture, same writer, after the fix:

| Run | Before | After |
| --- | --- | --- |
| 15 processes × 3 mounts, untraced | 3 of 45 took 3.3–7.8 s; the rest 11–14 ms | 2 of 45 took 2.4 s and 8.0 s, both the first mount in their process; the other 43 took 11.0–12.8 ms (median 11.5 ms) |
| 20 processes × 3 mounts, `strace` | 6 of 60 took 0.49–11.9 s, each in one `O_TMPFILE` creation | One `O_TMPFILE` creation per process, the root check. 2 of 60 took 6.3 s and 6.9 s, both first mounts waiting in it; later mounts created no file |
| One process, 105 mounts | — | 11.0–12.3 ms (median 11.1 ms), none over 100 ms |

Follow-up, checking the root at startup: the engine now checks the spool root on a
background thread right after plugin discovery. `measure_open` makes the same check
after discovery, then mounts three times. Same fixture and writer:

| Run | Before (the first mount checks the root) | After (checked at startup) |
| --- | --- | --- |
| 15 processes, archive opened 2 s after start | 2 of 15 first mounts took 2.4 s and 8.0 s | Check 0.19–0.22 ms in the background; 14 first mounts 11.5–11.9 ms, 1 took 14.1 s (see below); later mounts 11.0–13.3 ms |
| 10 processes, archive opened at start | — | Check 0.19–0.21 ms; first mounts 11.2–11.7 ms, later 11.0–11.7 ms |
| 20 processes, `strace` on the mounting thread | 2 of 20 first mounts waited 6.3 s and 6.9 s in the check's `O_TMPFILE` | No `O_TMPFILE` in the mounting thread; check 0.17–0.23 ms; 19 first mounts 165–200 ms (traced), 1 took 1.16 s |

None of the 45 startup checks waited this time, so these runs show that the check left
the first mount, not how long a slow check takes (2.4–8.0 s in the runs before). The two
slow first mounts that remain did not wait for the spool. Both waited while the worker
process started: in the traced one, the only system call over 0.3 s was the worker spawn,
`clone3(CLONE_VFORK)` for 0.99 s, which returns once the child has executed the worker
binary. Both ran a worker binary copied just before their process started: the traced
runs copied it before every process, the untraced runs only before the first, which is
the one that waited. The 24 untraced first mounts whose binary had been copied about
20–80 s earlier took 11.2–11.9 ms. Reads and executions that had to update a file's access time
did not wait in 40 tries each under the same writer, so the cause is not identified.

## HTML viewer under AppArmor's user-namespace restriction

The coordinator reproduced "Chromium transport stopped: failed to fill whole buffer" for
both HTML files with `me` running as an unconfined systemd user service. Ubuntu 24.04
sets `kernel.apparmor_restrict_unprivileged_userns=1`: an unconfined program's user
namespace gets no capabilities, so the uid map that Chromium's namespace sandbox writes
is refused (`unshare --user --map-root-user` there: "write failed /proc/self/uid_map:
Operation not permitted"), and the package's `chrome-sandbox` is not setuid root, so
Chromium exits before its first message. Runs started from the Claude desktop app had
passed because its processes run under that app's `claude-desktop (unconfined)` profile,
which grants `userns` and which Chromium inherits; the documented qualification ran on
WSLg, which has no such restriction.

Fix:
- `scripts/install-html-viewer-apparmor.sh` installs
  `/etc/apparmor.d/midnight-explorer-html-viewer`, a profile in the form of Ubuntu's own
  browser profiles (`abi <abi/4.0>`, `flags=(unconfined)`, `userns`,
  `include if exists <local/...>`), for one installation's viewer executable or for
  every build under a checkout's `_build/` (the attachment of the manual profile the
  coordinator gave the user, which it replaces). It needs root and says so; `--print`
  shows the profile and `--uninstall` removes it. Linux release archives carry it next
  to `me`, and the release README and system-requirements.md name the step: the tarball
  has no post-install step to run it from.
- The Linux browser launcher (`midnight-worker-platform/src/browser_sandbox_linux.rs`)
  checks before the browser can use its sandbox. When the restriction is on and
  `chrome-sandbox` is not setuid root, it reads the started browser's AppArmor label
  right after `exec`; when that label is plain `unconfined`, it runs the same `unshare`
  probe in a child process. If the probe is refused, it stops the browser and returns
  `SandboxUnavailable`, and the HTML viewer shows: "The HTML viewer needs permission for
  Chromium's sandbox on this system (Ubuntu restricts unprivileged user namespaces). Run
  once: sudo <installation>/install-html-viewer-apparmor.sh". A probe of the
  application's own environment alone cannot decide this: the application stays
  unconfined after the profile is installed, because the profile attaches to the viewer
  executable when it starts, so only the browser's own label shows whether the fix is
  in place.

Checked with the real HTML viewer through `browser::spawn` (a temporary probe program):

| Environment | Viewer path | Result |
| --- | --- | --- |
| Unconfined systemd user service | Hard-linked copy outside the installed profile's pattern | Stopped with `SandboxUnavailable`; the message named that copy's `install-html-viewer-apparmor.sh` |
| Unconfined systemd user service | `_build/debug/viewers/...`, covered by the installed `midnight-explorer-html-viewer` profile | Browser started |
| This session (`claude-desktop (unconfined)`) | Both | Browser started |

The script ran as a normal user: `--print` for a package root, a package folder, a path
with spaces and a checkout produced profiles that `apparmor_parser -QTK` compiled (a
deliberately broken profile was refused); a path with `*` was refused; install and
uninstall without root stopped with the `sudo` command to run. Its root-only install and
uninstall steps were not run here, because they change the system's AppArmor policy.

## Markdown viewer stuck at a tall table

The user's Markdown viewer stopped scrolling inside `docs/implementation-backlog.md`
(status "Bytes 68698–82097 / 119796", on the table rows PERF-01, UI-PERF-01 and
META-04, just above "Deferrals and residuals of the 18 November review"). Reproduced
through the engine (`Service`, 1000 × 700 viewport) with the integration tree's copy
(119,817 bytes) and the installed Markdown plugin: wheel steps (40 px) and Down (28 px)
stopped for good at 18 % (Bytes 19,367–23,535, just before the main backlog table),
with and without the retained raster, and so did the application's own retained-raster
path (`PanTo` targets). PgDn and End got past.

Cause: the provider keeps a table whole as one layout unit, and a window starts with the
unit that holds its anchor (`semantic_window::groups` and `select` in the Markdown
plugin). Scrolling past the bottom of a page asks for the next window anchored at the
new viewport top, which still lies inside that page. When the unit after the page does
not fit the request budget beside the units already selected (here the wire limits for
the anchors and cells of two large tables), the provider returns the same window. The
engine restored the camera, clamped it to that same page and asked again on the next
step: in the traced run every request was anchored at byte 21,768 and every reply was
bytes 19,367–23,535. A retained-raster refill did the same and then stopped asking (its
guard against identical refills). The provider behaves as designed; the engine had no
handling for a reply that does not advance.

Fix (`midnight-viewer-engine`): a continuation that leaves a page because the camera
moves past its edge remembers that page (`Continuation::bounded_by`). This applies to
wheel and key scrolling and to retained-raster refills whose camera lies outside the
page; prefetch refills inside the page are unchanged. When the reply does not reach past
that edge (`Continuation::seam`), the worker asks once, after the reply has completed,
for the page that starts at the edge (or ends there, backwards). Its first line comes to
the viewport top (its last line to the bottom), as a page step would, without skipping
content. That request carries no continuation, so a reply that cannot advance either
ends there, and deferred motion waits for it.

After the fix, same file and plugin: wheel, Down and PgDn, with and without the retained
raster, reach the end (position 1.0, Bytes …–119,817). The retained-raster pan reaches
the content end (camera 7,960.4, the reported limit), crossing the table at 23,535 and
the user's table at 68,719–82,118. There one page, bytes 82,118–82,778, holds only a
heading and a paragraph and is shorter than the viewport, so that frame has no retained
pixels and the window sends a plain scroll step, as it already does without them. End
and dragging the scrollbar to 75–100 % reach their targets.

Dragging the scrollbar to 69–71 % failed with "The requested viewport was not
completed." both before and after that fix. Those fractions lie between the last camera
of the table's window (68.0 %) and the first camera of the next window (about 71.5 %).
No window can show them at that camera: the table stays whole, and the short page
between them holds only a heading and a paragraph. After its one correction read, which
landed in one of the two windows again, the scrollbar position gave up. In a semantic
document it now settles at the loaded window's nearer edge on the side where the
document continues (`nearest_edge` in `worker_position.rs`, now for every mode; see
the next section) and acknowledges the position there. A sweep in 1 % steps from 60 to 80 %: 60–68 % land exactly, 69–71 %
settle at 68.0 % with each drag acknowledged and no error, 72–80 % land exactly.

## Scrollbar drags in plain text and source files with long rows

The user's scrollbar drags in `third-party-inventory.csv` (562 lines, 121,741 bytes, rows
up to 1,515 bytes, basic viewer) sometimes ended with "The requested viewport was not
completed.". Reproduced through the engine (`Service`, 1000 × 700 viewport) with the
integration build's plugins: of the drags from 0 to 100 % in 2 % steps, those to 12, 14,
44 and 52 % failed and were not acknowledged; all others landed.

Cause: the basic viewer reads at most about 8 KiB for one Text page (`READ_BYTES` in its
`decode.rs`) and the source viewer at most 8 KiB (`paging.rs`), whatever the engine asks
for (12 KiB). Eight KiB of this file's rows, about 180 bytes each, is just one viewport
tall: the traced windows were 700.8 px tall for the 700 px viewport, so their cameras
moved by less than a pixel and covered a sliver of the scrollbar 0.00001–0.00005 wide.
A drag's read, anchored from the previous window's extent, missed that sliver by a few
bytes (at 12 % the window's lowest position was 0.12001, at 52 % its highest was
0.519998). The one correction read asked for twice the lines, the byte cap answered with
the same window (backwards: 4 bytes shorter), and the position gave up.

The same content in the source viewer (the file copied as `.sql`) failed at 28 of the 51
positions. Its 8 KiB windows start inside a row, and a Source camera must lie below the
window's first complete line (`source_minimum_origin`, 16.3 px here), but these windows
allow only 0.77 px of camera travel: no camera qualified at all. The reread for that
leading context got the same window from the byte cap, and the drag failed. A failed
candidate is never shown, so later drags were estimated from the last shown window's
extent (0.0653 instead of 0.0673) and missed too: every drag from 52 to 98 %.

Fix (`worker_position.rs`): the settling at the nearest edge that Markdown windows got
(previous section) now applies in every mode (`nearest_edge`). When a completed reply
still cannot show the position after its bounded correction (one corrective read for
text and documents, the rereads `SourceRecovery` allows for source), the view at the
window's nearer edge, on the side where the file continues, is shown and the position is
acknowledged there; the frame reports the position of that view. A Source candidate
whose cameras all lie above its first complete line uses its whole camera range, as a
Source packet shorter than the viewport already does (`source_fallback_origin`): the
position is resolved there, or settles at the nearer edge. A candidate shorter than the
viewport (684 px at 16 %) has a single camera, its top, which settles the same way when
its byte inverse misses by a few bytes. The source viewer then shows that candidate
instead of keeping the previous page. Two limits keep this to near misses: a file
endpoint (0 or 1) still needs the real file edge, and a text or source reply that misses
by more than its own source span did not load the position's surroundings, so such a
drag still fails as before. A Markdown window has no such limit: beside a whole table,
the band no window shows can be wider than a small window next to it (in the current
backlog file at 20 % and 71 %, beside windows of 1,425 and 926 bytes).

After the fix, same files and plugins, each sweep after End and Home:

- CSV, 0–100 % in 2 % steps: all 51 drags acknowledged with their own sequence and no
  error. The four former failures show 0.1200, 0.1400, 0.4398 and 0.5200; the largest
  difference from a target is 0.0002 (44 %).
- The same content in the source viewer, 0–100 % in 2 % steps: all 51 drags
  acknowledged with their own sequence and no error (before: 28 refused). Four settled
  at the nearest view: 14 % at 0.1399, 16 % at 0.1601 (a candidate shorter than the
  viewport), 44 % at 0.4398 and 52 % at 0.5206; all others landed within 0.00005, and
  the drags after 52 % landed again.
- With the retained raster on, as in the application, CSV and source at 2 % steps:
  51 of 51 each, acknowledged and without error.
- In 0.5 % steps (201 drags each): CSV 201 of 201 and source 201 of 201, acknowledged
  and without error; the largest differences from a target were 0.0009 and 0.0007
  (both at 51 %). The raster and 0.5 % sweeps ran before the Markdown exemption, which
  changes nothing for text or source windows.
- The integration tree's current `docs/implementation-backlog.md` (129,860 bytes) in the
  Markdown viewer, 0–100 % in 1 % steps: all 101 drags acknowledged and without error.
  Nine fell in bands beside whole tables and settled at the nearest view: 18–20 % (at
  0.1783 and 0.1840), 43–44 % (0.4244), 58 % (0.5892) and 69–71 % (0.6859–0.6884).
  With the span limit applied to Markdown windows as well, 20 % and 71 % were refused.
- Final code, after that exemption: the CSV and the source sweeps at 2 % steps again
  51 of 51 each, the largest differences 0.0002 (CSV, 44 %) and 0.0006 (source, 52 %).

## Wheel and key travel across byte-capped text and source pages

The viewer acceptance run `20261120-143743` (scenario `viewers_scroll`, plugins
`_build/try-1505`) reported:

- In the source viewer, five wheel notches at once in a Rust file moved 256 px, not
  300.
- After PageUp, twelve Down presses in a CSV, a TSV or that Rust file moved far less
  than 12 × 28 px.
- In the plain-text view (Alt+F3) of the CSV, one notch up moved 32 of 60 px and showed
  the tail of the previous row as a line of its own, and Down jumped.

Reproduced with a temporary end-to-end probe, which is kept outside the tree:

- The real viewer window runs on the test platform and drives the real engine and the
  try-1505 plugins, in a 1340 × 820 window.
- The content area is 1312 × 712 in the source viewer and 1312 × 748 in the plain-text
  view. The font is monospace 14, with line numbers.
- Input is the harness's own: Slint wheel events of 60 px per notch, key presses
  through the viewer's bindings, and scrollbar drags.
- Movement is measured from rendered pixels with the harness's row matching. Each frame
  has held still for 300 ms, and again 700 ms later.
- The platform clock follows real time, and the window is painted on every turn.
- The fixtures are the run's generated `inventory.csv` and `inventory.tsv` (1,500 rows
  of 100–260 bytes, 271,492 bytes) and `long.rs` (6,000 lines).

The before and after runs share one build; the "before" runs have the change below
switched off:

| File, view | Before: wheel | Before: keys | After: wheel | After: keys |
| --- | --- | --- | --- | --- |
| CSV, source viewer | 60, −60, 300 | Down 28, Up −28; PageDown and PageUp could not be measured; 20 × Down 0 px | 60, −60, 300 | 28, −28, 712, −712; 20 × Down 560 |
| TSV, source viewer | 60, −60, 300 | as the CSV | 60, −60, 300 | 28, −28, 712, −712; 20 × Down 560 |
| CSV, plain text (Alt+F3) | down 49; up and five notches could not be measured | Down could not be measured, Up −18, PageDown 918 (screen 748), PageUp could not be measured, 20 × Down 412 | 60, −60, 300 | 28, −28, 748, −748; 20 × Down 560 |

In the plain-text view before the change, 11 of 12 single notches up and 11 of 12
single Down presses did not move the content as one piece (one notch moved −49 px, one
press 18 px). After it, all 12 moved −60 px and all 12 moved 28 px. Before the change,
"could not be measured" means the content changed but no shift matched 80 % of its rows,
as in the acceptance run (PageUp of the TSV: −653 px matching 7 % of the rows here, 18 %
there).

Cause, from engine traces of those runs: the providers' byte budgets.

- **Pages about one screen tall.** The basic viewer sends at most 8,180 bytes of text
  per page, and the source viewer at most 8,192, whatever the engine asks for (12 KiB
  and the rows of the viewport plus its 256 px guards). For these rows that is 44–46
  rows:
  - In the source viewer, pages were 733–766 px tall for a 712 px viewport. That left
    5–38 px of camera travel, and the retained guard shrank to half of it (10.7 px).
  - In the plain-text view, pages were shorter than the 748 px viewport, so they never
    reached retained scrolling at all. Every key went to the engine as a plain scroll.
- **Forward refills that could not advance.** A forward refill is anchored at the
  camera minus the guard. With a guard under one row, it anchored at the page's first
  row again, and the byte cap answered with the same page. The fallback for a reply
  that cannot pass its edge (`Continuation::seam`, written for Markdown tables) then
  loaded the page that starts where this one ends, without a continuation. The view
  jumped about 45 rows, and the new raster surface discarded the rest of the motion.
- **Backward pages starting inside a row**, where the byte budget ends. A source camera
  then handed every backward refill through a raster rebase, about 32 px per round
  trip. At the top of each page it showed the cut row's tail as a line (status
  "Col 135").
- **A refill that never advanced.** After PageUp, a camera at its page's edge waited
  for a refill that the same-page answer never advanced.

In the traced CSV key sequence before the change:

- 20 key presses went to the engine as plain scrolls;
- the camera was cleared 4 times;
- a new surface dropped pending motion twice;
- 40 refills became rebases.

After the change, all four counts were 0.

Fix, in the engine only (`worker_chain.rs`):

- **Holding back a short reply.** When the provider ends a Text or Source reply by
  bytes before the request's line budget, the reply is held back. The engine requests
  the contiguous next range (backwards, the previous one) and joins the replies into
  one page. That page reaches the worker as the reply to the original request, so
  positions, motion and raster continuity work as with a larger page.
- **Stopping and failing.** A chain stops:
  - at the line budget (backwards, one line short of it, as a provider's backward page
    holds one newline less);
  - at 64 KiB;
  - after 16 reads;
  - when a read adds nothing.

  A failed or cancelled continuation delivers what was joined. A chain whose request
  was replaced is cancelled.
- **No cut first row.** A backward chain's first row is dropped unless it is known to
  be whole: at the start of the file, or when the read ended at a row with a clear edge
  flag. So a cut row never shows as a line. A whole row dropped this way is shown by
  the page above it.
- **The source refresh.** Its request is joined the same way, so restyling does not
  shrink the page again.
- **No UI change.** The camera, the raster presentation and the wheel motion behaved
  correctly; they were starved of coverage.

Drags after the change, through the engine (`Service`, 1000 × 700, after End and Home),
all acknowledged with their own sequence and without an error:

- `third-party-inventory.csv` at 2 % steps:
  - source viewer: 51 of 51;
  - basic viewer: 51 of 51;
  - the same content as `.sql`: 51 of 51.
- The acceptance fixture `inventory.csv` at 1 % steps:
  - source viewer: 101 of 101;
  - basic viewer: 101 of 101.
- The integration tree's `docs/implementation-backlog.md` (Markdown, not joined; now
  133,059 bytes) at 1 % steps: 101 of 101. Five settled at the nearest view beside whole
  tables, as before this change: 18 % at 0.1740, 19 % at 0.1923, 42 % and 43 % at
  0.4144, and 68 % at 0.6783.
- With the retained raster on, as in the application, whose requests include the guard
  rows and whose replies are therefore joined:
  - `third-party-inventory.csv` at 2 % steps: source viewer 51 of 51, basic viewer 51 of
    51, `.sql` 51 of 51;
  - `inventory.csv` at 1 % steps: source viewer 101 of 101, basic viewer 101 of 101.

Every drag of the text and source sweeps landed at its target (largest difference below
0.00005). Before this change, some drags of the same CSV settled at the nearest view
instead (up to 0.0002 from the target in the basic viewer and 0.0006 in the source
viewer; previous section).

Limits:

- **Rust burst not reproduced.** The acceptance run's Rust results (five notches
  256 px in 3 of 3 runs, PageUp −792 px) did not reproduce here. All of these moved
  exactly 300 px for the burst, and 28, −28, 712, −712 and 560 px for the keys:
  - the probe on the real-time clock and on a 4 ms test clock;
  - with every page reply delayed by 1.2 s in the engine;
  - with five bursts fired right after drags, while source highlighting was being
    prepared;
  - after the scenario's full 51-drag sweep.

  Rust pages hold the line budget (78 lines, about 3.6 KiB), so this change does not
  touch them. A rerun of the scenario with this change is needed to settle it.
- **Rows longer than about 830 bytes.** With such rows, 64 KiB is less than the line
  budget, so a joined page can still be short of the viewport plus its guards. A row
  longer than 64 KiB is one fragment, as before.
- **Extra round trips.** Each refill of such rows costs one or two more provider round
  trips.

## Tests

All in the project's Docker build image, `--locked`:

- `cargo test -p midnight-viewer-engine --lib worker::tests`: 188 passed, 1 ignored,
  including all 15 `layout_opening` tests and the two new image-provider tests.
- `cargo test -p midnight-viewer-engine --lib` after the release fix: 499 passed,
  7 ignored, including `released_assets_and_scope_confirm_teardown_and_close_their_handles`
  and `released_member_captures_return_their_mount_reference_once`.
- `cargo test -p midnight-web-viewer-host --lib`: 79 passed.
- After the opening-floor changes: `cargo test -p midnight-viewer-render --lib` 25 passed,
  including `shared_contexts_see_the_installed_families_of_a_fresh_context`;
  `cargo test -p midnight-viewer-engine --lib` 499 passed, 7 ignored.
- `cargo test -p midnight-image-viewer-host --lib`: 33 passed, 4 ignored.
- `cargo test -p midnight-explorer --bin me window_backend`: 2 passed, including
  `only_an_explicit_zero_disables_the_shared_device`.
- `cargo build -p midnight-viewer-ui --example open_latency`: built.
- `plugins/midnight-viewer-image/scripts/ci.sh` (offline native tools): formatting,
  decoder and viewer tests (including
  `separable_reduction_matches_the_direct_area_average`, 4 × 6 × 37 × 23 pixels
  compared exactly), all-target Clippy with `-D warnings` and the release package
  build all passed.
- The basic and image plugins build with the new `dev` profile ("optimized +
  debuginfo"). The other plugin workspaces were not rebuilt.
- `cargo test -p midnight-worker-platform --lib spool`: 16 passed, including
  `sealed_reads_return_appended_bytes_across_chunks_and_coalesce_small_appends` and
  `bytes_changed_after_they_were_appended_are_never_returned`.
- Spool users: `cargo test -p midnight-filesystem-host --lib` 75 passed, 1 ignored;
  `cargo test -p midnight-engine --lib -- file_list size_warning private_spool`
  76 passed.
- Archive worker (`plugins/midnight-fs-archive`, dev profile): formatting, every test
  suite (library 28; integration 14, 2, 6, 5, 1, 1, 9, 5, 7 passed; manual benchmarks
  ignored as before), including `compressed_tar_is_indexed_while_staged_and_never_read_back_to_list`
  and `compressed_tar_staging_failures_keep_their_codes`, and all-target Clippy with
  `-D warnings`.
- `cargo test -p midnight-ui --lib viewer_plugins`: 11 passed, including
  `every_declined_view_of_a_file_has_a_visible_reason`.
- `rustfmt --check` on every changed Rust file passed.
- V4: `cargo test -p midnight-image-viewer-host --lib` 34 passed, 4 ignored, including
  `cleanup_state_stays_typed_and_out_of_the_failure_text`; all-target Clippy for the
  crate with `-D warnings` passed. The crate's native tests (`--test native --ignored`,
  run on the host against a private copy of the installed image plugin) passed 3 of 3,
  including `corrupt_content_and_cancelled_sessions_never_return_a_frame`. That test now
  opens text, a PNG cut inside its header and an empty file through the real decoder,
  and expects the plain image sentence for each.
- V5: the archive worker's suites passed again (library 28; integration 15, 2, 6, 5, 1,
  1, 9, 5, 7; manual benchmarks ignored), including
  `a_file_that_is_not_a_zip_says_so_before_the_technical_reason` (a text file and an
  empty file opened as ZIP); formatting and all-target Clippy with `-D warnings` passed.
- Spool staging in memory: `cargo test -p midnight-worker-platform --lib spool` 17 passed,
  including `clones_share_the_directory_and_count_every_created_spool`;
  `cargo test -p midnight-filesystem-host --lib` 82 passed, 1 ignored, including the
  seven `staging::memory_tests`:
  - `a_small_member_stays_in_memory_and_creates_no_spool_file`: no `O_TMPFILE`, and no
    volume query.
  - `a_large_member_goes_to_a_spool_file_and_a_growing_stream_spills_intact`.
  - `a_spill_counts_the_bytes_it_moves_against_the_volume_reserve`.
  - `mounts_share_one_memory_budget_and_return_it_when_they_close`.
  - `concurrent_mounts_never_hold_more_memory_than_the_budget`: 8 threads, 48 members.
  - `the_spool_root_is_chosen_once_per_process_and_again_after_a_failure`.
  - `a_per_process_spool_root_is_checked_by_every_mount`.

  `cargo test -p midnight-engine --lib -- file_list size_warning private_spool` 76 passed.
  All-target Clippy for both crates with `-D warnings` and `rustfmt --check` passed.
  With memory staging and the root cache switched off, 4 of the 7 failed: the small
  member, the spill reserve, the shared budget and the single root check. The other
  three check invariants that also hold with every stage in a file. With the code
  restored (`cmp`), all 7 passed.
- Spool root checked at startup: `cargo test -p midnight-filesystem-host --lib` 83
  passed, 1 ignored, including `a_root_checked_at_startup_spares_every_mount_the_check`
  (a mount that starts during the check waits for its choice and walks no candidates,
  its small member creates no file, and a second check keeps the root) and
  `a_per_process_spool_root_is_checked_by_every_mount`, which now also checks that the
  startup check keeps no per-process root. With the check made a no-op, the first test
  failed at once; with the code restored (`cmp`), both passed.
  `cargo test -p midnight-engine --lib archives` 34 passed. All-target Clippy with
  `-D warnings` (it checked both crates) and `rustfmt --check` passed.
- AppArmor sandbox detection: `cargo test -p midnight-worker-platform --lib browser` 22
  passed, 2 ignored, including the six `browser::native::sandbox::tests` (each decision
  step with injected restriction values, setuid owner and mode, labels and probe results;
  the message for packaged, development, spaced and unknown installations; the typed
  error). `cargo test -p midnight-web-viewer-host --lib` 80 passed, including
  `a_denied_sandbox_is_reported_with_its_own_fix_and_other_launch_failures_keep_the_prefix`.
  All-target Clippy with `-D warnings` (it checked both crates) and rustfmt passed.
  `python3 -m unittest scripts.tests.test_package_release` 14 passed, including
  `test_linux_packages_carry_the_html_sandbox_installer_as_an_executable`. These builds
  used a private target directory under `_build`, removed afterwards.
- Markdown tall-table scrolling: `cargo test -p midnight-viewer-engine --lib` 504
  passed, 7 ignored (in the build container), including five `worker::tests::seam` tests
  on a document whose table is taller than the viewport and the request window: the
  forward and backward continuation at the page edge, a retained-raster pan past the
  page, a reply that does advance (no further request), and a scrollbar position just
  past either edge of the window. With `Continuation::seam` disabled, the first three
  failed; with the code restored (`cmp`), all passed. The scrollbar test returned
  `Unavailable` before the nearest-edge settling existed and `Ready` after.
  All-target Clippy with `-D warnings` and rustfmt passed.
- The V4 and V5 regression tests were checked red and green: with only the two fixes
  reverted, `cleanup_state_stays_typed_and_out_of_the_failure_text` and
  `a_file_that_is_not_a_zip_says_so_before_the_technical_reason` failed; with them
  restored (byte-identical, `cmp`), both passed.
- Scrollbar settling in plain text and source: `cargo test -p midnight-viewer-engine
  --lib` 507 passed, 7 ignored (in the build container), including the two
  `worker::tests::nearest_edge` tests on rows like the CSV's.
  `text_byte_capped_window_settles_a_missed_drag_at_its_nearer_edge`: a Text window
  barely taller than the viewport, a drag just past either side of its cameras, and a
  correction the byte cap answers with the same window.
  `scrollbar_source_byte_capped_rereads_complete_a_drag_in_the_candidate`: Source
  rereads answered with the same window, for whole-row windows on either side, for a
  window that starts inside a row (the position inside its cameras and past them) and
  for one shorter than the viewport (either side of its single camera).
  `text_edge_uncovered_corrected_reply_settles_at_the_nearest_edge_without_a_third_read`
  replaces the test that required this case to fail. With the settling limited to
  Markdown again and the Source fallback off, these failed with "The requested viewport
  was not completed.", and the short-candidate case failed the same way against the
  fallback before it handled such candidates; with the code restored (`cmp`), all
  passed. `worker::tests::seam::a_small_window_beside_a_table_settles_a_position_farther_away_than_its_span`
  (the 20 % and 71 % case) returned `Unavailable` with the span limit applied to
  Markdown windows too, and `Ready` without it. The tests that a far
  miss or a clipped file edge still fails pass unchanged:
  `scrollbar_uncovered_correction_fails_without_false_ack_or_another_read`,
  `scrollbar_source_recovery_byte_cropped_reread_cannot_commit_an_uncovered_candidate`,
  `scrollbar_source_recovery_near_home_and_end_byte_crops_cannot_acknowledge_file_edges`,
  `source_enormous_partial_line_exhausts_bounded_prefix_reads_without_acknowledging` and
  `source_terminal_byte_exception_rejects_clipped_edges_and_interior_near_end`.
  All-target Clippy with `-D warnings` and `rustfmt --check` on the changed files passed.
  These builds used a private target directory under `_build`, removed afterwards.
- Wheel and key travel across byte-capped pages: `cargo test -p midnight-viewer-engine
  --lib` 536 passed, 0 failed, 7 ignored (in the build container), including the five
  new `worker::tests::chain` tests, whose replies follow the basic viewer's byte-capped
  selection:
  - `a_byte_capped_forward_reply_is_joined_to_its_line_budget`
  - `a_byte_capped_backward_reply_is_joined_and_starts_at_a_row`
  - `a_bounded_backward_join_drops_its_cut_first_row`
  - `a_superseded_chain_shows_nothing`
  - `adjacent_text_pages_join_and_a_cut_first_row_is_dropped`

  With the joining switched off (requests not tracked, so replies pass through as
  before), the four that drive the worker failed: the short reply was published on its
  own, or no further range was read. The pure join and trim test passes either way.
  With the code restored (`cmp`), all five passed.
  `cargo test -p midnight-viewer-ui --lib` 203 passed. All-target Clippy for the engine
  with `-D warnings` and `rustfmt --check` on the changed files passed. These builds used
  a private target directory under `_build`, removed afterwards.
