# Acceptance time — 2026-11-24

## Scope

The owner's goal: measure how long each test runs and make the tests faster without
losing coverage or functionality, and remove obsolete tests. This pass measured the
Windows acceptance suite at `c8c20f49` (the two slowest scenarios, `viewers_scroll` and
`ui_polish_dialogs`, took 20 minutes each) and the root workspace's Rust test
executables, and changed the acceptance harness and four scenarios. Follow-up to the
[test speed pass](2026-11-21-test-speed-pass.md) and [CI time](2026-11-21-ci-time.md)
of 21 November, which covered the Rust tests and CI but not the acceptance suite.

## Summary

The whole Windows acceptance suite took 3,619 s instead of 6,513 s (−44 %);
`viewers_scroll` 477 s instead of 1,209 s (−61 %) and `ui_polish_dialogs` 455 s instead of
1,202 s (−62 %), with the same subjects, targets, surfaces, buttons and checks. Almost
every scenario got faster, because most of the harness's time went into its own
unoptimized pixel loops: the acceptance test target is now built at `opt-level = 2` in
the debug profile. The rest came from waiting for conditions instead of fixed times in
the two slowest scenarios. No check was removed, sampled or loosened. Of the Rust tests,
two dominated their executables (58 s and 25 s); they now take 2 s and 0.4 s with the
same assertions, and the 69 test executables run 50 s faster (136 s before).

