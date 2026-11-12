# Historical branch retirement — 12 November 2026

The user authorized selecting useful historical changes, discarding obsolete
changes and deleting the remaining archive/thumbnail/fix-cargo branches. Backend
and Viewer compared actual patches with baseline
`369b1619a3eff1d9b6e07a1bcb16c4d4b2653bbe`; the Orchestrator accepted their source
comparisons. No code needed recovery from the five archive/thumbnail commits.
This is semantic source evidence, not a claim that those commits are ancestors
of main or that new runtime tests passed.

## Dispositions and retirement results

| Branch / reviewed tip | Useful-content disposition | Result reported by the Orchestrator |
| --- | --- | --- |
| `codex/archive-filesystems` — `99e619ccc8085f071d3454910317010962613dbf` | All three commits are already present or superseded by current archive/provider/worker code; port none. | Exact-tip lease checks preceded local and remote branch deletion. The clean worktree was removed without force. |
| `codex/thumbnail-worktree` — `ea4b0d8fc69934075b169ba7ca025cbd336e0a25` | Both thumbnail commits are superseded by current workers, typed sources, cache and lifecycle code; port none. | Exact-tip lease checks preceded local and remote branch deletion. The clean worktree was removed without force. |
| `origin/fix/cargo` — `8e084b3339ea3bf6cbc72567d7691af1dfb4fa7d` | Incorporated by upstream `origin/main` merge `a9f31d7472bc6ad12cf9856fd513061fef940b50` and completed local merge `28cc79add548eed0d6c9e465a6fe26a5d141f9b7`. Reconciliation retains Docker/native Rust bootstrap with guarded clean/dry-run/native setup, the full app/elevation/filesystem/viewer chain and `.tools/docker/` caches. | The Orchestrator verified exact-tip ancestry and deleted the remote branch using that exact-tip force-with-lease. No local branch existed. Final helper/hook/syntax results are recorded below; branch retirement and local merge are complete. |

Backend/root inspected the archive/thumbnail worktrees for ignored/local files;
the ignored directories identified by the checks were empty, with no unique file
to preserve. `_build` review-output cleanup belongs to Build. The already merged
`main-deep-audit` worktree is intentionally retained because it is the Orchestrator
host's current working directory; it is not an unmerged feature branch awaiting
recovery.

After Cargo-branch retirement the Orchestrator reported
`git branch -r --no-merged origin/main` empty. Archive, thumbnail and Cargo branch
retirement are complete. The Orchestrator also completed integration merge
`28cc79add548eed0d6c9e465a6fe26a5d141f9b7`, including upstream `a9f31d7`.
Branch deletion is not runtime qualification.

Build also reported removing four obsolete 7–8 November BMP screenshot directories,
recovering **4,341,444,608 bytes** (free space at that measurement:
5,322,670,080 → 9,664,114,688 bytes). It retained the panel report's `d731397f…`
debug executable and current UI/Markdown target and SDK caches. These are
time-specific cleanup measurements, not a current free-space guarantee. The
receipt is `_build/qa/build-tools-port-20261112/cleanup-old-bmps.json`; no large
generated receipt is copied into this durable note.

## Source content preserved on main

| Historical commit | Current implementation evidence and reason not to port |
| --- | --- |
| `4b07f3d4d815ec6ecfcb9df51a8b01bac2ebf7ac` — archive worker/typed tabs | [Raw archive identities](../../crates/midnight-core/src/archive.rs), [backend navigation/restore](../../crates/midnight-engine/src/archives.rs), [captured mount/worker sessions](../../crates/midnight-filesystem-host/src/session.rs), [safe member names](../../plugins/midnight-fs-archive/src/tree.rs) and [archive viewer capture](../../crates/midnight-viewer-engine/src/capture_archive.rs) retain the original boundaries. Main adds streaming/private disk staging, verified F5, sizes/timestamps, cooperative cancellation and native-provider/lifecycle ownership. Restoring the eager in-memory/old byte-ratio-limit implementation would regress the accepted streaming contract. |
| `dd396420518ed4bf70145dc107190f4eec137aa9` — sort rollback/unavailable-tab exit | [Presenter submission rollback](../../crates/midnight-ui/src/presenter.rs) and [archive sort/navigation/origin handling](../../crates/midnight-ui/src/presenter/archives.rs) retain every behavioral hunk. Both added [archive regression test](../../crates/midnight-ui/src/presenter/tests/archives.rs) bodies were reported byte-identical after newline normalization. Reapplying offers no missing behavior. |
| `99e619ccc8085f071d3454910317010962613dbf` — worker platform/SDK snapshot | [Worker platform startup](../../crates/midnight-worker-platform/src/lib.rs), separate [Windows](../../crates/midnight-worker-platform/src/windows.rs)/[Linux](../../crates/midnight-worker-platform/src/linux.rs) containment and the [worker SDK](../../worker-api/midnight-worker-sdk/src/lib.rs) retain legacy startup while adding explicit versioned policies/spool/store/browser work. All eleven introduced paths remain. The old SDK/startup-only snapshot cannot replace the newer policy contract. |
| `3ec6d66492f3beafacc04bfeaf14468e60fe1cfe` and `ea4b0d8fc69934075b169ba7ca025cbd336e0a25` — thumbnails | [Decode supervision](../../crates/midnight-thumbnail-host/src/decode.rs), [pinned source service](../../crates/midnight-thumbnail-host/src/service.rs), [validated cache](../../crates/midnight-thumbnail-host/src/cache.rs), [bounded scheduler](../../crates/midnight-thumbnail-host/src/scheduler.rs) and [typed archive source](../../crates/midnight-thumbnail-host/src/archive_source.rs) retain/supersede the old workers and host integration. Main uses thumbnail SDK 0.2.0, shared image decoding with SVG/AVIF, resolution-aware/cache-store isolation, archive leases and cooperative linked cancellation. The old SDK 0.1.0/duplicated decoder snapshots supply no identified missing functionality. |
| `8e084b3339ea3bf6cbc72567d7691af1dfb4fa7d` — Docker/native Rust bootstrap | Upstream `a9f31d7` incorporates the branch. [Root helper](../../build.sh), [Docker build](../../scripts/build-docker.sh) and [image dependency setup](../../scripts/docker-install-deps.sh) retain automatic native/Docker selection, cached Ubuntu/toolchain setup and user-account Rust bootstrap. [Guarded cleanup](../../scripts/clean-build.sh) and [explicit native package setup](../../scripts/setup-linux.sh) from checkpoint `b5b3b7b` are reconciled with those paths in completed merge `28cc79a`. This is incorporated functionality, not discarded branch behavior. |

