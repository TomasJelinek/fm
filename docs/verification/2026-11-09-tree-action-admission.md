# Inline-tree root handoff and action admission

Date: 2026-11-09. Working-tree prerequisite implementation, following the
[production integration plan](../superpowers/plans/2026-11-09-inline-tree-production.md).
This report does not claim ordinary panel tree support or completed operation
integration. The separately committed build/package work is `fab0d5d`.

## Implemented and reviewed

- Native/archive completion retains the original entries and index before UI
  publication. Root admission compares pane, navigation and both exact Arc
  allocations, rejects pending navigation and reuses completed listings.
- Typed selection capture requires the acknowledged view and exact selection
  allocation/revision. It retains occurrence order and source registrations,
  uses fallback only for an empty selection, and refuses context-only/collapsed
  targets. Failure/cancellation does not publish a partial batch.
- Captures retain real native/archive ancestry across navigation. Admission is
  bounded to 4,096 targets and 4 MiB of newly owned metadata/context bookkeeping.
  Mutation proofs and operation-specific overlap handling remain required.
- Review found final archive-source destruction under the snapshots lock.
  Replacement now detaches ownership under the lock and retires it outside the
  lock on a blocking worker. Archive completion keeps its admitted worker permit
  through retirement. Shutdown cancels publishers before draining both caches;
  late cancelled or superseded completion cannot restore a cached root.

Independent follow-up review found no further actionable issue in these paths.
No third-party dependency, public plugin ABI or native parent-proof rule changed.

## Regression evidence

Windows evidence is under `_build/qa/tree-integration/windows/`:

- `engine-tests.log`: 671 passed (469 unit and 202 integration), no failures and
  one explicitly ignored debug selection CPU benchmark. Doc-tests passed with
  no examples. `audit.json` records unchanged source inputs.
- The broad pre/post manifest covers 1,279 Rust source/manifest files, SHA-256
  `7a5d1d8902cc22fd5a00b23558028ea5a1e03354ed66e765e168cd757ba1a146`.
- Clippy identified two large copy-preparation error envelopes and one redundant
  async test wrapper. Boxing the error envelopes preserves retained preparation,
  retry and worker cleanup. After those three-file changes, `lint-copy-tests.log`
  records 90 passing copy tests and `lint-tree-test.log` one passing blocked-worker
  cancellation test. `clippy-final.log` records a successful
  `cargo clippy -p midnight-engine --all-targets --no-deps -- -D clippy::all`.

Linux evidence is under `_build/qa/tree-integration/linux/`:

- `results.json` and `engine-regression.log`: 663 passed (460 unit and 203
  integration), no failures and the same single ignored debug benchmark.
- Separate focused runs passed six root tests, eleven action-capture tests and
  four retirement tests; these overlap the full regression count.
- The audit covers 603 unchanged inputs across 15 local dependency packages,
  manifest SHA-256 `f18ff4244f706c32c97cc9c28cb0e725421f966882986236846ec3cb2b2f4cec`.
- The later three-file lint correction is checked separately in `lint-followup/`;
  its source delta is distinct from the frozen full-regression snapshot. All 82
  copy tests and the single blocked-worker cancellation test passed, as did the
  same Clippy command used on Windows. All 603 inputs remained unchanged during
  this follow-up. `lint-followup/results.json` records the three final file hashes.

TDD evidence: `_build/qa/tree-action-capture/` records runtime failures against
the explicit capture stub. `_build/qa/tree-roots/red.log` records five failing
root-handoff tests before implementation; the foreign-input rejection test
already passed. `retirement-red.log` reproduces destruction under the snapshots
mutex through the real archive controller before its fix.

## Limits

The engine still reports 52 unused/dead-code warnings associated with tree
primitives that are not fully connected to production. The Clippy command above
does not claim a clean `-D warnings` gate. No new suppressions were added.
Scoped rustfmt passed, new source files use UTF-8/LF and each is below 100 KB.

Global controller routing, presenter integration, descendant operation consumers,
expanded-branch watches and container discovery remain open. Tests here use
native/archive provider fixtures and engine boundaries; they do not establish
native plugin/Slint tree interaction, an updated application binary, a green
whole-workspace gate or completed VFS migration.
