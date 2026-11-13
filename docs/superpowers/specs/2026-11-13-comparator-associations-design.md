# File comparator settings and associations design

Design only, 13 November 2026. Requested canonical baseline: `4f27e16c6ad233b1294d01d446ae560d8b9a3a9a`.
No implementation, test execution or native qualification is claimed. The four unqualified `performance_probe` example changes remain outside this scope.

[Implementation plan](../plans/2026-11-13-comparator-associations.md) · [UI design and wireframes](2026-11-13-comparator-associations-ui.md).

## Decision and current behavior

Keep the existing built-in text and image comparators. Add ordered filename associations selecting Automatic, Built-in text, Built-in images or an external executable. No dynamic comparator API, plugin installation page, directory-comparison change or archive extraction is introduced.
The current comparison implementation is built into `midnight-compare-engine` and `midnight-compare-ui`; image decoding uses separately installed image-viewer workers. Those decoders are not comparator plugins.
`Ctrl+Shift+C`, F10 Compare files and the toolbar continue through the same existing action. Empty settings preserve current behavior, including manual A/B choice when the panel capture is incomplete.
Alternatives considered: an external-only switch cannot express per-type assignments; a dynamic comparator ABI adds packaging/lifetime/security work without serving this request. Ordered rules plus existing built-ins are the selected minimal design.

Source anchors (all FM paths below are relative to `D:/experiments/dev/FM`):

- `crates/midnight-compare-engine/src/worker.rs:108`: native captures, decoder discovery and current Automatic routing; `src/text.rs:10`: existing text budgets.
- `crates/midnight-core/src/file_compare.rs:6`, `crates/midnight-ui/src/presenter/file_compare.rs:9`, `crates/midnight-compare-engine/src/file_choice.rs:90`: immutable pair capture/resolution.
- `crates/midnight-explorer/src/comparator.rs:7`, `crates/midnight-compare-ui/src/lib.rs:111`: shared composition and comparator window.
- `crates/midnight-core/src/editor.rs:8`, `crates/midnight-platform-common/src/editor_rules.rs:13`: ordered bounded configuration and worker-side matching.
- `crates/midnight-platform-common/src/settings.rs:43`: actual current schema 31; `SECURITY.md` still describes an older schema and needs a focused documentation update.

## Frozen configuration and argument model

Add a separate Options-owned `UserPreferences.file_comparators: FileComparatorPreferences`; do not put routing into `ViewerPreferences.comparison_font`.
Define the following in new core `file_comparators.rs`, exported by core `lib.rs`:

```rust
struct FileComparatorPreferences { rules: Vec<FileComparatorRule> }
struct FileComparatorRule {
    enabled: bool, mode: FilterMode, expression: String, case_sensitive: bool,
    target: FileComparatorTarget,
}
enum FileComparatorTarget {
    Automatic, BuiltInText, BuiltInImages, External(FileComparatorProgram),
}
struct FileComparatorProgram {
    executable: PathBuf, arguments: Vec<String>, working_directory: Option<PathBuf>,
}
```

Empty rules are the default; unmatched pairs always use Automatic. An explicit matching Automatic rule stops further rule evaluation, permitting exclusions before a broad external rule.
Use the editor bounds: at most 64 rules, 64 argv items and 32 KiB total template-argument UTF-8 bytes per external target; filename expression at most 4,096 UTF-8 bytes. Use checked sums. Validate disabled rules too.
Modes are Masks or Regex; reject metadata Expression mode. Reuse the existing bounded filename compiler and its mask/exclusion/case semantics, without changing panel filters or editor behavior.
Executable and optional working directory are absolute native `PathBuf` values without NUL. No executable discovery through PATH, shell expansion or environment expansion. Missing executables may be saved; launch validates availability.
Each template string represents exactly one argv item. Require complete items `{left}` and `{right}` at least once each; repeated placeholders are allowed. Default arguments are `["{left}", "{right}"]`.
Replace only exact whole-item placeholders with the original path's `OsString`. Every other item is literal, including `--left={left}`, quotes, whitespace and empty strings. Do not join, split, quote, escape or stringify native paths.
The expanded plan uses existing `UserCommandProcessInvocation::Program` with `Vec<OsString>` and its existing 8 MiB/native OS argument validation. The 32 KiB template bound is not a new input-path length limit; OS refusal is reported without truncation.
Default cwd is the configured executable's parent; an explicitly configured absolute cwd is supported. No input-directory inference or cwd templates in this increment.

## Pair matching and routing

