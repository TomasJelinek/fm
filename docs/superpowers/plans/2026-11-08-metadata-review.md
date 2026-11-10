# Attributes review crash and compact controls

**Goal:** Fix the reported Review changes crash and reduce attribute-control clutter.
**Architecture:** Keep the existing backend preview/confirmed-operation contract.
Use one three-state attribute control per row, preserving Leave/Set/Clear semantics,
with clear state text, keyboard access and tooltips. This follows Salamander's
`CChangeAttrDialog` and `IDD_ATTRIBUTES` tri-state controls without copying code.
No new dependencies; all native inspection and mutation remain in the backend.

The user's existing implementation authorization covers this focused repair. The
current dirty checkout must be preserved; no unrelated resets or broad staging.

- [x] Reproduce Review changes through installed callbacks and backend events;
  identify the failing boundary and add a regression before changing product code.
- [x] Repair that boundary, preserving stale-preview, cancellation and Apply guards.
- [x] Replace repeated attribute buttons with compact, accessible tri-state controls;
  test all three states, unsupported controls and responsive geometry.
- [x] Run relevant UI/backend suites, inspect rendered layouts, update design and
  verification notes, build debug and verify startup with the existing plugins.

Review focus: deferred UI callbacks, editing after review, unchanged/invalid drafts,
capability masks, multi-selection semantics and 320 × 200 / large-font usability.

Additional steering in this task:

- [x] Verify Delete/F8 vs Shift+Delete/Shift+F8 dispatch and confirmation.
- [x] Center close glyphs and Hot Paths split-button hit regions; fill missing icons.
- [x] First visible application frame uses loaded theme and restored monitor geometry.
- [x] Missing panel paths ascend to an accessible ancestor, then the platform root.
- [x] Save manifest opens the native Save dialog directly.
