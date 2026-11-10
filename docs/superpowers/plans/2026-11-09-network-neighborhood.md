# Network Neighborhood implementation plan

> **For agentic workers:** Use superpowers:executing-plans or the existing
> coordinated parallel workers; preserve unrelated active work and scoped commits.

**Goal:** Browse network containers, devices and shares inside each panel, with
Salamander-style cached asynchronous discovery on Windows and Linux and a clean
extension point for a later macOS implementation.

**Architecture:** An independently built dynamic filesystem plugin owns platform
discovery and a bounded shared cache. Public Browse calls return typed virtual
nodes. Only an explicit Activate request can hand a selected share to the host's
native filesystem; no fabricated paths or native operations on discovery nodes.

**Tech stack:** Rust, existing public filesystem C ABI/SDK, Windows Unicode MPR,
Linux GIO/GVfs, Tokio host workers, existing Slint panels.

**Spec:** [Network design](../specs/2026-11-05-network-filesystem-design.md).

## Global constraints

- Public API/SDK only for plugins, independent Cargo workspace and tests.
- Separate Windows/Linux files; macOS later, without a misleading Linux fallback.
- Fee-free dependencies with actual pinned license review and distribution notices.
- No live network scan in ordinary tests; no subnet/port scan in production.
- No automatic mount, credential prompt or content open during listing/hover.
- 2 discovery calls application-wide; 128 nodes/page; 4096 nodes and 16 MiB cache;
  TTL 30 seconds; 10-second request deadline with late results discarded.
- Build/test output under `_build/`; compiler/dependency caches under `.tools/`.

## Review focus

- The same device caption on different scopes must retain different identities.
- A slow native call must not occupy the UI or create replacement worker threads.
- Restore/hover/listing must never trigger activation or authentication.
- A remote URI is not a native path; absent mounting cannot grant file operations.
- A removed plugin or unavailable discovery service must preserve manual access.

### 1. Public virtual address and activation boundary

Files: `filesystem-api/midnight-filesystem-plugin-api/src/activation.rs`,
`src/model.rs`, `src/abi.rs`, `src/wire/{encode,decode,validate}.rs`;
`filesystem-api/midnight-filesystem-plugin-api/tests/activation.rs`;
`filesystem-api/midnight-filesystem-plugin-sdk/src/native.rs`.

- [x] Add failing activation codec/admission tests for captured node identity,
  Unicode/long/raw paths, virtual/native confusion and malformed/trailing data.
- [x] Add capability-gated `Request::Activate { mount, node }` and typed native
  handoff response; malformed/foreign encodings fail before host navigation.
- [x] Keep native adapters source-compatible through explicit Unsupported handling;
  run API/SDK tests including old operation tags and descriptor compatibility.
- [x] Document the additive ABI 1.2 activation/refresh/manual-resolution contract;
  review found and fixed UNC activation validation and late-publication races.

### 2. Independent network plugin policy and platform adapters

Files: `plugins/midnight-filesystem-network/{Cargo.toml,src/lib.rs,src/catalog.rs,
src/address.rs,src/windows.rs,src/linux.rs,tests/catalog.rs,README.md}`;
platform-specific FFI/tests split into submodules as needed.

- [x] Write address codec tests for root/container/server/share identity, credentials,
  Unicode, scoped duplicate captions and parent restoration; then deterministic
  adapter-fixture tests for incremental pages, TTL reuse,
  duplicate scopes, empty/unavailable/offline/auth states, cancellation and bounds.
- [x] Implement backend-only shared cache and transactional reply ownership through
  the existing SDK; resolve/stat/parent never connect or start enumeration.
- [x] Implement Windows Unicode WNet enumeration and exact UNC share handoff;
  test buffer validation, resource cleanup and manual-server parsing separately.
- [x] Implement Linux GIO enumeration with cancellable operations and exact
  target-uri/native-path distinction; absent GVfs returns Unavailable, not empty.
- [x] Verify independent exported project builds/tests with no application source.

### 3. Host and panel integration

Files: host plugin discovery/transport facade, `midnight-core` network navigation
types, `midnight-engine` network controller, `midnight-ui` network presentation.

- [x] Add integration tests first: plugin absent, late refresh after navigation,
  blocked one-panel discovery, explicit handoff, parent/back and operation gating.
- [x] Reuse the audited generic C transport; do not clone native FFI code.
- [x] Add Network drive-menu entry for an available plugin (retain the existing
  Network classification of already-open native shares), show device/share
  rows with system/fallback icons, preserve full location and back context.
- [x] Add a Network button after Refresh in the shared toolbar. Use the system
  icon with the existing fallback, target the active panel, and disable the
  action when no network filesystem plugin is available.
- [x] Support manual server/UNC entry independently of cached discovery. Do not
  enable native mutation, content, checksum or size operations on virtual nodes.
- [x] Verify real user input routes (Enter/double click/refresh/back/cancel).

### 4. Packaging, qualification and documentation

- [x] Add separate Windows/Linux package builds and host package validation tests.
- [x] Audit pinned external/runtime library licenses and update inventory/notices.
- [x] Test protected installed runtime with controlled endpoints or injected OS
  adapters; live LAN smoke is opt-in and must not expose endpoint names in logs.
- [x] Update design, developer API docs, requirements and implementation backlog
  with exact platform capability limits. Keep macOS explicitly pending.
- [x] Independent review, focused tests, Windows debug build and startup smoke.
- [ ] Qualify controlled live SMB/FUSE endpoints; fixture tests are not this gate.
- [ ] Commit/push only coherent reviewed changes, excluding other in-progress work.

Evidence and current limitations:
[network qualification](../../verification/2026-11-09-network-neighborhood.md).
