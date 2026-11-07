# Recoverable copy and folder merge verification

The source tree now exposes explicit recoverable replacement and merge choices
through the existing backend queue. This is independent of F6/Move, which remains
an integration task. Guarantees and retained-original limitations are defined in
[TRANSFERS.md](../developer/TRANSFERS.md).

## Evidence

- Windows and native Linux verified-copy common tests: 9 passed. They exercise
  short writes, independent readback corruption/truncation, cancellation, source
  changes, terminal partial commits and retained recovery receipts.
- Linux engine copy/queue/archive regressions: 35 passed at the copy checkpoint.
- Windows engine copy-related tests: 37 passed on the later combined tree, which
  includes two other copy-named tests. One earlier full run had a safe pre-copy
  target-revision refusal; see the unresolved observation below.
- Windows native transfer integration: 13 passed; Linux: 16 passed. Native tests
  cover target substitution, protected original retention, late no-replace winners,
  unsupported strong conditional overwrite, merge ancestry and cleanup ownership.
- UI: 387 library tests, two actual dialog-layout and seven file-action integration
  tests passed after Find integration. The replacement controls distinguish
  Replace/Replace all keeping originals from Merge/Merge all, and preserve a usable
  dismissal/action row at 320×200. Software-rendered normal/small screenshots were
  inspected. Existing native source/target type, byte count and precise UTC date
  fields remain visible or scrollable, with Jobs recovery details.

Independent review found and corrected three concrete defects: Replace all could
cover a destination that appeared after verification, a selected source root could
be another file's replacement target, and native target display/revision snapshots
were not bracketed consistently. Regressions cover all three. Windows revision
checks now include native ChangeTime, including same-size edits with restored
LastWriteTime. Recovery history wording was corrected: Jobs retains a bounded
recent list, not persistent recovery records across restarts.

## Unresolved safe refusal observation

One Windows engine regression returned `consented destination changed` before any
bytes were copied or verified, although the test did not intentionally alter its
target. The isolated case passed. Instrumented full-suite runs, including the
original short-circuit predicate shape, did not reproduce a differing revision.
An owned-file ChangeTime/security-descriptor observation also found no change.
The original failing executable was replaced before a hash was recorded, so these
runs do not establish a causal fix or a controlled binary comparison. External
metadata activity remains a hypothesis, not a conclusion.

No revision check was relaxed and no test was ignored. The observed path fails
closed with source and existing target intact. Further reproduction should record
the exact expected/current native revision and binary hash; an unobserved race is
not labelled fixed. Temporary diagnostic instrumentation is removed afterward.

Native permission-dependent pre-existing tests retain their explicit ignore reasons.
Release/runtime deployment must be rebuilt from the final reviewed tree; the older
installed release is not evidence for these new source changes.