The archive comparison counted 103 commit/path observations, including repeated
paths, and inspected retained provider/search/sort/restore/containment regression
definitions. The thumbnail comparison inspected both actual patches and their
current counterparts. File presence and patch-ID ancestry alone were not the
basis for accepting semantic supersedence. Existing regression definitions were
not executed by these source reviews.

## Review receipts and remaining work

The detailed review artifacts were:

- `_build/qa/archive-branch-review-20261112/review.md`
- `_build/qa/archive-branch-review-20261112/file-inventory.csv`
- `_build/qa/archive-branch-review-20261112/retained-regressions.csv`
- `_build/qa/viewer-markdown-20261112/thumbnail-review.txt`

The user subsequently authorized deletion of the entire `_build/qa/` tree, owned
by Build. The paths above are historical provenance and need not remain present;
this note preserves the decision, exact historical commits and source basis. No
archive/thumbnail implementation was ported by those reviews. The later upstream
Cargo-branch merge is the separate incorporation recorded above; this documentation
change adds no dependency.
The Orchestrator owns the retirement operations; Documentation records its handoff.

Retiring obsolete branches does not complete archive container attachment/nesting,
generic provider migration, thumbnail native-panel/cache qualification or Markdown
latency work. Those remain in [Active work](../ACTIVE_WORK.md). Fresh tests are
required for subsequent source changes; neither this source comparison nor the
[earlier panel qualification](2026-11-12-panel-interaction-polish.md) proves their
future runtime results. The Orchestrator checkpointed the earlier guarded build
port/README as `b5b3b7b`, then completed merge `28cc79a` including upstream
`a9f31d7`. Build resolved `build.sh`; Documentation resolved README while retaining
the corrected selection shortcuts. README is already committed in the merge;
the remaining documentation is a separate checkpoint.

For the final reconciled script batch, the Orchestrator reports:

- Upstream standalone helper suite: **20 passed**.
- Combined helper suite: **46 cases, 45 passed and one platform skip**.
- Windows hooks: **seven passed**.
- Shell syntax: **six script checks passed**.

The groups are not counted as additive unique coverage. Build implemented the
[mount guard's kernel octal decoding fixes](../../scripts/check-clean-mounts-linux.sh)
after the Orchestrator's diagnosis and review,
including following filename digits and trailing path newlines. Documentation
records these owner-reported gates without rerunning them or claiming a live
package installation, Docker-image build or application/native UI qualification.
The earlier QA script log describes its pre-merge candidate only.

The seven-document checkpoint is committed as
`5ab0e86f40c0164f4700da96e1ff09a3b5f65de5`; merge `28cc79a` is pushed. Documentation
checks/hashes and this cleanup correction are under
`_build/verification/docs-20261112/`, outside QA.

The full QA cleanup is complete: the Orchestrator verified `_build/qa/` absent.
Build's accessible stage removed **681 roots**, reporting **40,625,377,280 bytes**
of space gain (about 40.6 GB). The reviewed frozen UAC helper completed the protected
remnants at **17:15:21 CET (16:15:21Z)** with per-directory ACL repair and no reparse
target traversal. Its separate final-stage free-space change was **155,648 bytes**;
the final free-space snapshot was **46,807,683,072 bytes**. Do not sum those
observations or the earlier four-BMP result: unrelated disk activity continued.
Receipt: `_build/verification/20261112-cleanup/qa-elevated-completion.json`, status
`completed`, `directoryAbsent: true`. Protected debug SHA-256 before/after matches:
`d731397f751647c5feb6ff2ba6ec822a598711bc48ee738e452f364363138984`.
No continued QA receipt or candidate-binary availability is assumed.
