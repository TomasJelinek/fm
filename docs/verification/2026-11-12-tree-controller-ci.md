# Tree-controller CI event-order regression

Baseline: c7dcd29 working tree; original failing CI ran b9f8ee4.
Scope: one integration-test fixture; production code unchanged.

Ubuntu job 130940094445 in run 43144250124 failed
foreign_view_is_refused_before_it_can_queue_behind_provider_validation at its
first-event assertion (17 passed, 1 failed, 1 existing ignored). The job log is
preserved in `_build/qa/tree-controller-ci-20261112/ubuntu-job.log`.

The test blocks the tree model in provider validation, submits a projection with
matching data but a different Arc identity, and requires rejection before the
blocked model can run again. Mailbox::push already checks that exact identity
before adding the command to its bounded queue. The fixture instead treated the
first raw engine Event as the rejection. Background icon results share that
stream and can arrive after the initial tree publication. The CI failure was
reported before the 200 ms timeout; its original assertion omitted the event.

A deterministic reproduction keeps a legitimate IconLoaded event pending before
submitting the foreign view. Against the original assertion, Linux fails with
that exact IconLoaded value (1 failed, 18 filtered; 0.14 seconds), recorded in
linux-red.log. This establishes the event-order defect without probabilistic
repetition or modifying production admission.

The corrected fixture retains the background event and skips only non-tree
messages inside one unchanged 200 ms outer deadline. It additionally requires
the exact rejected view stamp and request ID zero. The result is captured while
provider validation remains blocked; the gate is then released and the engine
shut down before asserting, so a failure does not strand the fixture worker.
Unexpected tree events still fail, and each skipped event does not reset time.

Qualification, with logs in the same QA directory:

- Linux full target: 18 passed, 1 existing ignored, 0 failed; 0.35 seconds
  (linux-green.log). Strict test-target Clippy passed in 33.17 seconds
  (linux-clippy.log).
- Windows full target: 18 passed, 1 existing ignored, 0 failed; 0.45 seconds
  (windows-green.log). Strict test-target Clippy passed in 1.02 seconds
  (windows-clippy.log).

The viewer agent independently reviewed the corrected fixture and found no
weakened deadline or admission assertion. Both receive-loop and gate-release
ordering preserve the requirement to reject before model validation unblocks.
This test-only correction requires no production rebuild or redeployment.
