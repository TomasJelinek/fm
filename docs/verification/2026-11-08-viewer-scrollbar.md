# Viewer scrollbar verification — 8 November 2026

This change separates immediate scrollbar feedback from asynchronous content work.
It also fixes starvation during continuous dragging: the engine completes one
active viewport and retains only the latest deferred target. Completed intermediate
content can appear without moving the thumb away from the pointer. See the
[internal contract](../developer/VIEWER_SCROLLING.md).

## Causal evidence

- The real software-rendered PositionBar initially left its pixels at the old
  location while the renderer was delayed. Subsequent cases exposed unnecessary
  stationary-click reloads and an old gesture surviving a context change. The
  corrected component passed four tests covering both axes, grab offsets,
  reversals, delayed metrics, endpoints and cancellation.
- A bounded ingress regression initially dropped the second scoped position when
  its one-slot channel was full. Six tests now cover 10,000 coalesced updates,
  final sequence/value retention, sealing on consumption, context/axis/input
  barriers and honest refusal reporting.
- A native measurement exposed a second defect after the thumb was responsive:
  both text and source displayed **zero content updates during 60 pointer moves**.
  Images displayed 49 updates. This result was not accepted as a scrolling fix.
  The retained report is `_build/goal/viewer-scrollbar-drag/native-stable.txt`.
- Two new UI-state tests reproduced rejection of useful intermediate completions.
  All six `TextPosition` tests then passed with monotonic intermediate content,
  independent axis acknowledgements, retained final retries and stale/context
  rejection. The native probe now fails unless content changes at least twice
  during the gesture and the final target completes in the original context.
- A real viewer-widget test reproduced a 16-pixel viewport/vertical-track shrink
  when a newly long text line required the horizontal scrollbar. Stable reserved
  gutters prevent this internal geometry change from cancelling a held drag;
  unnecessary scrollbar thumbs remain hidden.

The actual viewer-manager tests include pointer press/move/release with delayed
frames, same-mode selection and refused file/mode transitions. Input-admission
tests use a private UI adapter to distinguish accepted and refused commands
deterministically; they do not depend on discovery of plugins beside a test EXE.
The engine's channel tests independently exercise the real bounded queue.

## Native measurement method

The debug `performance_probe --scrollbar` example runs in the existing protected
QA runtime. It opens only authored fixtures: 100,000-line plain text (8,600,000
bytes), source (8,888,890 bytes), and a 4096×3072 BMP (37,748,790 bytes). It does not
read or save user preferences. The native window is 2250×1575 physical pixels at
2.25 scale. Initial DPI/layout negotiation must settle before the gesture starts.

The probe injects 60 pointer events through the actual Slint window over three
sweeps, including two reversals, then releases at normalized position 0.7. Latency
is measured to `AfterRendering` only when the actual thumb matches the latest
pointer target. This is application input-to-render latency, not a measurement
of physical mouse hardware or display scan-out. Content updates during dragging
and final acknowledged content readiness are measured separately. A changed
context, lost final position or frozen content fails the run.

Before the final run, the independently built Basic, Source and Image packages
were aligned with the debug application's staging bundle. The Image worker has
SHA-256 `DA94231DF77D2E3B7AC73D9B7B3A3E6209B5EC906122FD4B8FC6D1AF3628EA29`.
An earlier protected copy had a different PE hash; it is not silently treated as
the same artifact for deployment qualification.

## Native progress results

The corrected, uninstrumented debug run passed all three stages:

| Fixture | Rendered thumb samples / 60 inputs | Thumb median / p95 / max (ms) | Content updates during drag | Final acknowledged content (ms) |
| --- | ---: | ---: | ---: | ---: |
| Plain text | 49 | 19.44 / 98.00 / 101.68 | 24 | 69.58 |
| Source | 49 | 17.46 / 90.98 / 111.97 | 21 | 91.94 |
| Image | 59 | 30.94 / 41.21 / 44.54 | 49 | 30.94 |

Inputs superseded before the next rendered frame have no individual latency
sample. The counts are reported rather than implying every pointer event produces
a distinct frame. Text's debug tail latency remains measurable; these results
establish continuous content progress and correct final delivery, not a claim of
uniform 60 Hz rendering. First opening is measured separately and is not included
in gesture latency. Log: `_build/goal/viewer-scrollbar-drag/native-progress.txt`.

A separate instrumented run passed the same gesture checks. Its source-request
totals were:

| Fixture | Read calls | Requested bytes |
| --- | ---: | ---: |
| Plain text | 63 | 503,320 |
| Source | 62 | 2,000,088 |
| Image | 146 | 38,273,024 |

The Image totals were already reached at the first image frame and remained
unchanged after dragging: cached pixels were reused without source rereads. Byte
counters measure requested capacity, including short reads near EOF; they are not
actual disk-I/O byte counts. Plain/source reads stay near requested viewports rather
than loading the complete files. Instrumented latency is not mixed into the table
above. Log: `native-progress-counts.txt` in the same directory.

The uninstrumented run with two synthetic CPU jobs also passed. Content updated
22/21/53 times for plain/source/image, with final acknowledged content at
70.82/93.25/30.55 ms. Thumb median was 17.60/18.26/30.68 ms and p95 was
82.34/86.04/40.97 ms. Plain text had one 496.85 ms outlier; source/image maxima were
116.23/43.54 ms. This remaining debug-frame stall is recorded rather than hidden
by the median. It does not reintroduce content starvation or a lost final target,
but further latency profiling is still appropriate. Log: `native-progress-load.txt`.

## Final build and deployment

The final Windows debug application build passed after the PDF, atomic zoom and
wide-viewport cache follow-ups. Viewer engine tests passed 58, with two
installed-plugin tests intentionally excluded from the ordinary unit gate; viewer
UI passed 26 library and 12 integration tests on both Windows and Linux. Image
host passed 5 tests on each platform. Owned all-target strict Clippy passed on
both platforms. Workspace formatting and the 100,000-byte handwritten source
limit passed. See [the compatibility report](2026-11-08-viewer-tiles.md) for the
exact scope and remaining PDF/line-indexing limitations.

The runnable debug executable is `_build/debug/me.exe`, SHA-256
`01F000C3E40C87DB255B1C35FBF9657C3E6B5ECED545FCAB241DB4C8AA8712BB`.
Its protected runtime was deployed and registered at
`C:\Users\tjelinek\MidnightExplorer-20261108-013509-a5fe735d`, including independently
built viewer, filesystem and thumbnail packages. Deployment verifies every copied
file by SHA-256 readback and preserves plugin path validation. This is a debug
deployment; the release executable was not replaced.

Starting `_build/debug/me.exe --ui-smoke-test` passed with both the native renderer
and the software renderer, using only the authored fixture directory and in-memory
settings. The software snapshot was visually inspected: both panels, populated
fixture lists, navigation and the bottom command/function bars render without a
startup notice. Real user preferences had the same SHA-256 before and after both
runs. These application startup checks complement, rather than replace, the native
viewer drag measurements above. Evidence is in `app-build-final.log`,
`final-clippy.log`, `deploy.log`, `app-acceptance.txt`, `app-{native,software}.*.log`
and `app-smoke.png` under `_build/goal/viewer-scrollbar-drag/`.
