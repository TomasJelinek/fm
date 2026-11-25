# Viewer paging, a kept-ready HTML browser and text viewer opening

Date: 25 November 2026, main `ad9557bd`. Status: **design decisions and plans. Nothing in
this document is implemented**; the linked documents describe the current behaviour. The
probe numbers below are single runs on one loaded Linux machine, not a qualification record.

Three Architect decisions for the review items 2c and 2a (viewer plugin page size, the
8000-byte-row limit), 7c (a kept-ready HTML browser, backlog VIEW-08) and 8 (a text viewer
open under 200 ms). Each decision closes nothing in the
[backlog](../../IMPLEMENTATION_BACKLOG.md): its rows close only with the evidence its test and
measurement plan names.

**Probe environment** (all three decisions): Ubuntu 26.04.1, kernel 7.0, GNOME on Wayland,
32 cores, 125 GiB. The GPU driver had been updated without a reboot ("Driver/library
version mismatch"), so no process could use the GPU: every number here is CPU work or the
software renderer. Other agents' builds and an acceptance run used the machine at the same
time. Builds of main `ad9557bd` in the `dev` profile, except where stated. The GPU
measurements are planned in [After the reboot](#after-the-reboot-gpu-measurements).

## 1. Viewer plugin page size (items 2c and 2a)

### Problem and evidence

The basic and Source viewers answer a Text or Source viewport with at most about 8 KiB
(`plugins/midnight-viewer-basic/src/decode.rs:25-35`, `READ_BYTES` 8192 and `MAX_UNITS`
8191; `plugins/midnight-viewer-source/src/paging.rs:97,172,205,240,250` and
`preparation.rs:244,256,456,490,508`, `min(8192)`), while the engine asks for 12 KiB
(`crates/midnight-viewer-engine/src/worker.rs:1024`). The cause is the published model: one
`Anchor` per character, a CBOR map of 5 wire items and about 11 bytes, within 65,536 wire
items, 16,384 anchors and 1 MiB per message (`api/midnight-plugin-api/src/viewer.rs:5-7`,
`wire.rs:4-6`). The
[viewer presentation](../../developer/VIEWER_PRESENTATION.md#refill-reads-and-the-retained-frontier)
documents this as an API limit of about 13,000 characters per reply.

The engine joins byte-capped replies into one page of up to 120 KiB in at most 16 reads
(`JOIN_READS`, `crates/midnight-viewer-engine/src/worker_chain.rs:32`), one after another:

- A refill of 120 KiB takes 15 or 16 sequential replies. In the co-simulation a reply is
  answered only between worker turns, and a fast wheel spin over unwrapped 2400-byte rows
  still stalls under load (SCROLL-02;
  [long rows record](../../verification/2026-11-22-long-rows-row-groups.md#measurements)).
- Rows of 8000 bytes (item 2a): a window of 44 such rows needs 352 KiB. The page grows at
  rest by one request per refill (`worker_long_row_tests.rs`,
  `rows_of_8000_bytes_fill_the_window_at_rest`), so the window fills only after about 45
  sequential replies: about 0.75 s at the co-simulation's one frame per reply (estimate).
- An explicit whole-file Copy reads 256 KiB requests that the plugins answer 8 KiB at a time.

Probe: private copies of the two plugins (not committed) with the page cap set at run time,
built in release against a copy of the API whose aggregate wire limits were raised so that
per-character pages of 32 and 64 KiB could be encoded at all. Each plugin's own ABI test
harness walked 1.5 MiB of a 2 MiB generated file forward with `max_lines` 4096. "Plugin" is
request to terminal event in the harness (decode, page selection, encoding); "decode" is the
host's `codec::decode` with validation.

| Plugin, file | Cap | Text | CBOR payload | Anchors (spans) | Plugin | Decode | Per 120 KiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| basic, 2400-byte ASCII rows | 8 KiB | 8.0 KiB | 94 KiB | 8,181 | 1.11 ms | 0.48 ms | 16 replies, 25.5 ms |
| basic, same | 32 KiB | 32 KiB | 381 KiB | 32,757 | 4.40 ms | 1.93 ms | 4 replies, 25.3 ms |
| basic, same | 64 KiB | 64 KiB | 762 KiB | 65,525 | 9.61 ms | 3.99 ms | 2 replies, 27.2 ms |
| basic, Czech and Japanese prose | 8 KiB | 8.0 KiB | 63 KiB | 5,206 | 0.74 ms | 0.32 ms | 16 replies, 17.0 ms |
| basic, same | 64 KiB | 64 KiB | 509 KiB | 41,698 | 5.87 ms | 2.55 ms | 2 replies, 16.9 ms |
| Source, generated Rust | 8 KiB | 8.0 KiB | 101 KiB | 8,193 (555) | 1.30 ms | 0.53 ms | 15 replies, 27.3 ms |
| Source, same | 32 KiB | 32 KiB | 409 KiB | 32,769 (2,173) | 5.14 ms | 2.12 ms | 4 replies, 29.0 ms |
| Source, same | 64 KiB | 60 KiB | 771 KiB | 61,798 (4,096) | 10.2 ms | 4.19 ms | 2 replies, 28.9 ms |

Medians; "per 120 KiB" is replies times (plugin + decode). 64 KiB Source pages also needed the
plugin's 64 KiB read and context budgets raised (it refused with "Read range exceeds plugin
budget"), and dense Rust stopped at the 4,096-span limit at 60 KiB. Pages over 8 KiB exceed
today's wire budget: 163,806 items at 32 KiB, 327,646 at 64 KiB.

Estimate for run anchors (the decision below), from the same pages: anchors folded into runs
where the text and source steps are equal, payload counted analytically, encoding and
decoding timed on the page without its interior anchors.

| File, cap | Payload, runs (per character) | Decode | Per 120 KiB |
| --- | ---: | ---: | ---: |
| 2400-byte ASCII rows, 64 KiB | 64 KiB (762 KiB) | 0.008 ms (3.99 ms) | 8.3 ms (27.2 ms) |
| Prose, 64 KiB | 64 KiB (509 KiB) | 0.12 ms (2.55 ms) | 5.4 ms (16.9 ms) |
| Generated Rust, 60 KiB | 112 KiB (771 KiB) | 0.39 ms (4.19 ms) | 10.9 ms (28.9 ms) |

So: per-character anchors make the cost proportional to characters, 17–29 ms of plugin and
decode CPU per 120 KiB at every cap, and the payload 8–13 times the text. Larger pages alone
save round trips, not CPU, and need five times today's wire budget. Runs save both.

### Options

- **A. Fill each reply to the exact wire budget** within the current API. At most about
  12 KiB of text without spans, 16 → 10 replies per 120 KiB, the per-character cost stays.
  Plugin-only; acceptable as an interim step if B slips, not as the decision.
- **B. Raise `MAX_ITEMS` and `MAX_ANCHORS`** about five and four times in a new API: 64 KiB
  replies of 762 KiB, unchanged CPU, and a five times weaker host bound against faulty
  plugins. Released hosts would refuse such pages, so it needs negotiation anyway. Rejected.
- **C. Run anchors, negotiated per session, with 64 KiB replies** within today's wire
  limits. Selected.
- **D. Anchors packed into one byte string.** One wire item, but every boundary is still
  decoded, validated and stored; a binary format inside CBOR. Rejected for C.

### Decision

Plugin API/SDK **0.3.0** adds an optional run form of the text-to-source mapping, which a
plugin may use only on a session whose `Open` advertised it:

- `Open` gains `5: accepts ?u32`; bit 0 (`ACCEPT_ANCHOR_RUNS`) says that the host validates
  and accepts runs. Other bits are reserved and ignored.
- `TextPage` gains `8: runs ?Vec<AnchorRun>`. `AnchorRun` is `0: text_offset u32`,
  `1: source_offset u64`, `2: text_bytes u32`, `3: unit u8`: unit 1 maps each scalar to its
  own UTF-8 bytes, unit 2 to two source bytes per UTF-16 code unit (both byte orders; a
  surrogate pair is four bytes). Every scalar boundary inside a run is exact.
- A run's start and end are listed anchors, so first/last-anchor and ordering rules are
  unchanged. Runs are nonempty, ordered and disjoint, end on character boundaries, and the
  source end computed by the unit equals the end anchor's source offset. Escapes and
  normalized line breaks (CR, CRLF) stay point anchors. Unknown units are invalid.
- A run costs 9 wire items, so a page is bounded by its text (256 KiB) and spans (4,096),
  not anchors. The wire limits do not change.
- The basic and Source viewers answer up to 64 KiB when runs are accepted, and the engine
  asks Text and Source providers for 64 KiB instead of 12 KiB (each chained read stays
  within the layout's room). 64 KiB, not 120 KiB, keeps a reply about 5 ms of plugin work,
  which bounds cancellation, and fits the Source budgets and the 4,096-span limit.

The host validates runs and expands them into the existing per-character anchors right after
decoding (`crates/midnight-plugin-host/src/dispatch.rs:522`), and charges the expanded
anchors to the event queue (`events.rs:71` there). The engine is unchanged by this
decision. Its 16 bytes per boundary (about 1.9 MiB per 120 KiB page, up to 16 MiB for a
1 MiB row page) are a separate step: a compact mapping inside the engine, triggered by a
row-page memory measurement or report.

### API and compatibility

- The 0.1.0 and 0.2.0 packages stay byte for byte; the C ABI stays 1.0; `plugin.toml` and
  the `text-page.v1` model name do not change. Released hosts reject unknown manifest
  models (`crates/midnight-plugin-host/src/discovery.rs:232`), so the negotiation is
  in-band, as Source preparation's is.
- New host, plugin on SDK 0.1 or 0.2: the plugin skips the unknown `Open` field after
  wire validation and sends per-character pages at its own cap, as today.
- Released host, plugin on SDK 0.3: no advert, so the plugin sends per-character pages of at
  most 8 KiB. An old host would skip field 8 and see only the listed anchors, so a plugin
  must never send runs without the advert.
- A page with runs on a session without the advert is a protocol violation, refused like
  any invalid page.
- The basic viewer moves from SDK `=0.1.0` and Source from `=0.2.0` to `=0.3.0`; Markdown
  may follow for block text. The [viewer protocol](../../developer/VIEWER_PROTOCOL.md#limits)
  and the presentation's 8 KiB statement are rewritten when this lands.

### Tests and measurements

- API: run validation, accepted (ASCII, multibyte UTF-8, UTF-16LE/BE with surrogate pairs)
  and refused (empty, overlapping, unordered, inside a character, endpoint not an anchor,
  wrong computed end, unknown unit); a 64 KiB ASCII page with runs fits the wire budget; an
  all-escape page stays bounded.
- Compatibility: a 0.3 `Open` decodes with the released 0.1.0 and 0.2.0 decoders from the
  offline bundle; 0.2 pages decode with 0.3; a run page without the advert is refused.
- Host: expanded runs equal the per-character anchors of the same page (ASCII, UTF-8, CRLF,
  invalid bytes, controls, bidi controls, UTF-16 with and without BOM, lone surrogates);
  queue charges cover the expansion.
- Plugins: with and without the advert, at 8, 32 and 64 KiB, forward and backward, at both
  file ends, the expanded page equals the per-character page (text, spans, anchors); the
  10 GB sparse-file tests stay bounded.
- Engine: `worker_chain_tests` with 64 KiB replies (at most 2 reads per 120 KiB);
  `worker_long_row_tests` (8000-byte rows fill the window at rest within 6 replies);
  `worker_row_group_tests`; `measure_prepared_text_scrolling` with 64 KiB replies, the
  40-notch spin over unwrapped 2400-byte rows at 1x and 2x against 8 KiB replies on the same
  build. Target: no stalled frame.
- Measurements: the page probe again for the run form (plugin + decode at most 12 ms per
  120 KiB); the native `performance_probe` wheel spin over 2400-byte rows, Text and Source,
  on Windows and on Linux after the reboot (stalls, worker-turn p95); whole-file Copy of a
  64 MiB file, before and after.

### Implementation outline

| Owner | Files |
| --- | --- |
| Architect, Backend | `api/midnight-plugin-api/src/messages.rs` (`Open.accepts`), `src/viewer.rs` (`AnchorRun`, `TextPage.runs`, validation), the SDK's `Open` accessor and `Output::text`, `api/Cargo.toml` 0.3.0, the offline bundle (`api/scripts/package.sh`) |
| Backend | `crates/midnight-plugin-host/src/session.rs` (advertise), `dispatch.rs` (validate, expand), `events.rs` (charges) |
| Viewer | `plugins/midnight-viewer-basic/src/decode.rs`, `viewer.rs`, `Cargo.toml`; `plugins/midnight-viewer-source/src/paging.rs`, `preparation.rs`, `page_budget.rs`, `plugin.rs`, `decode.rs`, `syntax.rs`, `Cargo.toml`; `crates/midnight-viewer-engine/src/worker.rs` (64 KiB requests), co-simulation reply size |
| Perf, Documentation | Probe and co-simulation records; `docs/developer/VIEWER_PROTOCOL.md`, `VIEWER_PRESENTATION.md`, backlog SCROLL-02 |

## 2. A kept-ready HTML browser (item 7c, VIEW-08)

### Problem and evidence

Every HTML document and Office preview starts its own Chromium broker with a private profile
and its own cgroup or job object (`crates/midnight-viewer-engine/src/worker_web.rs:467` →
`crates/midnight-web-viewer-host/src/session_actor.rs:502` →
`crates/midnight-worker-platform/src/browser.rs` `spawn_with_extensions`); Space and Backspace
to the next file start another. Nothing is started ahead.

Probe, this machine, CEF without the GPU, the `dev` build's provider, a generated
2,000-paragraph page (393 KB) at 1000 × 700, 1x, through the host's `html_idle` example
(`crates/midnight-web-viewer-host/examples/html_idle.rs`), each run a new host process:

- First frame of the document (the example's measure, not the acceptance's first content)
  705–1124 ms, median 778 ms (17 openings); close 19–37 ms, one of 22 closes 3.4 s. The open
  document's tree: 10 processes, 196 MiB charged to its cgroup.
- Openings that then switched once to a fresh context and browser in the same process (the
  `scripts` mode's policy change) took a median 846 ms in all, against 715 ms without the
  switch in the same batch: a further document in a running browser cost about 130 ms. A
  kept-ready browser would therefore open this page in about 0.15 s instead of 0.75 s
  (estimate).
- The provider started without a document, in the host's environment: its tree settled
  after 0.9–1.2 s (8 processes, 0.67 s CPU). Parked, its main thread woke **505 times per
  second** (the 2 ms pump, VIEW-04) and the tree used 10–20 ms CPU per second; proportional
  set size 321 MiB, mostly shared library pages.
- The Windows 0.5.4 release run's first content after 20.7 s against a 10 s budget
  ([release record](../../verification/2026-11-25-release-0.5.4.md#whole-suite-acceptance-at-510d9abc))
  is one unexplained outlier; the same pages met the budget at `c7ef1e5c`. A kept-ready
  browser removes the start from the opening, but it does not explain 20 s; that run needs
  its own trace (see the plan).

### Options

- **A. Status quo**: a fresh browser per document.
- **B. One-use warm spare**: one pre-started provider process that has never held a
  document, adopted by the next HTML session and never returned. The image viewer's
  source-free, one-use decoder spare (`crates/midnight-viewer-engine/src/image_startup.rs`,
  [spare shutdown](../../developer/ENGINE_LIFETIME.md#image-viewer-spare-shutdown)) is the
  precedent. Selected.
- **C. One long-lived browser for all documents**, a request context per document: fastest
  and smallest with several windows, but the documents share the browser, network and GPU
  processes and their caches, one crash ends every HTML viewer, and it contradicts the one
  private profile per session of [SECURITY.md](../../../SECURITY.md). Rejected; it would
  need a web viewer API design of its own.
- **D. Return browsers to a pool after their document**: CEF offers no complete wipe of
  renderer, cache, socket, DNS, HSTS and worker state. Rejected.

### Decision

- **One spare per application**, shared by viewer windows, in the engine service next to
  the image spare. It is never started at application start: it is started after an HTML
  session of this run showed its first frame (so it never competes with that opening) and
  after each adoption. A ready spare not adopted within **120 s** is stopped and not
  replaced until the next HTML opening. A later, measured option: start one when the panel
  cursor has rested 0.5 s on a file associated with the HTML viewer.
- **Isolation is unchanged**: a process serves at most one document in its life. The spare
  has its own private profile, temporary directory and cgroup or job object from its start,
  and receives no source, resource grant or policy before adoption; `Open` carries those as
  today. Office previews, which launch without the link opt-in, start cold.
- **Revalidation at adoption**, on the session actor thread, never the UI thread: the
  provider's package identity (`Provider::validate`, as every opening checks today), the
  catalog revision (disabled viewers, associations), the launch extensions and liveness. Any
  mismatch stops the spare in the background and the session starts cold.
- **Parked state**: before a spare may wait 120 s, the provider must stop the 2 ms pump while
  it has no document: at most 10 wakeups and 2 ms CPU per second for the tree on Linux and
  Windows, by waiting for its first command with the delay CEF asks for, or by blocking once
  CEF reports its context initialized. Until a provider meets that budget, its spare lives
  30 s. The host keeps spares only for the bundled provider version that parks, as the 0.4
  host sends offline scripts only to its bundled provider.
- **Failures**: a spare that cannot start (sandbox refused, libraries missing, no cgroup
  delegation) is recorded and not retried until the next HTML opening, which reports the
  failure as today. The spare shows nothing itself.
- **Shutdown**: a retirement ticket like `ImageSpareRetirement`; disabling the HTML viewer, a
  changed association or a changed provider package also stop the spare.

Memory: about 200 MiB more while a spare is ready (196 MiB charged with a document), within
the per-browser limits of 2 GiB and 256 tasks (Linux) or 32 processes (Windows).

### API and compatibility

No web viewer API or wire change: the provider initializes CEF when it starts, before it
reads its first command (`plugins/midnight-viewer-html/src/native.rs:410,473`), and `Ready`
follows the document's browser (`native.rs:262`). The parked state is internal to the
provider. Host-internal: `midnight-web-viewer-host` gains a prepared browser (spawned,
validated identity, launch extensions) that `Session::open_with_options` can adopt;
`midnight-worker-platform`'s spawn is reused unchanged. Third-party web providers keep the
cold start. SECURITY.md gains: a browser may start up to 120 s before its document and holds
no document, source or grant until then.

### Tests and measurements

- Unit (spare manager, with a fake backend like `image_startup_tests.rs`): one spare at most;
  never at start; refill only after a first frame; expiry; no refill after a failure;
  revalidation refusal falls back to a cold start; shutdown stops a starting and a ready
  spare; the four-window limit is unaffected.
- Host session tests: an adopted spare and a cold browser give the same events and frames for
  the same document; a spare that died is replaced by a cold start without an error.
- Provider: a parked-provider probe for wakeups and CPU (the `html_idle` example gains a
  parked mode) on Linux and Windows, against the budget above.
- Acceptance: `viewers_files` records first and repeat HTML openings separately; a repeat
  opening over 500 ms is flagged as today.
- Measurements, Linux and Windows, with and without the spare: first frame of the first and
  of later openings, close, memory charged while ready, CPU while parked, on the generated
  page and on the large page the owner reported; with the GPU after the reboot. The
  provider's `diagnostic-timing` build splits CEF start from the document.

### Implementation outline

| Owner | Files |
| --- | --- |
| Backend | `crates/midnight-web-viewer-host/src/session.rs`, `session_actor.rs` (adopt a prepared browser), new `spare.rs`; `crates/midnight-viewer-engine/src/lib.rs`, `worker_web.rs`, new `web_spare.rs` with tests (lifetime as `image_startup.rs`) |
| Viewer | `plugins/midnight-viewer-html/src/native.rs` (parked loop), its README |
| Perf | `crates/midnight-web-viewer-host/examples/html_idle.rs` (parked mode), measurements |
| Documentation | `SECURITY.md` (HTML viewer section), `docs/developer/WEB_VIEWER_API.md`, `ENGINE_LIFETIME.md`, backlog VIEW-08 and VIEW-04 |

## 3. A text viewer open under 200 ms (item 8)

### Problem and evidence

On the discrete GPU with X11 a one-line text file took 268–322 ms to first content (release probe and
plugins, one shared wgpu device): window constructed 4 ms, shown 5 ms, renderer ready (native
window and swapchain) 250–330 ms, then 28 ms to content
([viewer speed record](../../verification/2026-11-20-viewer-speed.md)). Creating the Vulkan
swapchain made about twenty GLX requests to the X server, each answered after about one
compositor frame. `4e3e64c6` starts capture and decoders before the window
(`crates/midnight-viewer-ui/src/lib.rs:472-513`); it was measured only in Windows debug
builds (text 1,555 → 1,108 ms median at 225%,
[Windows record](../../verification/2026-11-25-windows-viewer-opening.md)).

Probe today, software renderer (`open_latency`, `MIDNIGHT_PROBE_RENDERER=software`, `dev`
probe and plugins), three openings each:

| Backend | First content | First output reached the window | Content frame |
| --- | --- | --- | --- |
| Native Wayland | 99 / 95 / 88 ms | 60 ms after the call (shown at 8 ms) | 99 ms |
| XWayland (`env -u WAYLAND_DISPLAY`) | 42 / 30 / 27 ms | 21 ms (shown at 8 ms) | 41 ms |

The viewer's own work fits well within 200 ms; the GPU window and swapchain are what remain.
On Wayland the compositor's configure and frame callbacks add about 60 ms even without a GPU.

Slint 1.18.1 decides what a hidden window keeps
(`i-slint-backend-winit` `winitwindowadapter.rs`): on X11 and Windows `hide()` unmaps the
native window, which keeps its surface and swapchain; on Wayland it destroys the native
window ("Wayland doesn't support hiding a window, only destroying it entirely"), and winit
cannot create a hidden Wayland window. Winit picks Wayland whenever `WAYLAND_DISPLAY` is set
(`crates/midnight-explorer/src/window_backend_linux.rs`). The Wayland rendering notifier
(`47cacc19`, `crates/midnight-ui-input/src/native_presentation_linux.rs`) is installed per
native window and removed when that window is destroyed.

### Options

- **A. Keep one hidden `ViewerWindow` for reuse** after a viewer closes. Selected for X11
  and Windows.
- **B. Create a hidden window at application start**: Slint creates the native window only
  when it is shown, so this needs a visible flash, and Wayland cannot do it. Rejected; the
  first opening of a run keeps its cost.
- **C. Make creation itself cheaper** (present mode, adapter, fewer X requests): driver and
  winit territory; measure only.
- **D. Keep closed viewers minimized or off screen**: visible in task switchers, and Wayland
  cannot place windows. Rejected.

### Decision

- **A pool of one**: closing a viewer keeps its `ViewerWindow` hidden when the viewer closed
  normally (not after a failed opening), uses the GPU renderer, has no native child (HTML,
  video), and runs where hiding keeps the surface: X11 and Windows, not native Wayland, and
  not with `SLINT_DESTROY_WINDOW_ON_HIDE`. The `OpenWindow`, session and timers
  are released as today (`f2f6b423`); only the Slint component and its native window stay.
- **Nothing of the previous document is shown or kept**: at close the content, textures,
  search, status and title are cleared and one blank frame is rendered before the window
  hides. Without that frame within 100 ms the window is dropped, not kept. An opening resets
  every property the session may have changed with one Slint function in the component's own
  file, rebinds the callbacks, applies placement, size, monitor scale and appearance, and
  then shows.
- **Lifetime**: the pooled window is dropped after 60 s unused, on an appearance change and
  at shutdown. It does not count against the four-window limit. Memory: its swapchain (about
  8 MB at 1000 × 700, 1x; about 43 MB at 225%) and renderer caches.
- **Wayland**: no pool until a GPU measurement shows what it would save. Reusing only the
  component and renderer (fonts, FemtoVG pipelines) is the possible gain there; it is enabled
  only if it saves at least 30 ms and the notifier keeps presenting (no return of
  `47cacc19`'s one-second freezes). XWayland is not forced: it changes input, IME and
  scaling.
- **Scope**: the 200 ms target applies to repeat openings, where the pool works; a run's
  first opening stays at the measured cost. Requiring it for the first opening would need B,
  which is rejected above.

### API and compatibility

No plugin, engine or settings change. `ViewerWindows` keeps its public API; the pool and the
platform rule are private to `midnight-viewer-ui`. Each platform's rule ("hiding keeps the
surface") lives in its own file. `MIDNIGHT_VIEWER_POOL=0` turns the pool off for
comparisons, like `MIDNIGHT_SHARED_WGPU=0`.

### Tests and measurements

- UI tests, software renderer: open A, change its state (search with text, wrap, mode,
  scroll, zoom, status), close, open B in the pooled window: the first frame and the read-back
  properties equal those of a fresh window opening B, at 1x and 2x. A failed opening, an HTML
  or video window and a Wayland window are not pooled; expiry and an appearance change drop
  the window (its `Weak` no longer upgrades).
- Native: `open_latency` with and without the pool, first and later openings, on X11 and
  Windows; on Wayland the component-only variant. Acceptance's latency notes record repeat
  text openings over 200 ms (flagged, not failed, on shared machines).
- Target: repeat text openings at most 200 ms on X11 with the discrete GPU and on Windows; Wayland
  recorded with its measured cost.

### Implementation outline

| Owner | Files |
| --- | --- |
| UI | `crates/midnight-viewer-ui/src/lib.rs` (take from and return to the pool), new `window_pool.rs` with `window_pool_tests.rs`, `window_pool_linux.rs` and `window_pool_windows.rs` (the platform rule), the viewer component's reset function in `ui/` |
| Perf | `crates/midnight-viewer-ui/examples/open_latency.rs` (first and repeat openings, pool switch), measurements |
| Documentation | `docs/developer/VIEWER_PRESENTATION.md` (window lifetime), backlog row, dated record |

## After the reboot: GPU measurements

Run these once the GPU is usable. First check that the loaded GPU driver matches the installed
one, and keep the monitor on: presentation blocks while it is off.

- **Decision 3, text viewer opening.** Build `open_latency`
  (`cargo build --locked -p midnight-viewer-ui --example open_latency`) and place it beside the
  build's `viewers` folder. Open a one-line text file three times per process, with
  `MIDNIGHT_PROBE_SHARED_WGPU=1 MIDNIGHT_VIEWER_OPEN_TRACE=1`:
  - on native Wayland;
  - on XWayland, with `WAYLAND_DISPLAY` unset;
  - with `MIDNIGHT_PROBE_RENDERER=software` as the control.

  Run two rounds, alternating the three, and record each opening's first content in ms
  (`first_content_ms`).
- **Decision 2, HTML with the GPU available.** Chromium may composite on the GPU now. Record:
  - the first frame, close time and memory of the 393 KB test page over six openings;
  - the second-document comparison with `html_idle`
    (`cargo build --locked -p midnight-web-viewer-host --example html_idle`);
  - the wakeups and CPU of a provider started without a document.

Windows, by the owner of the desktop lease: `open_latency` with
`MIDNIGHT_VIEWER_OPEN_TRACE=1` for the phase split of the 1.1 s opening, and the
`viewers_files` HTML page with the provider's `diagnostic-timing` build to explain the
20.7 s first content of the 0.5.4 run. Decision 1 needs no GPU until its native wheel
measurement.