Retain current pair resolution unchanged: two active-panel selections retain visible order; otherwise A/B retain physical left/right roles, explicit opposite selection wins, and an implicit counterpart is a unique exact basename from the captured visible native listing. Never substitute an unrelated cursor.
Resolve both inputs before association selection. The first enabled rule whose ONE pattern matches BOTH basenames wins; names need not be equal and extensions may differ.
Examples: `*.c;*.cpp` matches `before.c` plus `after.cpp`; `*.txt` matches two differently named text files; separate `*.txt` and `*.png` rules do not independently choose conflicting comparators for a mixed pair. A `*` external rule can intentionally cover mixed types.
Apply matching to losslessly representable basenames only. For a non-Unicode basename, custom rules do not match and Automatic retains the raw native paths. Do not use display text or the panel filter's permissive non-Unicode fallback to authorize an external rule (`filter.rs:54`, `editor_rules.rs:25`).
Freeze preferences per new comparison session with `FileCompareLaunch.comparators: Arc<FileComparatorPreferences>`. Manual A/B edits rerun selection using that same snapshot; later settings changes affect newly opened sessions.
The authoritative backend also owns an immutable compiled-rules snapshot for that exact accepted configuration. Share it across unchanged launches and manual retries; the launch/session must retain its matching compiled snapshot, never combine raw rules from one revision with matchers from another. The opaque handle/correlation plumbing is an implementation-owner choice within the reserved core/engine files.
Compile and validate replacements off UI, only when comparator configuration changes. Publish the replacement coherently with successful load/save acceptance; invalid compilation or failed persistence leaves the previous accepted cache usable. Unrelated preference edits and repeated launches do not recompile. An open Options draft is not the accepted launch policy.
Use one serialized compiler operation, no compiled-revision history map and no per-keystroke compilation. Retain at most one current set, two historical sets pinned by the existing two comparison sessions, and one in-progress replacement: at most four distinct sets. External process tracking must not retain a compiled set after its comparison session ends.
Add a comparator-only aggregate matcher budget of 16 MiB per set, covering the combined program/DFA ceilings for all retained include/exclude regexes. Distribute effective builder limits across those expressions, capped by the existing 2 MiB per-engine limits; reject over-budget candidates explicitly. Existing panel/editor compiler defaults remain unchanged. Four sets imply at most 64 MiB of these configured engine budgets, plus bounded configuration/metadata and one compiler's transient work; this is not a whole-heap/RSS claim and allocation evidence remains required.
Perform selection and capture work on the comparator worker. External routing occurs before image-plugin discovery, content reads or decoding; capture/validate both native regular files through the existing injected native authority, with no direct-opener fallback.
Automatic preserves the present algorithm: two enabled registered non-document image providers select Images; no providers on either side selects strict text decoding; one image/one other or any selected document provider produces the current error. Missing image discovery still does not disable text.
BuiltInText bypasses image discovery and uses the existing strict text reader/limits. BuiltInImages requires enabled non-document image providers on BOTH sides. Forced-target failures never silently choose another target.
Preserve viewer-disabled-provider policy for built-in image/Automatic routing. External is an explicit independent program choice, not permission to load a disabled decoder.

## External launch, authority and ownership

Reuse `UserCommandService::launch_user_program` and platform validation; do not route through editor `{file}`, Custom Command selection expansion, a shell invocation or an association opener.
Inject a narrow `ExternalCompareLauncher` callback into compare-engine Service; explorer implements it over its existing bounded engine command sender. A Service without that callback reports external launch unavailable and retains the chooser.
Add comparator-specific `Command::LaunchExternalComparison { process, validation, cancel, reply }`. `process` is Program-only; `reply` is a one-slot synchronous result channel, consumed only on the comparator worker with cancellation-aware timed receives, never on UI.
The opaque core `ExternalCompareValidation` trait supplies `revalidate(&Cancellation) -> Result<(), FsError>`; its compare-engine implementation retains the two captures and original paths. Construct the process argv from those original paths, not `CapturedFile::path` (plugin-backed captures may store an empty path).
Engine's new private `desktop_external_compare.rs` reserves existing desktop launch capacity and the shared eight terminal/editor/Custom Command process slots plus cleanup ticket before native execution. Busy admission fails promptly; no engine-loop wait, new queue, pool or larger quota.
On the admitted native worker: check cancellation, revalidate BOTH retained captures, drop that validation owner on the worker, check cancellation again and call the existing platform Program launcher. These checks are sequential, not an atomic pair snapshot.
Immediately place every successfully returned child into `ProcessTicket::own`, retaining quota and TaskLifetime before reply delivery. Existing tracker/reaper ownership survives reply-channel loss, session/window close and runtime shutdown.
The result distinguishes NotStarted (validation/admission/spawn failed), Started (pid, optional tracking-delivery warning) and Unknown (transport lost before outcome known). A post-spawn delivery/tracking failure cannot become NotStarted; never automatically retry an ambiguous outcome.
Successful handoff emits compare-engine `Output::ExternalStarted` and closes the temporary comparator/chooser window. The shared tracker continues owning the child. Confirmed NotStarted returns `ChooseFiles` with both original drafts intact; Unknown retains drafts with an explicit uncertain-outcome message and no automatic retry.
Cancellation prevents pending work where possible; closing the chooser never kills or retracts an already started external program. Do not wait for the GUI tool to exit on a comparator/UI worker. Reuse cleanup-only tracking (`completion: None`); exit code 1 is not generically an error or a comparison result.
Windows keeps existing absolute `.exe`/`.com` validation, rejects `.bat`/`.cmd`, uses native argv and null streams. Linux keeps executable regular-file/cwd/native argument validation and null streams. No new platform adapter is required.
The configured executable is trusted application code, with its own argument parser and permissions. The app does not construct shell source or promise to sandbox a user-configured interpreter/tool. Native executable replacement and ordinary pathname races remain outside this guarantee.
External tools receive ORIGINAL mutable paths and may edit/delete inputs or observe later replacements. Revalidation establishes only the checked native revisions at the handoff boundary; it provides no write prevention, immutable snapshot, simultaneous-pair or TOCTOU guarantee. State this in the external-target settings help; built-ins remain read-only.
Archive/virtual selections continue to yield unspecified chooser sides. Do not pass archive display paths, extract temporary inputs, materialize providers, copy back edits or add cleanup/export lifecycles. Users may explicitly select ordinary native files in the chooser.

