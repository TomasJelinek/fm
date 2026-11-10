# Select all and select files only — 9 November 2026

Ctrl+A retains its all-entry behavior, including directories and directory links.
The new configurable `selection.files` action uses Ctrl+Shift+A and replaces the
selection with displayed non-directory entries. The synthetic parent row `..`
remains unselectable. F10 exposes both actions in the Selection category. Existing
explicit shortcut assignments take precedence over the new default.

The common row policy is shared by backend archive/tree selection and immediate
UI feedback. Native complete views use the directory bitset; streaming views
retain independent file/folder defaults until enumeration finishes. Archive bulk
selection now previews accepted commands immediately and retires that preview
only through the existing revision/view-checked acknowledgement path.

## Verification

Windows x64, project-pinned offline toolchain, debug/test profiles without debug
symbols or incremental output. Artifacts are under
`_build/qa/select-files-only/`. No dependency or plugin ABI changes were needed.

- The new shortcut regression first failed because the action did not exist.
  Compatibility coverage then exposed a conflict with existing user assignments;
  the new default now yields to those assignments.
- Core shortcut tests: **17 passed**.
- Engine selection-related tests: **52 passed, 1 ignored**. Coverage includes
  complete and streaming views, directory links, partial bitset words, filtered
  archive members with opaque IDs, and expanded tree selection.
- UI library tests: **628 passed**. Archive coverage checks immediate and final
  selection for All, FilesOnly and Clear, including duplicate-name member IDs.
- UI integrations: keyboard settings **20 passed**, F10 menu **5 passed**, and
  installed selection key/command/presenter bridge **1 passed**. Ctrl+A and
  Ctrl+Shift+A preserve the opposite pane and command editor, exclude `..`, and
  display the accepted result before the backend acknowledgement arrives.
- `cargo fmt --all -- --check` and scoped `git diff --check` passed.
- Clippy for core/UI with all targets and `-D warnings` passed. This was a
  package-scoped check, not a clean-Clippy claim for the whole workspace.
- Source-size check: **1,487 files**, all within the 100,000-byte limit.
- Independent scoped review found no remaining actionable issue after correcting
  archive preview admission and testing acknowledgements between bulk requests.

Commands used:

```text
cargo test -p midnight-core --test shortcuts --offline --locked -j2
cargo test -p midnight-engine --lib selection --offline --locked -j2
cargo test -p midnight-ui --features slint/unstable-winit-030,slint/unstable-wgpu-30 --offline --locked -j2 --lib --test selection_bindings --test main_menu --test keyboard_options
cargo clippy -p midnight-core -p midnight-ui --all-targets --features slint/unstable-winit-030,slint/unstable-wgpu-30 --offline --locked -j2 --no-deps -- -D warnings
```

The Slint features above match the existing application dependency configuration;
they were enabled for the test invocation without changing dependency manifests.
This is scoped validation of selection changes, not a claim that every unfinished
feature in the workspace has been fully validated on all platforms.

## Debug executable

`cargo build -p midnight-explorer --bin me --offline --locked -j2` passed.
The build still reports 13 existing engine warnings from unfinished tree-action
capture work; they are outside this selection change.

- Executable: `_build/debug/me.exe`
- Size: 86,270,976 bytes
- SHA-256: `55cdc365e097d202530f238b3515d3790f34f0b7a61b1e7ca9d53dc2f5f5fe41`

A matching private runtime was deployed and registered after verifying all 579
plugin files and the executable. Both the private candidate and `_build/debug/me.exe`
then passed native Windows startup smoke tests: exit code 0, empty stderr,
screenshots produced, and user preferences unchanged. The prior frozen runtime
installations were also unchanged. Evidence is in `runtime-deployment.json`,
`native-candidate.json` and `native-debug-build.json` in the artifact directory.
The smoke uses isolated settings and checks startup/rendering; actual selection
key handling is covered by the integration tests above.
