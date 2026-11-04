# Navigation and appearance implementation ledger

Work in the user's existing non-Git workspace. The design and implementation
are authorized; no new approval or repository initialization is required.
Preserve platform separation, backend-only filesystem work and disabled write
operations. Specification: [navigation and appearance](../specs/2026-11-04-navigation-and-appearance-design.md).

- [x] Inspect Salamander accelerators, hot paths and native drive presentation;
  consult Total Commander tab conventions without copying code/assets.
- [x] Add compact footer controls, remove pane status bars, show incomplete
  directory sizes with text and color, and map implemented commander shortcuts.
- [x] Add backend font discovery, independent density base sizes, virtual font
  selector and responsive Options controls.
- [x] Preserve native pixel scroll/inertia and settle gently after input stops.
- [x] Wire safe Ctrl+Enter/Ctrl+P insertion and exercise the installed bridge.
- [x] Add native-root icon keys, bounded backend lookup and per-OS implementations.
- [x] Add typed bookmark/tab identities, native codec migration and independent
  per-pane tab navigation, default folder captions and stale-result rejection.
- [x] Add compact accessible tab controls and named/numbered bookmark controls.
- [x] Derive native window title version0.1 from Cargo SemVer0.1.0.
- [x] Compact the Touch profile to40px controls and smaller chrome/gaps, preserving
  existing font choices; verify affected keyboard/pointer/render behavior.
- [x] Resolve initial preferences-load/local-edit race without dropping saved paths,
  including bounded reconciliation after the window closes.
- [x] Independently review changed navigation, UI bridge and settings contracts;
  correct per-field edits and deferred-save shutdown handling found in review.
- [x] Run complete Windows/Linux tests, strict Clippy, formatting and size guards;
  repeat the eight affected Touch UI tests after the final density change.
- [x] Build both applications, inspect fresh native Windows screenshot, launch
  the verified interactive application and record exact verification evidence.

Evidence is kept in `.superpowers/sdd/navigation-appearance` and
`.tools/navigation-appearance-*`. Earlier scoped RED/GREEN logs use
`.tools/navigation-*`, `.tools/commander-*`, `.tools/font-backend-*` and agent
handoff paths. The verification report must distinguish headless interaction
tests, Linux engine smoke and native Windows rendering.
