# Checksum UI polish implementation plan

> Execute inline using the executing-plans and verification skills.

**Goal:** Make calculating, inspecting, copying and exporting checksums clear.
**Architecture:** Retain modeless backend sessions; reorganize Slint pages and
reuse bounded presentation components and the shared footer.
**Tech stack:** Existing Rust/Slint; no dependency changes.
**Spec:** ../specs/2026-11-08-checksum-polish-design.md

## Constraints and review focus

Preserve captured identities, async cancellation, partial-output consent and
320x200 minimum. Review late generations, empty/no-algorithm drafts, large fonts,
unavailable output algorithms and long Unicode paths. Retain all existing work;
do not stage or commit the unrelated dirty worktree.

## Implementation

- [x] Add failing result-area and export-consent UI regressions to checksum tests;
  render the current component to `_build/qa/checksum-polish/before`.
- [x] Rework `checksum-window.slint` into result-first pages with shared footer;
  extract small repeated controls only where reused. Update presentation mapping
  in `src/checksum_windows.rs` if necessary for available algorithm selection.
- [x] Reorder `checksum-results.slint` status before hashes; preserve context-copy
  tokens and add explicit full-row keyboard copy with no uncalculated output.
- [x] Test page navigation, frozen run options, export consent/generation resets,
  cancel/save, minimum and large-font layouts, all palettes and existing lifecycle.
- [x] Inspect renders, fix findings, run Windows/Linux focused checks, scoped
  Clippy and debug build; record evidence and update feature/design documentation.
