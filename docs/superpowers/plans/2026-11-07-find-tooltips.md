# Find Tooltip Host Registration Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. No delegated agents or commits for this task.

**Goal:** Place shared hints relative to each native Find host with the existing cursor gap and edge clamping.

**Architecture:** Extend the UI-thread weak host registry to Main and Find components. One common geometry/anchor path owns placement; only Main dispatches footer-press, panel-pointer and drag observers.

**Tech Stack:** Existing Slint 1.18.1 event hook and tooltip lowering adapter; no dependency or generated-code edits.

**Spec:** Existing `tooltip.rs` contract plus `docs/developer/FIND.md`.

## Constraints and review focus

- Preserve the embedded-popup coordinate contract and prior event-hook chaining.
- Registry borrows end before Slint properties or callbacks are evaluated.
- Each host reads its own cursor/client bounds; Find must not alter Main state.
- Keep the existing 14px/18px cursor gap and shared placement implementation.
- Observe input without consuming clicks or invoking Main drag/footer actions for Find.
- No filesystem work, OS geometry queries or unsafe Rust in UI callbacks.

## Steps

- [x] Add focused tests for independent Main/Find cursor/resize/exit tracking and
  an actual timed Find hint near the right edge of a 360×620 host.
- [x] Observe missing Find registration fail.
- [x] Add `tooltip::install_find(&FindWindow)` and a common typed host registry.
- [x] Root wires installation/presentation synchronization into Find creation/refresh; no root Find files edited here.
- [x] Run focused tooltip unit tests and existing full tooltip hover/click regression.
- [x] Run affected strict Clippy and report host geometry evidence to root.

## Evidence

- Missing `install_find` compile failure: `_build/goal/find/tooltip-red.log`.
- New focused tests2 passed: `_build/goal/find/tooltip-green.log`, including an
  actual timed tooltip rendered by the software backend, right-edge flip with
  14px gap, 8px host margins and independent Main/Find cursor/client bounds.
- Root's final Windows batch passed UI392 library tests, Find2 integration tests,
  dialog2, file-actions7 and existing Main tooltip1; `_build/goal/find/ui-final.log`.
  Strict UI all-targets Clippy passed in `_build/goal/find/ui-clippy.log`.
  Native WGPU qualification remains separate from these software input tests.
