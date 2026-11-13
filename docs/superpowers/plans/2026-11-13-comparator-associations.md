# File Comparator Associations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans when Root authorizes implementation. Steps use checkbox (`- [ ]`) syntax. This design-phase handoff authorizes no product edits, builds or execution.

**Goal:** Configure per-file-type built-in/external comparison while preserving existing actions and native pair authority.

**Architecture:** Ordered BOTH-basename rules route on the comparator worker using one accepted immutable compiled snapshot. External Program launch reuses existing desktop admission and process tracking; UI owns configuration/presentation. Existing text/image comparators and native-only pair resolution remain.

**Tech Stack:** Existing Rust, Slint, strict preferences codec, platform-common filename compiler/cache and native Program adapters; no new third-party package.

**Spec:** [Comparator settings and associations design](../specs/2026-11-13-comparator-associations-design.md). [UI design and wireframes](../specs/2026-11-13-comparator-associations-ui.md). All implementation below is **PLANNED**, not completed or tested.

## Global Constraints

- Root allocates exact files and source/target/native-input leases before implementation; Root owns Git/integration. Current Custom Commands work retains priority. Serialize shared Core, Options, main.slint, lib.rs and composition hunks.
- Preserve the four unqualified `performance_probe` example changes. Do not mix Source recovery, directory comparison, decoder changes, archive materialization or a comparator plugin ABI into this feature.
- Keep platform implementations separate and non-UI work in backend services. Build/test/generated output belongs under `_build/`; toolchain caches use `.tools/`. Handwritten files stay below 100,000 UTF-8 bytes.
- Third-party libraries must be usable for intended development, application use and distribution without mandatory fees, subscriptions or royalties. No new third-party package is proposed; verify the internal dependency edge's resolved features and follow the dependency policy if its graph changes.
- At most 64 rules, 64 argv items and 32 KiB template-argument UTF-8 per external target; filename expression at most 4,096 UTF-8 bytes. Validate disabled rules. Only Masks/Regex, not metadata Expression. Native paths are not truncated by these configuration budgets.
- Require complete `{left}` and `{right}` argv items; preserve other items literally. Existing expanded native-argument/OS validation and 8 MiB limit apply. The settings aggregate budget remains 4 MiB; schema 32 must cascade all older mandatory fields, including Automatic disk concurrency.
- One serialized off-UI compiler; at most current + two session-pinned historical + one replacement sets. Aggregate matcher engine budgets are 16 MiB/set and at most 64 MiB over four sets, plus bounded metadata and one compiler's transient work; this is not a whole-heap/RSS claim. Panel/editor compiler defaults remain unchanged.
- Use the existing two comparator sessions and eight shared launched-process slots, with real worker/child lifetime and cancellation. No new queue, pool, quota, shell path, preview subsystem or per-keystroke regex compilation.
- Author meaningful tests first in each owned packet. Build runs targeted boundaries under coordinated leases; full suites wait until coherent implementation/integration is complete. No commands below were run for this plan.

## Review Focus

- Lossy/non-Unicode basenames must not select an external rule; original native paths and argument boundaries survive (Tasks 1/2/5).
- Comparator-only schema-32 saves must include Automatic disk concurrency and older mandatory groups (Task 2).
- B capture/revalidation failure after A succeeds causes zero spawn and worker-side retirement of A (Tasks 4/5).
- A child created across cancellation, receiver loss or shutdown stays tracked; Started/Unknown is not retryable NotStarted (Tasks 4/5).
- Unaccepted Options drafts, stale policy revisions and invalid/failed cache replacements cannot replace accepted settings; cache owners stay bounded (Tasks 2/3/5).

---

### Task 1: Core configuration and handoff contracts

**Interfaces:**
- Consumes existing `FileCompareSource`, `Cancellation` and Program invocation types.
- Produces the spec's configuration records, `FileCompareLaunch.comparators`, `ExternalCompareValidation::revalidate(&Cancellation) -> Result<(), FsError>` and typed NotStarted/Started/Unknown handoff.
- Pins `FileComparatorProgram::arguments_for(&[PathBuf; 2]) -> Result<Vec<OsString>, &'static str>` before dependent owner packets.

