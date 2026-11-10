# Find status, sorting and compact results

The source implementation adds Masks/Regex/Expression matching, live scanned
directory status, five sortable columns and one compact row per result. Native
search remains read-only; content matching remains literal UTF-8. No dependencies
or saved settings were added by this slice. See [FIND.md](../developer/FIND.md).

The directory slot is updated before native root open/enumeration, and sampled
by the async controller at most four times per second while the worker can be
blocked. Terminal/Stop/restart/Close guards prevent stale status revival.
Sorting uses the same engine locale as panels and numeric size/native-time
comparisons. The backend publishes a bounded permutation and inverse map; the
discovery-order vector remains authoritative for Go to file validation. Selection
uses stable IDs without a UI sort or full-list scan, including streamed tails.
Sort workers wait on notifications when idle and coalesce active changes.

## Causal evidence

Artifacts are under `_build/goal/find-refactor/`:

| Regression | RED | GREEN |
| --- | --- | --- |
| Status during blocked native open | `current-directory-red.log`: no directory arrived | `backend-final3.log`: root open, nested enumeration and content-read cases pass |
| Locale/numeric sorting and cancellation | `sort-red.log`: discovery order and ignored cancel | `backend-final3.log`: all three sort cases pass |
| Sort after Stop | `after-stop-red.log`: partial 64 results could not reorder | `backend-final3.log`: fresh order token and retained partial results pass |
| Header/cell geometry | `header-geometry-red.log`: header y386, first row y313; screenshot showed centered text | `header-geometry-green.log`: explicit top header/full-cell alignment and actual geometry assertions pass |

The actual ComboBox fixture initially assumed Home/End support. Pinned Slint's
common ComboBox implements Up/Down/Return; corrected pointer-open/Down/Return
checks now verify emitted modes 0/1/2 without a product workaround.

## Debug qualification

- Windows: current Main UI library 494 and Find window 4 pass in
  `windows-final-ui.log`, including all five real pointer headers, mode choice,
  aligned single rows, minimum-size behavior and every theme/density.
- Linux: Main UI library 490, parent focus 1, frame resize 1, metadata editor 2 and
  drive details 1 pass in `linux-ui.log`. After the isolated geometry correction,
  current Find window 4 pass in `header-geometry-green.log`; backend 19 pass in
  `backend-final3.log` (actor 5, predicates/traversal 7, status 3, sort 3, Stop/sort 1).
- Common archive/bookmark/viewer preference follow-ups 11 and native drive
  information 4 were also rerun for their owner.
- Focused backend strict checks passed. Final UI/app strict attempts on both
  platforms stopped at the separately owned viewer-engine `collapsible_if`
  diagnostic at `src/lib.rs:248`; no whole-workspace/strict completion is claimed.

Standalone full Main Slint generation passes (`slint-check-final.log`). The
populated Linux captures were inspected at 1400×720 and 320×200:

- `_build/goal/find-refactor/renders/find-populated-wide-1400x720.bmp`
- `_build/goal/find-refactor/renders/find-populated-minimum-320x200.bmp`

Headers sit above rows; filename/path/time columns align, size is right aligned,
and the selected row remains visible after shrinking. The compact busy form
gives its disabled editor area to results while retaining the draft. Current
directory and progress stay separate, and Close remains reachable. Historical
incorrect screenshots are retained as `red-header-wide.bmp` and
`red-header-minimum.bmp`. The headless Linux font set does not supply every CJK
glyph; raw paths and full accessible labels are preserved. These are populated
software-window fixture captures and pointer/key tests, not native desktop/GPU
tests. Root owns final application deployment/native qualification.

Native-dialog migration and the ordinary-Move success-notice cleanup remain
separate pending work; this Find slice does not close those requirements.
