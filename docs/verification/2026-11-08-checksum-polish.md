# Checksum UI polish verification

Scope: result-first checksum window and its presentation controls. Existing
checksum calculation, captured identities, async cancellation, clipboard and safe
manifest publication remain on their original backend paths. No dependencies or
platform implementations were added.

## Changes and reference

The original window placed four rows of settings and two unrelated selectors above
results. The new view uses Results / Options / Save manifest / Details, switching
to a page selector when narrow. Only calculated algorithms appear in the output
chooser. That choice controls the displayed hash and Copy label. File, bytes and
status precede the measured-width hash; all other hashes remain available from the
selector and context menu. Ctrl+Shift+C copies the whole row.

Options are locked while calculation/export runs. Calculate requires an algorithm.
Save manifest opens a destination form; Save requires output, a nonempty destination
and any required partial-run consent. The backend still validates independently.
Consent survives page switches but resets for a new generation. The footer shows
context-relevant actions and keeps Close reachable. Errors have an icon, Details
action and dismiss control; dismissal retains diagnostics.

Reference: Salamander `src/plugins/checksum/lang/lang.rc::IDD_CALCULATE` and
`dialogs.cpp::CCalculateDialog` (configuration, result area and Stop/Save state).
This is independently authored Rust/Slint UI, with no copied artwork or code.

## Causal checks

Artifacts live in `_build/qa/checksum-polish/`.

- `red.log`: two new regressions failed against the original production UI:
  results began at y=189 instead of below 110 at 860x620, and Save dispatched an
  unacknowledged partial export (the backend would refuse it).
- `selected-hash-red.log`: a render-driven regression caught SHA-1 selection
  leaving SHA-512 as the first visible table column. The selected digest now owns
  the displayed column; header geometry is explicit rather than centered within
  a very wide cell.
- `review-final.log`: five actual-component tests passed, importing the production
  Slint files. They cover result area, selected-algorithm mapping, Ctrl+C and
  Ctrl+Shift+C tokens, locked options, missing algorithms, partial consent/new
  generations, empty destination, stopping export, page preservation, dismissing
  errors and copying diagnostics.
- The layout coverage includes 22px fonts, mouse/touch, dark/light/blue, 320x200,
  460x720 and 1000x720. The existing lifecycle/layout suite additionally checks all
  themes/densities and four pages, native-window ownership, context menus,
  cancellation, refused release and Escape.

Before/after images were inspected. At minimum size with large text, the form
scrolls and the footer stays visible; the entire form does not fit simultaneously.
Single-line destination controls remain bounded rather than expanding vertically.
Software rendering proves component layout and dispatched input, not GPU/native
desktop behavior by itself.

## Independent review

The read-only reviewer found two additional issues. Both were reproduced by
`review-red.log` before correction:

- The native combo widget assigns its own index, removing its original binding.
  Repeated choices in Results and Save could consequently show different labels
  for the same algorithm. Explicit synchronization on selected value/model changes
  now keeps them aligned. The regression switches both ways, checks visible labels
  against dispatched Save algorithm IDs, and replaces the model for a new run.
- At minimum size with large text, the header consumed the results viewport.
  Populated rows now use a single-line compact layout when short; a header is shown
  only when a full row and scrollbar still fit. The regression checks the list
  viewport, including a warning, instead of checking only the Close button.

The first broad attempts preceded these corrections (one also caught a test-only
Rust type-inference error). Only the final verification logs below qualify the
final sources; intermediate failures are retained for traceability.

## Final checks

- Windows: 507 UI library tests + five polish tests + three checksum lifecycle /
  all-theme layout tests passed (515 total, `windows-verified.log`). The run used
  `--features slint/unstable-wgpu-30` to include the application's renderer API.
- Linux/WSL: 503 UI library tests + five polish tests + three checksum tests passed
  (511 total, `linux-verified.log`).
- Strict UI Clippy, all targets, no dependencies, `-D warnings` passed with the
  same Windows feature selection (`clippy.log`). Scoped Rustfmt/diff checks passed;
  all 1,251 checked source files satisfy the 100,000-byte limit.
- Independent review was repeated after its two fixes and found no further
  important issues.

The Windows debug application build passed in 3 minutes 43 seconds. It reports
34 existing warnings in separate unfinished inline-tree work; those were not
suppressed, and scoped UI lint is not a claim that the whole workspace is clean.
The executable `_build/debug/me.exe` is 83,480,064 bytes, SHA-256
`2d15aaa62731bae300041f4e2a05d6d7668ac3f742dffff6eee428450e8118f0`.
Release was not rebuilt during this debug iteration.

The executable and existing verified plugin packages were copied with readback
checks to `C:\Users\tjelinek\MidnightExplorer-20261108-122321-7ed57705` and registered
by executable hash. Plugin trust checks remain enabled. The rebuilt debug app's
native startup smoke test passed with owned Unicode fixtures, exit zero and empty
stderr (`app-startup.json` and logs). Its screenshot was inspected. Preferences
were unchanged before/after, SHA-256
`bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`.

A separate native Windows renderer probe imported the actual checksum component
with synthetic rows, captured all four pages, exercised hash-copy dispatch and
closed through Escape. The final probe waits for `AfterRendering` before taking
each snapshot; it passed with empty stderr (`native-final.stdout.log`,
`native-final-render/`). This checks native presentation/input, not native hashing
or manifest publication. An earlier timer-only snapshot showed the two summary
lines joined; subsequent geometry inspection and the completed-frame captures
rendered them separately with unchanged production code. No speculative library
or product workaround was added for that capture observation.