**Owner:** Backend. **Depends on:** this design. **Produces:** the exact configuration types in [the design](../specs/2026-11-13-comparator-associations-design.md), `FileCompareLaunch.comparators`, `ExternalCompareValidation`, typed launch request/result and `Output` consumer assumptions for Tasks 2–5.
**Modify:** `crates/midnight-core/src/{lib.rs,file_compare.rs,protocol.rs}`. **Create:** `crates/midnight-core/src/file_comparators.rs`, `crates/midnight-core/tests/file_comparators.rs`.
- [ ] Pin validation and `FileComparatorProgram::arguments_for(&[PathBuf; 2]) -> Result<Vec<OsString>, &'static str>`: both whole placeholders required, repeated placeholders, embedded placeholders literal, empty arguments, NUL/relative executable/cwd refusal and checked bounds.
- [ ] Define Program-only external handoff, opaque final revalidation and NotStarted/Started/Unknown outcomes; no dynamic ABI or shell variant. Use `Cancellation` and a one-slot reply sender.
- [ ] Default preferences are empty rules/Automatic. Document cloned launch snapshots and native argv identity; avoid logging full commands or paths in new diagnostics.

### Task 2: Matching and strict preferences persistence

**Interfaces:**
- Consumes Task 1 values and existing bounded filename compiler/native path codec.
- Produces one immutable accepted compiled ruleset correlated with its exact preferences revision, plus strict schema-32 persistence and one ordered pair selection.
- Freeze the local opaque compiled-handle/correlation plumbing at this handoff before Tasks 3/5; Architect explicitly leaves its local name/organization to the implementation owner.

**Owner:** Backend. **Depends on:** Task 1. **Produces:** accepted immutable `comparator_rules` compiled cache/selection plus v32 roundtrip/migration for Task 3/5.
**Modify:** `crates/midnight-core/src/preferences.rs`; `crates/midnight-core/src/protocol.rs` (serialized after Task 1 for cache publication plumbing); `crates/midnight-engine/src/preferences.rs`; `crates/midnight-platform-common/src/{lib.rs,settings.rs,filter.rs}`; `crates/midnight-platform-common/tests/editor_preferences.rs` only if shared migration fixtures need adjustment.
**Create:** `crates/midnight-platform-common/src/comparator_rules.rs`, `crates/midnight-platform-common/src/settings/file_comparators.rs`, `crates/midnight-platform-common/tests/file_comparators.rs`.
- [ ] Test first enabled BOTH-basename match, mixed `.c`/`.cpp`, different basenames, exclusion/case/Regex semantics, explicit Automatic stop and unmatched fallback.
- [ ] Require both basenames to be losslessly Unicode before custom matching; test raw non-Unicode names use Automatic, without changing CompiledFilter's existing visibility behavior.
- [ ] Test old supported settings default empty, pre-v32 field refusal, v32 missing/null/unknown fields refusal, disabled invalid regex, raw native path roundtrip and aggregate budget.
- [ ] Test comparator-only v32 encoding emits every older mandatory field, especially Automatic disk concurrency and v30 editor/history fields. Preserve minimum-version selection and transactional writes.
- [ ] Compile once for an accepted changed configuration, off UI; repeated launches/unrelated saves reuse it, and invalid/failed replacements preserve it. Pin the exact revision into sessions, enforce one compiler/current + two session-pinned historical + one replacement sets, and the 16 MiB per-set aggregate matcher budget with comparator-only effective compiler limits. Preserve panel/editor defaults; no per-keystroke compile or revision-history map.

### Task 3: Settings UI and captured launch preferences

**Interfaces:**
- Consumes Tasks 1/2's frozen configuration, save feedback, matching accepted-policy identity and backend effective decoder metadata.
- Produces transactional Options drafts and new-session launch snapshots; no UI matcher, content capture or process launch.

