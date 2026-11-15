# Archive preview option implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement the assigned tasks.

**Goal:** Add “Allow previews inside archives” to Options → Previews, checked by default and controlling archive thumbnails.

**Architecture:** Extend existing preferences and the reversible Options transaction. Gate the existing archive-thumbnail admission and presentation paths; keep source reads and decoding in their existing backend workers.

**Tech Stack:** Rust, Slint, existing strict JSON settings codec. No new dependencies.

**Spec:** The user's checkbox request; legacy settings must retain the enabled default, while explicit disabling must survive restart.

## Constraints and ownership

- Debug builds only; generated output under `_build/`, caches under `.tools/`.
- Root owns the `_build/` Cargo lease and all Slint, UI-adapter, integration-test and documentation edits.
- Backend owner: core preference/default and platform-common settings codec/tests.
- Presenter owner: preference edit tracking/Options rollback and archive-thumbnail policy/tests.
- Existing unrelated changes remain intact; no Git integration in this request.
- Shared field: `UserPreferences.thumbnails_archives: bool`, default `true`.
- Shared callback: `thumbnail-options-changed(enabled, network, archives, field)`; field 2 changes archive permission.

## Review focus

- Legacy preferences default to enabled; explicit false survives strict schema 31 roundtrip.
- Turning off the setting hides cached archive pixels immediately and rejects late archive frames before the scheduling pulse.
- Native thumbnails, command artwork and the separate network opt-in retain their behavior.
- Options Cancel, Reset, OK and delayed preference loading preserve independent choices.
- The checkbox remains reachable with large fonts and narrow windows.

## Tasks

- [x] Stage the shared field and tests, then observe the settings, policy and UI failures before implementing their behavior.
- [x] Implement strict codec migration and conditional encoding without changing legacy encodings when the default remains enabled.
- [x] Implement admission/presentation/late-frame checks and reversible preference tracking.
- [x] Add the default-checked checkbox, adapter bindings and actual pointer-driven Options test with retained archive pixels.
- [x] Verify the review-discovered same-popup toggle → Reset case after preserving the archive checkbox's two-way binding through every component boundary.
- [x] Compile compatible tests once; run targeted codec, presenter, archive-thumbnail, Options and app-bridge checks, with UI tests serial.
- [x] Independently review the final code and rendered settings page; build and smoke-test the Debug executable.
- [x] Record exact verified results and artifact identity in the scoped verification report and active ledger.
