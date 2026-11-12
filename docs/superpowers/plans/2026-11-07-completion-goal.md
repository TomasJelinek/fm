# Midnight Explorer Completion Goal

> Execute with `superpowers:executing-plans`; independent investigations and
> implementations may run in parallel as explicitly requested by the user.
> The latest user instruction authorizes implementing the listed backlog and
> missing designed features; do not request another general design approval.

**Goal:** Implement and verify the remaining assigned functionality, audit the
design for further omissions, and maintain accurate feature/API documentation.

**Architecture:** Keep the existing Rust workspace, Slint UI and Tokio/backend
message boundaries. Extend tested services instead of creating parallel mutation,
copy, selection or plugin-loading implementations. Independent plugins consume
only versioned public SDKs and retain their own dependencies/tests.

**Tech stack:** Rust 1.99, Slint 1.18.1, Tokio, native OS adapters and the existing
public native/process plugin interfaces. New dependencies require a free-use,
distribution-compatible license review and refreshed inventory/notices.

**Spec:** [Main design](../specs/2026-11-04-dual-pane-file-manager-design.md),
its linked extensions and [accepted stages](2026-11-05-follow-up-stages.md).
The evidence matrix is [IMPLEMENTATION_BACKLOG.md](../../IMPLEMENTATION_BACKLOG.md).

## Global constraints

- Preserve Unicode and raw platform/provider identities; no application MAX_PATH.
- All non-UI I/O, parsing, hashing, enumeration and expensive cleanup stay on workers.
- Keep Windows/Linux/macOS bodies and tests in separate files/crates.
- Rustfmt max_width = 100; handwritten source at most 100,000 bytes.
- Copy uses SHA-256 over source bytes and independent destination readback.
- Ordinary Move removes each exact source immediately after that file's copy,
  independent SHA-256 destination readback, publication and required metadata
  succeed. Do not wait for the whole batch and do not use source recovery folders
  or a recovery opt-in. Later error/cancellation retains earlier completed moves;
  changed identities and failed verification never authorize deletion.
- More than one copy stream requires positively classified local SSD at both ends;
  HDD/network/unknown remains serial with shared device/mutation budgets.
- Never operate on navigation `..`; selections retain captured identities/revisions.
- Native plugins require protected code paths; runtime registration does not waive
  package/ancestor checks. No startup copying from untrusted plugin locations.
- Preserve user preferences during QA. Use in-memory smoke settings or isolated
  probes; never run the normal app with test --left/--right paths.
- Build/test output under `_build`; toolchains under `.tools`; private runtime
  installations and protected native QA fixtures are explicitly separate.
- Work in the existing user-selected checkout/branch to preserve the launch path
  and avoid duplicate build caches. Concurrent agents own disjoint files; root
  integrates shared contracts/UI. Serialize Cargo/build changes when necessary.

## Review focus

1. Concurrent namespace/source changes between consent, verification and publish:
   preserve unexpected data and report the actual provider guarantee.
2. Cancellation and full queues: no permit release while native I/O is still active,
   no source deletion or phantom queued job after rejected admission.
3. Raw Unicode paths, long names, aliases and links: preserve identities and never
   infer authority from display strings or follow links implicitly.
4. Multiple views/tabs and stale search/selection results: asynchronous completion
   cannot retarget another panel, overwrite edited forms or resurrect old results.
5. Plugin removal, hostile archives, interrupted processes and platform limitations:
   capabilities remain truthful, resource use bounded, failures explicit.

## Execution ledger and deliverables

Baseline: `30e89ecb920c8ce144313dbafb5932386a32911b`, clean checkout.
Existing Windows/Linux backend suites, actual D: release startup and all five
viewer packages were verified at that baseline; repeat checks when relevant changes
or failures justify it, rather than rebuilding unchanged plugins for every task.

- [x] Audit design requirements against code/tests; continue maintaining the evidence matrix.
- [ ] Resolve the reported keyboard-settings search crash with causal evidence and
  native-input regression; preserve an honest investigation state if unreproduced.
- [ ] Complete copy: directory merge, explicit overwrite contracts, links/metadata,
  per-item/remaining-conflict choices and a shared backend queue.
- [ ] Finish ordinary per-file Move/Cut/F6 with source identity revalidation;
  remove the superseded source-recovery workflow.
- [ ] Implement bounded modeless Find with name/content queries and safe result actions.
- [ ] Implement rectangular selection across Details/Brief/Thumbnails and providers.
- [ ] Implement Custom Commands settings, safe expansion/launch and conditional toolbar.
  Include the user's subsequent settings-list drag ordering request, preserving
  command IDs, shortcuts, unsaved drafts and one save per completed valid drop.
- [ ] Implement Ctrl+F8 checksum jobs, result view and verified manifest publication.
- [ ] Implement inline directory/archive/provider expansion with lazy bounded enumeration.
- [ ] Implement network-neighborhood providers and per-platform native filesystem plugins.
- [ ] Implement typed metadata expression filters alongside existing masks/regex.
- [ ] Implement SSD-to-SSD concurrency and shared device scheduling; measure fairness/UI latency.
- [ ] Extend archives and media to the accepted audited scope, preserving sandbox/resource budgets.
- [ ] Implement and natively qualify macOS; qualify required Linux distributions and release packaging.
- [x] Fix shortcut updates on OneDrive cloud-placeholder Desktop without weakening
  executable/plugin trust or following symlink/junction destinations.
- [ ] Close additional audited gaps, update documentation/hotkeys/APIs/licenses,
  perform independent review, rebuild/test releases and commit/push under TomasJelinek.

Each substantial deliverable gets a focused implementation plan with exact files,
interfaces and regression cases before production changes. Tests cover real owned
fixtures, errors, races, cancellation and native paths. A documented design or a
compile-only check is not implementation/qualification. Keep this goal active until
the assigned work is actually complete; report real external validation dependencies.

## Initial parallel ownership

Reviewed delivery: `41a08b1c38062f29a2428b25bce2aead98535baa` contains the
redirected/cloud Desktop shortcut fix, native tests and verification record.
Fresh Windows regression passed before commit; push to `codex/viewer-plugins`
was verified under GitHub account TomasJelinek. Other goal changes remain separate.

- `goal_design_audit`: requirement/code matrix only, `docs/IMPLEMENTATION_BACKLOG.md`.
- `goal_keyboard_crash`: keyboard-specific diagnostic/presenter/tests; shared UI
  changes require coordination.
- `goal_transfer_contract`: native overwrite/merge/move feasibility and exact contract
  proposal, initially read-only with report `_build/goal/transfers/contract-review.md`.
- Root: integration, implementation plans, desktop shortcut publishing, subsequent
  copy workflow and review/validation. No concurrent edits to shared contracts.

## Initial rulings

- Existing accepted designs and the explicit implement-all instruction authorize
  implementation. Skill approval prompts do not restart an already approved workflow.
- `RENAME_EXCHANGE` is not a conditional compare-and-swap against a consented inode.
  Assess the main design's explicitly selectable recoverable replacement contract;
  never silently label path replacement as conditional overwrite.
- The OneDrive Desktop has cloud tag `0x9000601a`, not a name-surrogate junction.
  Shortcut-only handling can distinguish cloud placeholders; plugin trust remains
  deliberately stricter. Native namespace stability still requires verification.