The faster harness exposed three scenarios that had relied on its slowness. Two were
test races and are fixed (`viewers_controls`, `viewers_thumbnails`); the third is a
product defect that the old pace hid: `viewers_presentation` fails because the PDF
viewer drops a Ctrl+PageDown pressed before the previous page has been shown
([finding](#finding-pdf-page-keys-dropped)).

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on `D:`
(97 % full, 8.3 names off), Rust 1.99.1. Debug test profile, debug information off,
`CARGO_INCREMENTAL=0`, a private target directory. Plugin packages: a private copy of the
release build of `ca70e1af` (`--plugins`). The desktop lease was held for every run; the
processor load before each run was 1–8 %; other agents may have compiled at times
during the runs (not sampled).

## Method

- **Timing notes.** A new harness module (`tests/acceptance/timing.rs`) sums wall-clock
  time per named phase on the scenario's thread; every scenario's report now ends with
  `timing` notes (`timing<TAB>phase<TAB>seconds<TAB>count`, staged children included).
  `viewers_scroll` times each subject's open, sweep, pixel deltas, keys and notches, the
  probes their drags, settles, still and settled frames and renders; the polish gate its
  focus, hover, tooltip and Escape checks, each action's phases, its state samples and
  `ui_polish_dialogs`' restores and surfaces; `App::settle` is counted everywhere.
- **Same-session before and after.** `before` is `c8c20f49` with the timing notes only
  (commit 1, no behaviour change); `opt only` adds the profile change (commit 2); `after`
  is the final tree. Each ran `viewers_scroll` and `ui_polish_dialogs` on one idle
  machine within three hours.
- **Whole suite.** The coordinator's run of `3618ff50` (23 November, this machine)
  is the before; the final tree's whole run
  (`20261124-035513`) the after. `c8c20f49` has five product commits more than
  `3618ff50`; the scenarios they touch (drag-out, the progress window, Secure this
  folder) are small.
- **Rust tests.** Every test executable of `cargo test --workspace --no-run` ran alone,
  one after the other, with libtest's JSON output and `--report-time`
  (`RUSTC_BOOTSTRAP=1` for the test process only), as in the 21 November pass.

## Where the time went

`viewers_scroll`, before (1,211 s): 709 scrollbar drags took 527 s (a drag 284 ms and its
landing 456 ms); the 316 settled frames of the movement measurements 496 s (1.57 s each:
still for 300 ms, then unchanged 700 ms later); 323 still frames 240 s. A render of the
viewer window into the harness's buffer took 49 ms and an idle `App::settle` 39 ms — in
both, mostly the harness's own unoptimized code (allocating and filling the frame
buffer, copying and hashing it), not the product's renderer, which is already built at
`opt-level = 2`.

`ui_polish_dialogs`, before (1,227 s): hover checks 307 s, the 309 restores of the
scratch folders 218 s, focus checks 187 s, tooltip checks 59 s; of the 169 button and
Enter actions, opening 87 s, the samples before the action 137 s, waiting for its effect
107 s. An `App::settle` took 56 ms and a state sample (every window rendered, its texts
and controls read, the folders hashed) 145 ms.

## Changes

1. **Timing notes** (above). Cheap: one `Instant` read at each end of a phase.
2. **Harness optimized** (`[profile.dev.package.midnight-acceptance] opt-level = 2` in the
   root `Cargo.toml`). Only the acceptance test target changes; the product crates it
   drives keep their debug profiles. A viewer render took 4.6 ms instead of 49 ms, an idle
   `App::settle` 14 ms instead of 39 ms, a polish-gate state sample 24 ms instead of
   145 ms. The acceptance crate compiles in about 2 minutes. CI only lints this target
   (`cargo clippy`), which does not generate code, so CI's time does not change.
3. **`viewers_scroll`** (same subjects, every 1 % and 2 % target, the same checks):
   - A scrollbar drag has landed once the viewer has acknowledged it (no position
     pending, not loading), shows the content where the thumb is (to 1e-4) and its
     positions held still for 100 ms (`viewers_probe::land`); otherwise the 300 ms quiet
     of `settle` applies. An acknowledgement means the target's pixels are there
     ([viewer scrolling](../developer/viewer-scrolling.md): a final acknowledgement needs
     usable pixels covering the target), so no refill pause can follow it.
     Covered as before: the landing position, error statuses, thumb against content and
     the page's bytes at every target; a late correction within 100 ms still restarts the
     quiet period.
   - A series of measurements (pixel deltas, keys, notches, HTML keys) starts each one
     from the frame the previous one settled on when the content still shows exactly that
     frame and nothing is pending, instead of settling a second time (≥ 1 s). A notch that
     moved on after its rest starts the next measurement afresh.
   - The arrows back after a page step pause between groups of four until the content is
     still for 300 ms instead of a settled frame (≥ 1 s). The first rest after the page
     step (an eased step must end before a reverse key) and the measurement's end keep
     the settled frame; twenty Down presses in a row are separately checked to move
     twenty lines.
4. **Polish gate, interactive checks** (`ui_polish_dialogs`, the red cases of
   `ui_polish_checks`, the archive password dialog of `ui_polish_viewers`): focus, hover
   and tooltip checks run with the harness's Slint clock held; each of their waits
   (`App::wait_ui`: the frame after a Tab, the hover transition, the tooltip delay) moves
   the clock by its length at once and settles. Animations and timers reach the same state
   without real time passing. Escape, Enter and the buttons, whose effects go through the
   Engine, keep real time. The red cases still flag every broken twin and pass every
   corrected one (`ui_polish_checks` PASS). `ui_polish_jobs` calls the checks with the
   clock running (unchanged). `ui_polish_dialogs`' restore no longer calls `dismiss_all`
   (which always waits 120 ms) when nothing is open; the 150 ms for a late window stays.
   The effects found per button were compared before and after: the same buttons, all
   with effects; only the `commands` field (background commands) varies between runs, as
   it did before.
5. **Races the slow harness hid** (both failed 3 of 3 with the optimized harness and
   passed with the old one):
   - `viewers_controls`, HTML links: the HTML viewer drops a link activation within
     500 ms of the previous one (`ACTIVATION_INTERVAL`, `midnight-viewer-html`
     `links.rs`: a click or double click acts once). The web and mail links were clicked
     about 400 ms apart. Clicks on HTML links now come at least 700 ms apart.
   - `viewers_thumbnails`, still view: the tile rows cut at the list's edges are requested
     and drawn too and may arrive after the whole tiles the scenario waits for. The check
     now waits until every requested tile row shows its preview before sampling its ten
     frames, and a failure names the changed frames and the tiles still loading.

## Results

Same session (`viewers_scroll`, `ui_polish_dialogs`), seconds:

| Scenario | Before | Opt only | After |
| --- | ---: | ---: | ---: |
| `viewers_scroll` | 1,211.0 | 764.1 | 476.5 |
| `ui_polish_dialogs` | 1,227.3 | 901.2 | 455.0 |

`viewers_scroll` phases, before → after: drags 216 → 73 s (763), drag landings 357 →
150 s (783; median per subject 721–811 ms → 250–319 ms), settled frames 496 → 194 s (316 →
167), still frames 240 → 101 s (323 → 226), renders 105 → 14 s.
`ui_polish_dialogs` phases: hover 307 → 31 s, focus 187 → 21 s, tooltips 59 → 12 s,
restores 218 → 106 s, state samples 83 → 14 s (576 → 585), `App::settle` 317 → 102 s.

Whole suite on Windows, `3618ff50` (before) and the final tree (after), the 25 slowest
scenarios before:

| Scenario | Before (s) | After (s) | Change | Status |
| --- | ---: | ---: | ---: | --- |
| `viewers_scroll` | 1209.3 | 476.5 | −61 % | PASS → PASS |
| `ui_polish_dialogs` | 1202.1 | 455.0 | −62 % | PASS → PASS |
| `viewers_selection` | 289.4 | 146.3 | −49 % | PASS → PASS |
| `ui_polish_checks` | 263.8 | 130.7 | −50 % | PASS → PASS |
| `ui_review` | 223.1 | 149.6 | −33 % | PASS → PASS |
| `ui_themes` | 207.3 | 141.7 | −32 % | PASS → PASS |
| `ui_polish_jobs` | 206.2 | 166.5 | −19 % | FAIL → PASS |
| `shortcuts_catalog` | 187.3 | 116.0 | −38 % | PASS → PASS |
| `viewers_files` | 129.0 | 69.9 | −46 % | WARN → WARN |
| `viewers_archives` | 125.4 | 97.9 | −22 % | PASS → PASS |
| `viewers_controls` | 102.1 | 72.6 | −29 % | PASS → FAIL¹ |
| `viewers_presentation` | 84.9 | 58.9 | −31 % | PASS → FAIL² |
| `tools_checksum_unlimited` | 82.2 | 77.8 | −5 % | PASS → PASS |
| `find_result_actions` | 80.3 | 39.3 | −51 % | PASS → PASS |
| `journeys_concurrent_jobs` | 78.6 | 92.1 | +17 % | PASS → PASS |
| `files_copy_options` | 70.8 | 51.3 | −28 % | PASS → PASS |
| `ui_polish_viewers` | 70.5 | 40.5 | −43 % | PASS → PASS |
| `panels_dialogs` | 70.0 | 48.5 | −31 % | PASS → PASS |
| `files_transfer_jobs` | 67.0 | 50.8 | −24 % | PASS → PASS |
| `files_copy_kinds` | 64.2 | 22.0 | −66 % | PASS → PASS |
| `options_buttons` | 63.6 | 46.1 | −28 % | PASS → PASS |
| `tools_find_results` | 63.5 | 22.0 | −65 % | PASS → PASS |
| `tools_jobs_copy` | 59.9 | 86.1 | +44 % | PASS → PASS |
| `files_move_kinds` | 57.2 | 20.2 | −65 % | PASS → PASS |
| `find_archives` | 56.4 | 62.3 | +10 % | PASS → PASS |
| The other 92 | 1,380.8 | 859.7 | −38 % | |
| **Whole run** | **6,513.4** | **3,618.7** | **−44 %** | |

¹ Fixed in change 5 after this run: PASS in both reruns (`20261124-051455`,
`20261124-054650`; 197 s and 130 s). `viewers_thumbnails` (27.8 s → 23.6 s, FAIL in this
run) passed in both reruns too (25.9 s, 25.8 s).
² The [PDF page key finding](#finding-pdf-page-keys-dropped); FAIL in every run with the
optimized harness (four of four).

`viewers_controls` took 130–208 s in runs of its own (before and after alike) against
72.6 s in the whole run: alone it pays for the HTML viewer's first start and the
42,225-file folder. The scenarios that got slower are bound by real copy jobs on a disk
that was 97 % full: `tools_jobs_copy` spends 1.2 s of its 86 s in `App::settle`.

## Finding: PDF page keys dropped

`viewers_presentation` presses Ctrl+PageDown six times 100 ms apart on a six-page PDF and
waits for page 6. With the optimized harness (presses about 110 ms apart instead of about
160 ms) it stopped at page 2, 3 or 5 in every run; with the unoptimized harness it
passed. Each key sends `ViewerInput::DocumentPage { frame, delta }` with the frame the
window shows (`midnight-viewer-ui` `lib.rs`, `viewer.document-next`); the engine drops an
input whose frame is older than the latest page it published
(`midnight-viewer-engine` `worker_image.rs`, `self.frames.accepts(*frame)`), so a key
pressed while the previous page is being rendered is lost. A person who presses
Ctrl+PageDown quickly, or holds it, gets fewer pages than presses. The scenario is
right. **Fixed** the same day ("Viewer: document page steps pressed before the page is
shown all count"): a page step is fenced by the document, not by the latest frame, and
steps add up against the requested page ([page steps](../developer/viewer-scrolling.md));
with the fix `viewers_presentation`'s six presses reached page 6 in every run.

## Rust tests

All 69 test executables of `cargo test --workspace --no-run` at `c8c20f49` ran one after
the other in 135.6 s (execution only; building them took 333 s with most dependencies
already built). 54 tests failed, all in `midnight-plugin-host`'s library: they need a
trusted plugin installation root, which no folder on this machine provides (as in the
21 November pass); their times are not meaningful. The slowest executables were the
engine library (49.2 s), `midnight-ui`'s integration executable (30.2 s), the viewer
engine library (9.6 s) and the comparator UI library (8.8 s); every other one took under
5 s.

Two tests dominated their executables and were changed:

| Test | Before | After | What still covers it |
| --- | ---: | ---: | --- |
| `midnight-engine` `copy_operations::tests::streamed::a_streamed_selection_keeps_about_a_hundred_bytes_per_item_beside_its_paths` | 44.1 s in the run, 58.3 s alone | 2.0 s alone | The same test with 2,500 files instead of 20,000: the retained bytes are capacities that grow in powers of two, and an eighth of the items keeps every capacity's ratio to the count, so it reports the same 119 bytes per item against the same 128-byte bound, far above the 64 prepared roots that make a selection streamed. |
| `midnight-ui` `job_progress::only_the_info_button_above_the_chart_shows_its_explanation` | 25.3 s | 0.4 s for all nine `job_progress` tests | The same assertions; the progress fixture runs on a virtual Slint clock (`TestUi::with_virtual_clock`), and the 800 ms rests for the tooltip delay move that clock instead of sleeping (also `chart_explanation_snapshots`). |

The engine library executable then took 14.3 s instead of 49.2 s (1,992 passed, 30
ignored) and `midnight-ui`'s integration executable 15.2 s instead of 30.2 s (1,127
passed, 9 ignored), together 50 s less of the 136 s.

Not changed: `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared`
(10.8 s) builds one selection beyond the real `MAX_NATIVE_TRANSFER_ROOTS` bound, which is
what it checks; the viewer engine's long-row gesture tests (5–9 s each) and the
comparator's scroll-parity tests (5–9 s each) wait for their background worker's frames
in real time, 30 ms of quiet per frame; the checksum manifest tests that the engine's
integration executables compile again were measured and kept in the 21 November pass.

## Obsolete tests

None removed. Every `#[ignore]` in `crates/` and `plugins/` (281) carries a reason that
names how to run it (an installation, a desktop, a measurement). The acceptance suite's
permanent skips each give their reason: `files_trash` and `journeys_links_readonly` on
Windows, `ui_matrix_full` without `--all-themes`, and `viewers_scroll`'s kinetic coasting
placeholder (`SKIP`, not written yet: Slint's events carry no touchpad phases), which is a
backlog marker rather than a test. No scenario touched here checks behaviour that no
longer exists or duplicates another check exactly: `viewers_controls`' keyboard and wheel
scrolling of the large log overlaps `viewers_scroll` but also records the Ctrl+End latency
the report lists, and takes seconds.

## Not changed

- The 1 % and 2 % sweep targets of `viewers_scroll` are requirement R2
  ([viewers](../developer/acceptance/viewers.md)); none was dropped or sampled.
- `settled_canvas`'s 700 ms check after a still frame stays for every movement
  measurement's end: a glide that waits at the edge of the prepared strips for a refill
  resumes later, which only that pause catches.
- `act`'s waits for a button's effect (400 ms, then 900 ms more), its two quiet samples
  250 ms apart before the click and the 500 ms after a dismissal stay: they go through the
  Engine in real time. Polling for the effect would not be faster, since a state sample
  costs about as much as the wait it would save.
- Running scenarios side by side would shorten the whole run further but would put
  timing checks (landing, glide, frame and highlighting latencies) under load; not
  attempted.
