# Plugin options Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Present readable viewer names and real installed filesystem packages,
with more visible viewer rows, heading reorder actions and bottom Refresh.

**Architecture:** Existing bounded backend discovery retains separate viewer and
filesystem results; UI handles only presentation and preferences. Discovery reads
trusted manifests/artifacts without loading code. Typed manifest ownership avoids
false HTML rows while preserving malformed/mixed-package diagnostics.

**Tech Stack:** Existing Rust/Tokio services, filesystem/viewer hosts and Slint;
no added dependencies.

**Spec:** `_build/qa/plugin-options/design.md`, approved by parent on 2026-11-10.

## Global Constraints

- Keep plugin IDs, wire/API identities, routing and persisted settings unchanged.
- No new dependencies, commits, broad cleanup or uncoordinated Cargo builds.
- Preserve unrelated dirty source and capture scoped before-images under `_build`.
- Retain actual malformed, duplicate, untrusted and ambiguous-package diagnostics.
- Filesystem status means installed manifest/artifact validation, not code loaded.
- Platform implementations remain separate; all discovery/metadata work is backend.
- Root owns options-dialog.slint; request only available-height input wiring.

## Review Focus

- Pure web worker must not become a false native diagnostic; mixed bundles fail.
- Older installed manifests remain readable and have useful names without descriptions as titles.
- A filesystem discovery error must not erase valid viewer results or invent runtime status.
- Refresh preserves extension drafts and selection by stable identity.
- Narrow/high-scale settings retain accessible controls and keyboard list navigation.

### Task 1: Correct viewer transport classification and native metadata

**Files:** `crates/midnight-plugin-host/src/{discovery.rs,tests.rs}`;
`crates/midnight-image-viewer-host/src/providers.rs` and its discovery tests;
shipped native viewer plugin.toml and image/pdf/web JSON manifests.

**Interfaces:** PluginInfo adds optional friendly name metadata distinct from
description; native Manifest accepts optional validated name. Native discovery
delegates single image/web manifests and rejects mixed transports. Image discovery
rejects a web manifest alongside image/native manifests.

- [x] Write failing pure-web and all mixed-transport discovery tests.
- [x] Run RED using the coordinated Linux Cargo lease and preserve logs.
- [x] Implement delegation/ambiguity handling and optional-name validation.
- [x] Add clean shipped names and run discovery/name compatibility tests GREEN.

### Task 2: Publish real backend plugin catalogs

**Files:** `crates/midnight-core/src/{viewer.rs,protocol.rs}`;
`crates/midnight-engine/src/viewer_plugins.rs`, focused new filesystem catalog
module/tests; `crates/midnight-native-filesystem-host/src/{discovery.rs,lib.rs}`;
`crates/midnight-filesystem-host/src/discovery.rs` and focused tests.

**Interfaces:** FilesystemPluginKind Native/Archive/Network and
FilesystemPluginInfo {id,name,version,kind,location,status}; new
Event::FilesystemPluginsListed(Result<Vec<FilesystemPluginInfo>,String>).
Existing bounded viewer refresh worker discovers both catalogs and publishes
their results independently. Native filesystem Package carries explicit kind;
filesystem manifests accept optional bounded friendly name metadata. Engine
builds clean legacy viewer names and groups diagnostics without losing reasons.

- [x] Add regressions for legacy names, explicit names and real filesystem kinds.
- [x] Observe RED; implement metadata/catalog publication without launching code.
- [x] Verify malformed manifests, missing artifacts, duplicates, trust rejection
  and independent catalog failure behavior with actual filesystem fixtures.
- [x] Run focused host/engine suites and relevant strict Clippy when coordinated.

### Task 3: Present separate viewer and filesystem subviews

**Files:** `crates/midnight-ui/src/plugin_options.rs`;
`src/presenter/viewer_plugins.rs`; narrow event arm in `src/presenter/desktop.rs`;
`ui/plugin-options.slint`; existing/focused plugin options integration tests.

**Interfaces:** Presenter retains independent catalogs/pending/errors; Refresh
requests both through existing backend command. PluginOptions gets
available-height; root supplies content.visible-height in options-dialog.slint.
Reuse ListHeader with show-add/show-remove false and actions2/3 mapped to viewer
priority up/down. Filesystems is read-only with real package details.

- [x] Add native UI/presenter regressions for taller list, heading actions,
  bottom Refresh and read-only Filesystems display.
- [x] Implement Viewers/Filesystems subviews with 8–12 viewer rows.
- [x] Preserve current viewer enable/priority/extension/restart behavior and
  keyboard selection/reveal. Show profile descriptions in selected details.
- [x] Run plugin options native tests and existing association/persistence tests;
  shared settings layout/scaling gates stay with parent. Relevant host/core strict
  Clippy and the parent's full rustfmt check passed.
- [x] Save verification report and screenshots under `_build/qa/plugin-options`;
  report exact remaining integration/deployment limits to parent. No commits.

## Qualification checkpoint

Linux host discovery RED proved six defects/metadata gaps; all four host suites
then passed (118 tests, seven existing opt-ins ignored). Core names passed 2/2;
trusted filesystem catalog fixtures passed 4/4; strict core/four-host Clippy passed.
The actual installed Windows catalog reproduced its technical basic-viewer title
before the naming change. UI layout RED rebuild was explicitly waived by parent
under the developer instruction for reversible low-impact presentation changes;
the new native layout/readonly behavior regressions run in the combined GREEN gate.
Parent Windows UI library evidence includes all junction activation cases passing;
Windows plugin UI 5/5 and both actual installed-catalog opt-ins subsequently passed.
Both refreshed plugin BMPs were reviewed. Full Linux core 51/51 and engine 661 passed
after retention cleanup (seven existing opt-ins ignored). The Windows debug
application was subsequently built, deployed, registered and startup-tested with
empty stderr. Build and private runtime copies share SHA-256
`E71ED919E0B0EAE14DEC5B3062213F9DA6978F287206AC849CB20FA0BCAC6E11`.
See the [final artifact and gate record](../../verification/2026-11-10-settings-polish.md)
for exact paths, tests and snapshot scope. This records a debug deployment;
published release and the broader implementation goal are not claimed complete.