**Owner:** UI. **Depends on:** Task 1's frozen types; Task 2's save behavior. Can proceed alongside Task 4 after core names freeze.
**Modify:** `crates/midnight-ui/ui/{options-navigation.slint,options-dialog.slint,main.slint,argument-editor.slint}`; `crates/midnight-ui/src/lib.rs`; `crates/midnight-ui/src/presenter/{preferences.rs,settings_picker.rs,file_compare.rs}`; `crates/midnight-ui/src/presenter/preferences/options.rs`.
**Create:** `crates/midnight-ui/ui/comparator-options.slint`, `crates/midnight-ui/src/comparator_options_presentation.rs`, `crates/midnight-ui/src/presenter/comparator_draft.rs`, `crates/midnight-ui/tests/comparator_options.rs`.
**Tests to extend:** `crates/midnight-ui/src/presenter/{settings_picker_tests.rs,file_compare_tests.rs}`; `crates/midnight-ui/src/presenter/preferences/options_tests.rs`; `crates/midnight-ui/src/presenter/tests/preferences_load.rs`; `crates/midnight-ui/tests/options_transaction.rs`.
- [ ] Add topic 10 without renumbering existing/reserved IDs. Show ordered mask/target rows and the fixed Automatic fallback; provide External executable/argv/cwd fields and mutable-originals help.
- [ ] Add UI's read-only Built-in types coverage from current effective backend decoder metadata, including unavailable/unknown states and document exclusion. Empty rules remain informative; no static extension manifest, dirty-state change, content reads or discovery on UI.
- [ ] Reuse editor draft/raw-path semantics and ArgumentEditor rows; parameterize insertion controls so existing editor `{file}` behavior is unchanged.
- [ ] Pin Cancel, Reset followed by Cancel/OK, failed OK/retry, off-page invalid draft, unrelated autosave, late initial load, stale picker and raw executable/cwd preservation.
- [ ] Freeze an `Arc<FileComparatorPreferences>` into each launch; test subsequent preference edits cannot retarget an open comparison. Existing shortcut/menu/toolbar callbacks stay unchanged.

### Task 4: Existing desktop Program launch and process ownership

**Interfaces:**
- Consumes Task 1's Program-only `Command::LaunchExternalComparison { process, validation, cancel, reply }` and existing tracker/admission services.
- Produces the bounded NotStarted/Started/Unknown outcome after real native work; `ProcessTicket::own` owns every returned child before reply delivery.

**Owner:** Backend. **Depends on:** Task 1. **Produces:** `Command::LaunchExternalComparison` handling and tracked typed outcome for Task 5.
**Modify:** `crates/midnight-engine/src/{lib.rs,desktop.rs}`. **Create:** `crates/midnight-engine/src/desktop_external_compare.rs`, `crates/midnight-engine/src/desktop_external_compare_tests.rs`.
**Reuse/read-only boundaries:** `user_command_runtime.rs`, `user_command_process_tracker.rs`, Windows/Linux `user_commands.rs` and `editor.rs`. No platform edits are required by the current design.
- [ ] Clone the configured `UserCommandService` before its existing move into the Custom Command controller; use desktop launch capacity, shared eight process slots and pre-reserved cleanup tickets.
- [ ] On the native worker, revalidate/drop the two-input guard, check cancellation and launch Program-only. Immediately wrap the child in `ProcessTicket::own` before any reply/send/cancellation point.
- [ ] Test busy/full admission causes zero spawn, failed/cancelled B validation causes zero spawn, both captures retire off UI, and missing service/executable errors remain NotStarted.
- [ ] Hold the native spawn across cancellation, lose the result receiver and stop the ordinary reaper/runtime: the actual returned child and quota must survive through existing tracker ownership. Started/Unknown never becomes a retryable NotStarted.
- [ ] Keep cleanup-only completion and no process kill. Do not interpret exit code 1 as comparison failure. No new generic process framework, pool, quota or UI wait.

### Task 5: Comparator routing, launcher adapter and window handoff

**Interfaces:**
- Consumes Task 2's accepted compiled snapshot and Task 4 launch handling through `ExternalCompareLauncher`.
- Produces forced/Automatic built-in routing or reliable terminal `Output::ExternalStarted`; the compare session pins one policy and never reconstructs paths from display text.

