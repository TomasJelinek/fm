# Native controls and full path implementation plan

**Goal:** Apply the user's latest navigation, icon and complete-path requests
above verified commit `d4ef58a`, preserving the backend/UI boundary.

**Design:** [Native controls and full path](../specs/2026-11-04-native-controls-and-full-path-design.md).

## Ownership

- Backend worker: core icon roles/keys, engine icon scheduling/cache, separated
  Windows/Linux icon adapters and meaningful backend/platform tests.
- UI worker: all `fm-ui` choices/presenter/model/callbacks, named icon palette,
  Slint controls/path layout and interaction regressions.
- Root: documentation, integration review, complete verification, native QA
  and the follow-up source-only Git commit. No overlapping source ownership.

## Tasks

- [x] Add semantic icon contracts and bounded prefetch/negative-cache behavior.
- [x] Implement native stock/known-folder and host-theme icon adapters.
- [x] Share stable navigation choices between quick buttons and dropdowns;
  retain aliases, native targets, remembered directories and tooltips.
- [x] Use the named host-icon palette without tinting native pixels, retaining
  clear vector fallbacks when a platform lacks a suitable icon.
- [x] Display the full wrapped command directory with a bounded viewport for
  extreme paths and a usable command editor at all supported densities/fonts.
- [x] Run focused regression tests and independent code review; fix findings.
- [x] Update current README/security/design and verification records.
- [x] Complete Windows/Linux checks, inspect native Windows rendering and
  launch the verified application.
- [x] Inspect staged paths and commit the verified native-controls changes.
- [ ] Push to origin/main and report the verified remote hash after the user's
  subsequent bottom-button alignment adjustment (explicitly requested).
