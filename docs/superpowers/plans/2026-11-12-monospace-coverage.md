# Monospace catalog coverage correction

**Goal:** Offer installed code fonts that render basic text without falling back to
proportional faces. Keep the general viewer font catalog unchanged.

**Approved scope:** `crates/midnight-viewer-render/src/fonts.rs`, its monospace
integration tests, and this task's plan/evidence. The coordinating task authorized
the correction after the exact Ubuntu CI failure. No dependency, glyph painter,
raster, UI, permission policy, font installation, or independent commit changes.

**Evidence:** Run 43122031929, Ubuntu job 130866843187, failed its existing equal
column-width assertion for Noto Color Emoji: 35.5625 versus 126.5625. The official
Ubuntu font reports fixed-pitch spacing but lacks Latin letter mappings. It has
ASCII digits; fixed-pitch metadata alone does not make it a code font.

**Design:** Retain the existing fixed-pitch requirement and require every printable
ASCII scalar (space through tilde) in the same face's cmap. Use pinned Swash APIs.
Keep literal family names, general installed-family enumeration, cancellation,
sorted deduplicated results, and completed-catalog OnceLock sharing unchanged.
No font-name blacklist or fallback-dependent width tolerance is introduced.

## Steps

- [x] Preserve exact CI log and inspect the real font's metadata and cmap.
- [x] Add a basic-text coverage regression and reproduce the original width failure
  with an isolated Fontconfig fixture under `_build/qa/monospace-20261112`.
- [x] Add the narrow coverage eligibility check in `monospace_families`.
- [x] Run focused native Linux tests with the real font and the ordinary catalog;
  run formatting/lint and coordinate Windows verification with the root lease.
- [x] Record package/version/hash, RED/GREEN results and unchanged general catalog;
  request independent review. Keep exact-head CI status separate from local passes.

The isolated fixture uses an official Ubuntu package extracted under `_build` and
an explicit child-process Fontconfig configuration; it does not change installed
fonts or user configuration. The shared checkout and target leases are required by
the coordinating task; no additional worktree or commit is created.

## Verification ledger

- Linux RED with real Noto fixture: both original width and new coverage tests
  failed; widths exactly matched CI, 35.5625 versus 126.5625.
- Linux GREEN: fixture 4/4 and ordinary fonts 4/4; targeted strict Clippy passed.
- Coordinated renderer lib/raster/source-continuity follow-up: 14/14 passed.
- Rustfmt applied through the existing Linux toolchain; diff whitespace and LF
  checks passed. Independent root and renderer-owner reviews found no issue.
- Linux build lease released to the tree backend task; root owns final Windows
  qualification and combined integration. No independent commit was made.