**Owner:** Viewer exclusively owns compare-engine shared routing and compare-ui; Backend exclusively owns Task 4 platform launch/engine tracking. Root or assigned application integrator owns explorer composition. **Depends on:** Tasks 1, 2, 4; Task 3 supplies accepted launch snapshots. These are proposed reservations; current Custom Commands work retains priority and no comparator product code is authorized by this plan.
**Modify:** `crates/midnight-compare-engine/{Cargo.toml,src/lib.rs,src/worker.rs}`; `crates/midnight-compare-ui/src/lib.rs`; `crates/midnight-explorer/src/comparator.rs`.
**Create:** `crates/midnight-compare-engine/src/external_compare.rs`, `crates/midnight-compare-engine/src/comparator_routing_tests.rs`, `crates/midnight-compare-ui/tests/external_compare.rs`.
**Extend integration tests:** `crates/midnight-compare-engine/src/{file_choice_tests.rs,capture_tests.rs,worker_tests.rs,worker_raw_windows_tests.rs,worker_raw_unix_tests.rs}`; `crates/midnight-explorer/src/native_content_tests.rs`.
- [ ] Add only the internal platform-common dependency needed for its existing matcher. Reconcile Cargo graph/features under dependency policy; no new third-party versions, install or licensing alternative is proposed.
- [ ] Route after pair resolution/capture and before decoder discovery; External performs zero content reads/decodes. BuiltInText skips image discovery; BuiltInImages requires both enabled non-document providers; Automatic preserves the current branch behavior.
- [ ] Implement `ExternalCompareLauncher` over the existing command sender and a single bounded reply. Final guard retains captures plus original paths; never reconstruct paths from display text or plugin-backed CapturedFile.path.
- [ ] Consume the exact accepted cached rule-set handle for both initial and manual choices; pin it through the session, never rematch on UI/recompile per launch, and never lose/duplicate ExternalStarted through latest-output frame/status replacement.
- [ ] Started closes the chooser while child ownership persists; NotStarted returns the A/B chooser with drafts intact; Unknown is explicit and never automatically retried. Preserve existing manual chooser/raw-edited-path behavior.
- [ ] Test forced target failure/no fallback, disabled image providers, mixed images/text, document refusal, decoder discovery failure/text availability, unavailable launcher, archive/virtual unspecified sides, and no launch after either native capture fails.
- [ ] Mechanically update FileCompareLaunch constructors in the listed engine/explorer tests, compare-ui tests `{ui_polish,difference_navigation,keyboard,fonts,file_choice}.rs` and `crates/midnight-compare-ui/examples/native_comparator.rs`; do not alter existing expected behavior.

### Task 6: Coherent qualification and current documentation

**Interfaces:**
- Consumes frozen reviewed owner packets, exact source/dependency identities and coordinated build/native leases.
- Produces scoped execution receipts and current guarantees; planned tests and this documentation are not passing evidence.

**Owner:** Build executes; Perf reviews unnecessary discovery/CPU/I/O and retained owners; Documentation updates current guarantees; Root integrates. **Depends on:** frozen reviewed Tasks 1–5.
**Docs:** `docs/developer/FILE_COMPARISON.md` (feature and settings behavior) and `SECURITY.md`'s focused preferences/external-comparison paragraphs. No new competing Options guide.
- [ ] Build targeted groups: core `--test file_comparators`; platform-common `--test file_comparators`; engine `--lib desktop_external_compare`; compare-engine `--lib comparator_routing` plus existing capture/file-choice/worker suites; UI `--test comparator_options --test options_transaction`; compare-ui `--test external_compare --test file_choice` and existing keyboard/difference/font boundaries; explorer native-content boundary.
- [ ] Reuse the existing native Program-argument harness on Windows/Linux: verify exact two paths and argv items, Unicode/metacharacters/empty literals, long paths, absent executable and permissions. ASCII quote filenames are Unix-only; Windows quote tests use literal argv values/valid filenames.
- [ ] Qualify one installed no-fee GUI comparator on each intended platform only under Root/Build native leases; verify explicit launch, process cleanup and original-file edit semantics. No paid-only or trial dependency is selected by this plan.
- [ ] Check source size, formatting, targeted Clippy, existing Options/editor controls and built-in Auto actions. Compile compatible artifacts once, then run bounded headless groups and serialized desktop input. Broader final checks occur after integration, not between owner tasks.
- [ ] Publish actual commands/results, ignores, source identity, resolved dependency/features and unresolved native behavior; authored tests or this design are not passing evidence. No performance claim follows from avoiding discovery alone.

No unresolved material product choice blocks implementation. Owners may choose local helper/DTO organization inside these boundaries; new files beyond this list, provider materialization, plugin APIs or process-policy changes return to Root for scope allocation.

## Planned targeted commands

Build chooses the coordinated target directory/feature closure and uses the repository Cargo wrapper on Windows. These commands describe future checks, not execution in this assignment:

```text
cargo test -p midnight-core --test file_comparators --locked --offline
cargo test -p midnight-platform-common --test file_comparators --locked --offline
cargo test -p midnight-engine --lib desktop_external_compare --locked --offline
cargo test -p midnight-compare-engine --lib comparator_routing --locked --offline
cargo test -p midnight-ui --test comparator_options --test options_transaction --locked --offline
cargo test -p midnight-compare-ui --test external_compare --test file_choice --locked --offline
```

Extend these with the existing capture/file-choice/worker, Options/editor, compare keyboard/difference/font and explorer native-content boundaries named in each task. Run compatible groups once and serialize real desktop input; retain failed/ignored counts and platform limits. Root reviews each owner deliverable before dependent integration.
