# Viewer images and file colors implementation plan

> **For agentic workers:** Use superpowers:executing-plans or
> superpowers:subagent-driven-development task by task, with focused review.

**Goal:** Complete binary/text viewer controls, add an independent zoomable image
viewer and configurable file color rules; verify existing expression filtering.

**Architecture:** Keep independent plugin parsers and tests separate from host
contracts. Restricted image workers return bounded rasters. Backend matching and
rendering publish immutable presentation snapshots to Slint.

**Tech Stack:** Existing Rust 1.99, Slint 1.18.1, Tokio, Parley, image 0.25.10,
moxcms 0.8.1 and bounded regex engine; no paid dependencies.

**Spec:** `docs/superpowers/specs/2026-11-07-viewer-images-file-colors-design.md`.

## Constraints and review focus

- All generated output under `_build`; handwritten files <=100000 bytes, rustfmt100.
- Native code/tests separated per platform; Unicode source identities preserved.
- Plugins import public API/SDK only and own their tests/build scripts.
- Exercise stale results, close/cancel during decode, enormous dimensions/lines,
  selection contrast on every theme, and late settings load after local edits.

## Task 1: Basic viewer behavior

- [x] Inspect Salamander and current basic plugin/host routing.
- [x] Add failing tests for automatic binary routing, text wrap and searched text.
- [x] Implement wrap in viewer input/frame/rendering, retain binary fixed columns.
- [x] Verify plugin-owned tests and host renderer/engine/Slint input tests.

## Task 2: Restricted image plugin and public protocol

- [x] Define/test a bounded public raster-worker protocol and host supervision.
- [x] Implement independent image plugin with pinned thumbnail codecs, orientation,
  color conversion, cached decoded pixels and viewport-only raster output.
- [x] Test malformed input, resource limits, protocol validation, zoom sampling and
  standalone Windows/Linux plugin builds with released public API packages.

## Task 3: Image host/window integration

- [x] Add tested discovery/routing for removable process viewer packages.
- [x] Connect source leases, cancellation, zoom/pan/fit/1:1 to the common viewer.
- [x] Verify native package loading/removal, image display, controls and Space/Escape.

## Task 4: File color rules

- [x] Consult Salamander color rules; test ordered matching, invalid patterns and
  strict preference persistence before implementation.
- [x] Add backend rule compilation/matching and immutable per-entry style results.
- [x] Add reusable contrast/state derivation and Colors settings with previews.
- [x] Verify all themes/views, selection states and late-load settings behavior.

## Task 5: Integration and review

- [x] Verify Ctrl+F12 filters and document actual syntax/limits.
- [x] Update design/API/hotkeys/license inventory and packaging/build scripts.
- [x] Run relevant Windows/Linux tests, strict Clippy, formatting and size guard.
- [x] Review changes, rebuild release, verify native UI and commit/push source only.
