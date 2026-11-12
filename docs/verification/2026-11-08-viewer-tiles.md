# Viewer tiles and scrollbar input — 8 November 2026

These are working-tree debug/test checks. Native measurements and deployment are
separate acceptance steps; this report does not claim that every viewer mode
already scrolls smoothly.

Raster images use a lazy 256-pixel source-raster grid. The backend retains at most
32 MiB/128 tiles and charges at most 64 MiB across cache, in-flight replies and
retained frame owners. Source/page/scale keys and nonreused IDs prevent stale
replies from entering a replacement surface. Reply headers are checked against
the requested raster before a larger payload can be read or allocated.

The UI caches at most 32 MiB/128 textures, allowing another 32 MiB for the preceding
Slint model during replacement. Framework/driver-deferred GPU destruction is not
an immediate physical memory guarantee. Ordinary pan changes two GPU offsets,
retains the same row model and uploads no existing tile again. New/evicted IDs
change rows; rare long-travel coordinate rebases are bounded. Missing coverage
retains undelivered motion. Native pixel gestures get no second coast; discrete
wheel input uses the shared 200 ms motion with immediate reversal. Current-source
failure stops retained-image motion; an old failure cannot disable a newer file.

Text input retains one pending absolute target per axis/context before bounded
enqueue. The backend finishes one active read and retains one latest deferred
vertical target. A bounded page is staged until its matching successful completion
and layout. Completed intermediate pages remain visible while the latest desired
thumb is retained; an initial/same-ack page cannot falsely settle that target.
Cancellation/error establishes a new context and retains completed content. File
transitions cannot repaint old content under a new generation. A relative wheel
or other accepted navigation cancels obsolete unsent absolute intent.

The shared scrollbar retains grab offset and extent during drag. Permanent gutter
space prevents a newly long text line from resizing the viewport and cancelling
capture. The actual-widget regression first failed on a 548→532 pixel height
change, then passed with conditional bar visibility inside stable gutters.

Windows engine tests passed **58**, with two installed-plugin tests intentionally
ignored by the ordinary unit gate. Image-host tests passed **5**. These cover
cache/new-frontier reuse, eviction, retained/in-flight leases, source/page/scale
identities, 10,000-position bounded ingress/defer cases, exact page completion,
no-match search and newer relative-horizontal input. Logs:
`_build/goal/viewer-performance/containment-zoom-backend-windows.log` and
`scrollbar-backend-windows.log` (image-host checks).

The Windows UI library passed **26**, plus **12** cases in `keyboard`,
`preferences`, `wrap_control` and `position_bar`. Real manager callbacks cover
movement before a reply, unchanged uploads/model identity, stale replies,
intermediate completed content during drag, mouse release, refused transitions,
same-mode selection, current/old source failure and immediate-close preferences.
Internal input admission is injected for deterministic accepted/refused tests;
the production sender delegates to the same bounded `WindowSession` queue.
Logs: `_build/goal/viewer-performance/containment-zoom-ui-windows.log` and
`_build/goal/viewer-scrollbar-drag/final-ui.log` (includes the final cache boundary
case). At a wide 4 Mi-pixel viewport, a diagonal transition initially exceeded the
128-entry UI cap. Its causal RED→GREEN regression now verifies that both old and
incoming visible rectangles stay complete while optional guards can be evicted;
the byte/entry caps did not increase.

`ViewZoom` is one atomic message capturing the source point from the displayed
camera, independent of backend frontier prefetch. Its causal anchor test failed
with the old camera calculation and passed with explicit source anchoring. It
covers zoom bursts, still-current older pan frames, replaced files and resize
coordinates. Manager tests verify one enqueue and no preference save on refusal.
There is no independent plugin API or dependency change.

PDF/document providers retain the earlier bounded **single-viewport** path,
selected from actual document metadata. Hayro traverses a page per request, so
image tiling would amplify interpretation. The engine test requires one 1000×700
request and no tile/guard requests. Untiled presentation preserves raster metrics
and routes both scrollbar axes to the existing backend inputs. Optimized PDF
panning remains separate work.

Linux passed the same **58 engine +5 image-host** tests, **10** shared motion tests,
and **38** viewer UI cases (26 library +12 integrations), including the cache
boundary. All-target strict Clippy passed for viewer engine, viewer UI, image host
and UI-input on Windows and Linux. Linux uses software-window fixtures; native
GPU pointer measurements here are Windows-only. Logs are
`_build/goal/viewer-performance/containment-zoom-{backend,ui,clippy}-linux.log`,
`wheel-motion-linux.log` and `containment-zoom-clippy-windows-green.log`.

Source decoding still uses bounded 32 KiB context reads, not a whole-file preload.
Source line-number/Markdown controls, text tiles and random-position line indexing
remain separate work. Actual debug pointer latency, tail outliers, continuous
content updates, zero image rereads and the exact tested worker hash are in
[the native scrollbar report](2026-11-08-viewer-scrollbar.md). Those measurements do
not establish uniformly frame-perfect latency.