## Persistence and Options behavior

Use schema 32 for nonempty comparator rules, with a strict tagged DTO in new `settings/file_comparators.rs`. Earlier supported versions default to empty rules and reject the new field; v32 requires it and rejects null/unknown targets/fields/invalid native encodings. Future unsupported versions still fail atomically.
Preserve the existing minimum-required-version encoder. Cascade the comparator feature flag through older mandatory groups: a comparator-only v32 save must include `disk_io_concurrency: Automatic` and all other fields required by v31/v30/etc. Empty/default routing can retain the previous minimum version.
Reuse existing native path encoding and the 4 MiB aggregate settings budget. Compile/validate every changed rule set, including disabled rules, through the shared backend cache before acceptance/publication; unchanged accepted rules reuse its validation result during serialization. Save errors leave the prior persisted settings intact.
Add an Options File Comparators page, topic ID 10, preserving reserved ID 2 and existing IDs. Use ordered rows: Enabled, file types/pattern, comparator. Add/edit/remove/reorder rules; show an uneditable final “Other file types — Automatic” row.
Follow UI's read-only Built-in types section even when there are no overrides: effective enabled non-document image extensions → Automatic/Built-in images; other regular files → Automatic/Built-in text, subject to existing encoding/content checks. Obtain coverage from existing backend decoder metadata and current-session enablement; report unavailable/unknown coverage honestly, never hardcode manifest examples as installed support. This section does not create rules, dirty settings or promise mixed pairs/image decodability. See the [UI design and wireframes](2026-11-13-comparator-associations-ui.md).
For External, show program Browse, argv rows with `{left}`/`{right}` insertion and optional cwd Browse. Reuse ArgumentEditor after parameterizing its current editor-only insertion controls; do not introduce shell-style parameter parsing.
Mirror editor raw-path/argument drafts: unchanged lossy display text retains original native identity, picker results retain raw paths, and actual edits intentionally supply new Unicode values. Preserve argument row whitespace, empty values and order.
Join existing Options baseline, restore_fields, persistable filtering, draft discard/reset/all-page acceptance, dirty tracking and initial-load EditedFields merge. Cancel and unrelated autosaves cannot leak comparator drafts; Reset stages empty rules until successful OK/save. Keep independent comparison-font changes outside this rollback field.
Do not present failed persistence as applied. In-flight sessions retain their captured routing snapshot; detailed field-error presentation can follow the existing editor save-feedback mechanism without a new validation service.

## Reference behavior and acceptance boundary

Read-only Salamander reference: `D:/experiments/salamander-main/src/cfgdlg.h:120` and `src/dialogs5.cpp:2348` define ordered editor masks, executable, arguments, cwd and a staged settings copy; `src/plugins/filecomp/filecomp.cpp:429` registers its comparison plugin action and `:173` stores that plugin's own display/diff options. `src/plugins/filecomp/dialogs.cpp:55` supplies its two-file chooser.
That repository's comparator is a menu plugin; this application's comparators are built in. Adopt only the useful association/editing concepts, not C++/assets, ANSI conversions, MAX_PATH or shell-style command-line expansion. No evidence found for a general Salamander comparator-association table; do not claim one exists.
No new third-party package, dynamic comparator dependency or paid component is proposed. An internal compare-engine -> platform-common edge can reuse the existing filename compiler/cache; verify its resolved feature graph under dependency policy rather than adding a second matcher. Performance constraints were reconciled with the dated private Perf review under `_build/verification/perf-comparator-settings-20261113/`; its request for zero INTERNAL capture does not prohibit the two metadata/authority captures required for a safe external handoff, and it does not authorize changed session-snapshot semantics or a preview feature.
Meaningful acceptance covers BOTH-basename priority, raw argv/path preservation, strict v31/v32 migration and mandatory-field cascade, Options isolation/late load, forced routing/no fallback, failed B capture preventing launch, cancellation/ambiguous launch, tracker survival and unchanged built-in behavior. Exact tests and ownership are in the [implementation plan](../plans/2026-11-13-comparator-associations.md).
No material product decision blocks the plan. UI layout details, DTO names and local helper organization belong to implementation owners; expanding archive support, a plugin ABI or external sandboxing is a separate decision.
