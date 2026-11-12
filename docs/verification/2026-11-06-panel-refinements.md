# Panel refinement verification checkpoint

This records observed checks before the user-requested deletion of the entire
`_build` directory. It is a checkpoint, not a claim that later changes were tested.

- Windows UI library: 327 tests passed after error formatting and metadata refresh
  changes. Linux UI library: 324 passed at the corresponding platform revision.
- Metadata preview Windows backend: 15 matching tests passed, including native
  refresh, replaced-target refusal and proposed metadata projection. The earlier
  Linux run passed 15 before the final projection change.
- Metadata real UI: virtualized 256-row preview, exclusive attribute controls,
  timestamp enable/Now, stale draft gating and four tabs passed. Render review
  covered desktop/touch and font sizes 11/32. The review found and fixed centered
  list headers, a long timestamp label overlap and narrow error-card content.
- F7 native Windows tests: create_file 9 and mutations 20 passed. Linux create
  module: 15 passed, including an explicit cross-mount absolute target. Real UNC
  or separate Windows drive access has not been exercised on this machine.
- Linux delete confirmation: 1 passed. Linux error-notice integration: 3 passed,
  covering full Unicode path, backend Copy details, persistent popup, Escape,
  informational severity and a minimum-size panel error band.
- Windows error formatter RED was observed with two expected single-line-output
  failures; the implementation then passed in the full UI library run.
- Python packaging/license scripts: 22 passed; inventory consistency check found
  424 package/version/source records, 405 crate names and 46 direct dependencies.

Known outstanding verification at this checkpoint:

- Linux `keyboard_options_validate_apply_disable_and_restore_effective_bindings`
  failed because no preference-save command arrived after the old editor interaction.
  The other 9 tests passed. The new keyboard editor redesign must preserve this
  validation/persistence behavior and replace the old interaction fixture.
- The new Windows keyboard editor workflow test failed as expected because the
  current editor has no `Search commands` control. Implementation is pending.
- Selection-aware directory-size and actual-error-glyph tests have expected RED
  evidence; their agent is still implementing the behavior.
- The Linux multi-target command stopped at the keyboard failure; subsequent
  metadata/panel/theme/tab targets in that invocation were not verified.

Media and archive work remains in independent worktrees and is not included in
these application counts. Their dependency additions require a fresh inventory.

After cleaning, regenerate viewer SDK bundles with `scripts/build-viewers.ps1` or
`scripts/build-viewers.sh`; those scripts create their required SDK configuration.
The local Linux toolchain/native dependencies remain in `.tools`; only target and
temporary output under `_build` is disposable. Recapture native Cargo metadata/tree
inputs before checking the generated license tables again.

## Clean rebuild and subsequent Windows checks — 2026-11-07

The complete `_build` tree was removed after stopping its build processes and
verifying the absolute target and reparse points. It occupied 183.26 GiB. A clean
offline, locked Windows release app build and all three independent viewer builds
completed with exit code 0. The new tree was 2.56 GiB and D: had 214.57 GiB free at
that checkpoint. Later test builds and new feature work are separate from this
release snapshot; do not assume the earlier EXE contains subsequent edits.

Observed subsequent checks:

- Windows keyboard editor: 14 actual interaction tests passed, including alias
  add/remove/restore, conflict rejection, native text editing, physical digits,
  AltGr, and Tab/Escape capture exit. A duplicate logical-digit capture failed first
  and was corrected in the popup's own key handler.
- Settings layout: desktop/touch and 12/14/32-pixel font footer bounds passed.
  Dark/light normal and large-font settings were visually inspected. The complete
  dialog layout matrix passed eight themes, both densities and large fonts.
- Jobs: actual row selection survives a newly inserted job, Cancel leaves the
  popup open, Escape closes without cancelling another job — 1 test passed.
- Windows UI library: 354 tests passed after archive and selected-folder summary
  integration. Focused size backend subsequently passed 37 library and four
  end-to-end tests. Actual metadata editor and size-error integrations also passed.
- Windows native regressions: locations 2, empty-file creation 9 and mutations 20
  passed after native location and F7 path-prefix changes. Strict UI and Windows
  platform Clippy passed all targets with warnings denied.

Review found an Options category focus mismatch: Down changed the visible category
but Enter activated the previously focused category. The actual-key regression
failed with index 0 instead of 1; the focus correction passed the actual-key
regression. Media integration, the new Previews category, final summary publication
coalescing and final Linux UI checks remain outstanding at this checkpoint.

## Final combined-source checks — 2026-11-07

- Windows UI library: 360 passed, including installed archive routing, thumbnail
  stale-frame/animation bounds, delayed network classification and late preferences.
- Actual Windows UI interactions: 29 passed across keyboard/Previews (15), Jobs (1),
  errors (3), metadata (1), panel activation (7), size glyphs (1) and the all-theme
  dialog matrix (1). The matrix now visits all seven settings topics.
- Windows core library: 24 passed. Engine library: 174 passed, one opt-in test
  ignored; selected-folder integrations: 5 passed. The streaming-selection fixture
  was updated to distinguish excluded directory-link totals from unknown file-link
  byte sizes; both behaviors are explicitly asserted.
- Windows common platform: 36 passed, including versioned archive/preview preferences,
  native path identities, atomic saving and independent copy readback corruption tests.
- Full Windows application workspace strict Clippy, all targets: passed after review
  fixes. Source-size guard: 680 handwritten source files, all at most 100,000 bytes.
- Native Windows/ Linux dependency graphs were captured for twelve workspaces:
  446 package/version/source records, 422 crate names, 51 direct records. Python
  packaging/inventory/build-hook regressions: 27 passed. All declared licenses are
  present; media distribution notices retain the reviewed missing-license override.

Thumbnail cross-review found and corrected queued-I/O timeout cancellation,
source revision pinning across video samples, stale memory cache recovery after
another instance clears disk cache, mixed-case Windows DLL trust classification,
and network opt-in during delayed storage classification. Native helper runs are
recorded separately in the thumbnail verification ledger. Final release build and
Linux UI verification were still running when this checkpoint was written.

Final Linux checks also passed: UI library 357 and actual interactions 28;
engine directory sizes 39, metadata 15 and selected-size integrations 5; core 24;
native Linux platform 100 with 14 opt-in tests ignored. The strengthened streamed
file/directory-link regression passed separately. Large-font metadata and size-error
renders were inspected. Every one of the twelve Rust workspaces passes rustfmt.
The final inventory was recaptured after the last dependency change and its
consistency check passes with the same 446/422/51 counts.

Final Windows release build completed successfully after the last discovery fix.
The default-renderer native UI smoke run from a new private installation exited 0,
using in-memory test preferences. Its snapshot was inspected: both panels, Unicode
names, full command path and the entire bottom action row render correctly. The
installation includes all three viewer libraries, archive worker, image/video
workers, required runtime libraries and license/source notices. The executable is
41,096,192 bytes. Final `_build` size is 14.17 GiB (46,115 files), with 203.03 GiB
free on D:, compared with the deleted 183.26 GiB tree.
Release EXE SHA-256: `a1a54b2883220f78ee284d54645f9b3a1c00cc99aa410181d45523a532f85118`.
