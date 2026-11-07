# Parent-row safety, deployment and keyboard-search investigation

The synthetic `..` row is navigation only. Presenter admission now refuses
empty target sets for deletion/metadata operations, while creation remains a
location-based command. Footer availability uses entry identities and resolved
selection state rather than displayed filenames. Real selected entries remain
valid targets when the caret is on `..`, matching Salamander's enablers in
`src/mainwnd1.cpp:2748–2910`.

The backend also rejects captured clipboard/menu/copy targets ending in parent
navigation, while preserving real entries beneath unnormalized directory prefixes.
The new regression failed before the fix and passes afterwards; all
three `file_targets` tests pass. The native Windows mutation test rejecting
roots, parent components and special namespaces also passes. Existing native
mutation validation and source identity pinning remain unchanged.

Windows plugin discovery intentionally rejects code below a directory another
account can replace. The deployment helper now optionally creates a desktop
shortcut to its private verified runtime. Deployment checks passed for readback
hashes, notices, protected ACLs and refusal of existing destinations, unsafe
ancestors, reparse inputs and incomplete packages. This does not make the shared
checkout itself a trusted plugin installation.

The larger persisted settings are boxed only at the save/load message boundary.
On Windows x64, `Command` is 248 bytes and `Event` is 224 bytes instead of making
every queue item carry the 464-byte preferences structure inline. Settings storage
and the public plugin protocols are unchanged. Strict Clippy passes for the
affected crates; 67 focused settings, shutdown and window-placement tests passed,
including the real native-window restore test.

## Keyboard-search crash: not yet reproduced

The reported first-character crash has not been reproduced and must not be
marked fixed solely because tests pass. Windows recorded `me.exe` terminating
through `ucrtbase.dll`, exception `0xc0000409` / subcode 7. That is not sufficient
to identify a Rust panic site or establish memory corruption.

Verification performed:

- Software-rendered typing, deletion, Unicode queries, empty results and a
  previously scrolled command selection.
- Actual Windows release, both private installation and the reported shared
  build path after dismissing its plugin warning: letters a–z and digits 0–9,
  each followed by Backspace. The query and filtering were visually confirmed.
- `native_keyboard_search` example with Winit/FemtoVG-WGPU on Direct3D, Dracula,
  Segoe UI 13: incremental queries, deletion, no results, end-of-list selection
  and filtering again. The final run also resizes the actual Windows client to
  320×200 logical pixels, asserts its native size, and restores 1340×820.
  It exits successfully with the expected final query and both size assertions.

Native logs and window captures are under `_build/qa`; the reproducible native
example remains in `crates/midnight-ui/examples/native_keyboard_search.rs`.
No speculative exception swallowing or security relaxation was introduced.

## Final integration verification

- Windows UI: 504 tests passed across the full-suite run, its remaining test
  binaries, the corrected `native_controls` geometry test and the three new
  compact-action tests. The explicit performance benchmark remains ignored.
  The older geometry assertion assumed the noncompact theme height; it now
  checks the actual footer height and that the command row stays above it.
- Linux: all 375 UI unit tests, the 43 focused UI regressions, the additional
  command/mutation menu regressions and final four compact/native-control
  tests passed. Existing backend, plugin and renderer checks are recorded in
  their feature-specific verification documents.
- Strict Clippy with `--all-targets -- -D warnings` passed on Windows and Linux
  for core, platform, backend, main UI, input, viewer and comparator crates.
  Workspace rustfmt and the 100,000-byte handwritten-source limit passed.
- The final Windows release built successfully and passed native GPU UI and
  filesystem smoke tests from the verified private installation containing all
  plugin families. Startup reported no shared-checkout trust warning; the complete
  bottom toolbar rendered in `_build/qa/ui-followups-release.png`.
- `me.exe`: 38,966,272 bytes; SHA-256
  `e2f56e068cec81a8f02e7862a6d3fb1039c90e804b2cdb8c3fb685664c6f68d5`.
  The installed copy has the identical digest. The desktop shortcut targets the
  private installation. Final smoke tests used in-memory settings; the user's
  preferences file hash remained unchanged.
