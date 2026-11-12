# Production tree controller integration

The qualified controller candidate was ported into the actual root workspace on
2026-11-09, preserving the newer Network protocol, navigation fences and snapshot
fields. This is production backend routing through `Engine::run`, not another
unintegrated source copy. The native-plugin application cutover committed during
integration is `a578b6fc29fd987dc0e186cf0541a1c4b325c55c`.

## Scope and behavior

`Command::Tree` and `Event::Tree` connect bounded, persistent workers for both
panes. Native/archive completion hands off its exact entries and index before
publishing the completed listing. Enable, Commit, branch/viewport requests,
occurrence selection and typed native/archive navigation use the existing
backend session. View and selection publish atomically; exact projection Arc
acknowledgment remains mandatory. Equal contents or reconstructed display paths
do not grant source authority.

Listing and branch permits remain owned through real provider return, completion
application and retirement. Navigation, collapse and shutdown cancel stale work
without replacing the permit of a blocked callback. Archive navigation targets
retain their registrations across pane replacement and engine shutdown. Root
budget/provider failure preserves the complete ordinary listing. The resource
and cleanup contract is in [TREE_CONTROLLER.md](../../crates/midnight-engine/TREE_CONTROLLER.md).

Two integration-specific additions were made after reviewing the frozen
candidate:

- Network navigation invalidates the old tree session along with native/archive
  navigation. Completed-root capture rejects a competing Network generation,
  including an older retained archive root. A retained older Network snapshot
  does not block a newer archive root. The added archive assertion failed before
  the guard and passed afterward.
- `TreeDisplayIndex::root_rows` maps original root entry IDs to visible occurrence
  IDs for constant-time UI focus/disclosure adoption. Its map replaces transient
  duplicate-alias validation and is charged under the projection byte limit.
  Descendants and filtered-out roots are absent. The new API test first failed
  because the field was missing, then passed after implementation.

No dependencies, plugin APIs, native action proofs or Git index entries were
changed by this backend integration. The presenter is a separate coordinated
change. Descendant operation consumers, container discovery/attachment, panel
filter/prefix routing and branch watches remain outstanding TREE-01 work; this
report does not claim full feature completion.

## Fresh Windows verification

All commands ran from the root workspace, using `.tools/cargo`, `.tools/rustup`,
the configured `_build` target, `_build/tmp` for TEMP/TMP, incremental compilation
disabled, debug information disabled, Cargo `--locked --offline`, and two jobs.
No Network discovery command or LAN scan was requested.

| Gate | Result | Log under `_build/qa/tree-controller/` |
| --- | --- | --- |
| Core library | 30 passed | `integrated-core-engine-units.log` |
| Engine library, including typed capture/security, lifetime and Network fixtures | 477 passed, 1 optional benchmark ignored | `integrated-core-engine-units.log` |
| Core tree projection and selection ranges | 22 passed | `integrated-core-tree-green.log` |
| Real Engine tree boundary | 13 passed | `integrated-engine-boundaries.log` |
| Ordinary native/archive navigation, selection, search and Unicode | 12 passed | `integrated-engine-boundaries.log` |
| Core/engine all-target Clippy | passed with `-D clippy::all` | `integrated-core-engine-clippy.log` |

Final distinct executions total **554 passed, zero failed, one ignored**. The
earlier focused run additionally passed 62 tree-related unit tests and one
matching boundary test; those duplicate executions are not included in the
total. `integrated-network-root-red.log` and `integrated-root-index-red.log`
record the two new regression/API failures before their changes.

The exact successful test/lint arguments were:

```text
cargo test --locked --offline -j 2 -p midnight-core -p midnight-engine --lib
cargo test --locked --offline -j 2 -p midnight-core --test tree_view --test selection_ranges
cargo test --locked --offline -j 2 -p midnight-engine --test tree_controller --test navigation --test archive_selection --test search --test unicode
cargo clippy --locked --offline -j 2 -p midnight-core -p midnight-engine --all-targets -- -D clippy::all
```

Clippy still reports 13 unused/dead-code Rust warnings from the separately
unrouted action-capture consumers and branch request accessors. This is not a
claim that `-D warnings` passes. All 19 owned source/contract files passed scoped
rustfmt at width 100, `git diff --check`, strict UTF-8 decoding, LF line endings
and the 100,000-byte source limit. Their paths and hashes are recorded in
`_build/qa/tree-controller/integrated-source-manifest.json`.

The original candidate's RED/GREEN and resource evidence remains in
`_build/qa/tree-controller/verification.md`. Installed-plugin and app/Slint
qualification remain separate; the coordinated presenter work requires its own
combined verification.

## Fresh Linux verification

The root integration was also tested on WSL Ubuntu 24.04, using the pinned local
Linux toolchain and offline dependencies. Temporary permission fixtures used the
native Linux filesystem rather than DrvFS. Core units passed 30 tests, engine
units 468 tests (one optional benchmark ignored), projections/ranges 22 tests,
and the Engine controller boundary 13 tests: **533 passed, zero failed, one
ignored**. Core/engine all-target Clippy passed with `-D clippy::all`, retaining
the same 13 unused Rust warnings described above.

Logs are `_build/qa/tree-controller/integrated-linux/{units,tree-projections,controller,clippy}.log`.
This verifies the integrated backend source; it does not qualify Linux desktop
input or claim complete descendant action support.
