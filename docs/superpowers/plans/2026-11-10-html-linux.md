# Linux Chromium Viewer Completion Plan

> Use `superpowers:subagent-driven-development`; independent provider work may
> proceed beside native tree-watch work. The coordinator owns integration leases.

**Goal:** Build, package and qualify the existing embedded Chromium viewer on Linux.

**Architecture:** Preserve the accepted CEF process-plugin/API design. Complete
Linux runtime packaging and input where native evidence requires it; qualify the
actual supervised host and Slint surface under the existing user cgroup delegation.

**Tech stack:** Existing pinned CEF/cef-rs, public web-viewer API, Rust and Linux
system libraries. No new browser engine, host-private plugin dependency or sandbox bypass.

**Spec:** [Embedded Chromium](../specs/2026-11-09-html-viewer-design.md).
The read-only environment/source audit is `_build/qa/html-linux/audit.md`.

## Global constraints

- Preserve unrelated dirty source and the already qualified Windows provider.
- All artifacts under `_build`; toolchain/dependency caches under `.tools`.
- Free-of-charge dependencies; verify exact runtime/tool licenses, record pinned
  hashes and notices. Do not let cef-dll-sys download an unvalidated fallback.
- Use an owned private Linux installation. Keep Chromium namespace/seccomp sandbox
  enabled and the existing memory/pid cgroup limits. No root-cgroup reconfiguration.
- Separate platform implementations, rustfmt 100 columns, source <=100000 bytes.
- Linux provider target is `_build/html-viewer/provider-target/linux`; request the
  coordinator's lease before any shared main-workspace Linux build/deployment.

## Review focus

1. An invalid/mismatched CEF_PATH fails before Cargo, without an implicit download.
2. Cleared launch environment still resolves only the intended sibling libraries.
3. Closing/reset/crash reaps renderer/GPU/network processes and releases the cgroup.
4. Editable content consumes text/modifiers correctly without losing viewer Escape/Find.
5. XWayland evidence is labeled correctly; it does not imply physical IME or native Wayland proof.

## Task 1: Native prerequisite and deterministic runtime package

Owned files: `plugins/midnight-viewer-html/scripts/build-linux.sh`, platform-specific
link/bootstrap files, independent packaging tests and plugin documentation/notices.

- [ ] Run the existing native cgroup containment test via `systemd-run --user`
  using the existing owned delegated subtree. Record exact exit/containment result.
- [ ] Obtain the exact pinned Linux CEF runtime from its authoritative distributor,
  verify published archive digest plus local SHA-256, inspect actual notices and
  stage only beneath `_build`. Prepare required free native tools/libraries in `.tools`.
- [ ] Add failing tests for invalid CEF metadata/digest and incomplete package input;
  make the Linux helper validate before invoking Cargo and use a Linux registry path.
- [ ] Set literal sibling ELF lookup (`$ORIGIN`) and produce complete notices,
  dependency metadata and runtime file hashes. Check actual ELF dependencies with
  `readelf`/`ldd` in the launcher's cleared environment.
- [ ] Build/test the independent plugin against the public registry API; preserve
  exact dependency versions/features unless a demonstrated requirement forces a reviewed change.

## Task 2: Native interaction and installed viewer

Owned files: `plugins/midnight-viewer-html/src/platform/linux.rs`, focused separate
Linux mapping helpers/tests and existing independent native-runtime tests if needed.
Host/supervisor changes require a reported defect and coordinated ownership first.

- [ ] Qualify rendering, source-relative assets, offline script/network denials,
  Find/Next, text selection/Copy, editable Space/Enter/Shift+Enter, zoom/resize and close.
  Add behavioral failures before correcting Linux input/lifecycle defects.
- [ ] Run the actual host and Slint viewer under the delegated user manager with
  explicit WSLg display environment. Inspect rendered frames, not only exit codes.
- [ ] Verify policy context replacement, stale frames, provider crash/close and
  whole-tree cgroup cleanup; retain ownership while slow processes actually exit.
- [ ] Obtain independent scoped code/security review and fix confirmed defects.
- [ ] Update HTML verification/design status, dependency inventory and build guidance
  with exact hashes and platform limits. Coordinator integrates the final app only
  after the concurrent tree-watch source freezes and affected gates pass.
