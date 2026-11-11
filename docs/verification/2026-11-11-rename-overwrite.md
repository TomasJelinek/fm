# Rename file collision decisions

The accepted behavior uses the existing Copy/Move overwrite window when a Rename
meets an existing ordinary file. Preparation is read-only. Replace requires the
exact job and conflict version; Skip, Cancel and shutdown preserve both entries.
Directory/link collisions remain unsupported, without destructive directory
merging. Same-name and case-only renames keep native behavior; distinct hard-link
aliases cannot authorize replacement.

The implementation reuses `TransferMoveCapture::try_native_move` and
`RecoverableCurrentEntry` from the accepted [native Move design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md).
The new optional `PreparedMutation::prepare_rename_replacement` adapter binds the
source to its original prepared identity and the destination to its retained
parent. The existing no-replace Rename runs first. Only a file collision enters
this additional preparation. Changed target revisions discard consent and require
a fresh prompt. Other native failures never enter a Copy or delete-first path.

Salamander's `RenameFileInternal` in the local reference
`D:/experiments/salamander-main/src/fileswn5.cpp` (around lines 2108–2300) confirms
the file-overwrite interaction through `COverwriteDlg`. Its delete-before-rename
publication was not reused. Midnight retains the displaced original and exposes
its recovery path in Recent Jobs. Committed/recovery native outcomes are reported
as actual namespace facts even if cancellation or another error follows.

Windows replacement remains qualified only for the existing fixed local NTFS
capability. The original destination is moved into a private recovery directory;
source publication and any restoration use no-replace operations, preserving a
new destination winner in the namespace gap. Linux retains the existing local
filesystem/protected-parent qualification and exchange/recovery implementation.
Neither platform contract promises an atomic identity compare-and-swap,
concurrent-writer snapshot, or physical-media durability. Unsupported providers
return an explicit error rather than relaxing these guarantees. No plugin ABI or
dependency changes are required.

## Verification status

Source tests added before shared-target execution:

- Platform Windows `mutations::rename_replacement*`: no mutation while preparing,
  explicit replacement retains destination bytes, cancellation, stale target,
  self/hard-link/directory rejection and original-source identity replacement.
- Engine `operations::rename::tests::rename_conflict*`: real native worker through
  versioned decisions; explicit Replace and recovery-path reporting, stale target
  re-prompt, stale response rejection, Skip/Cancel/shutdown, native same-name and
  case-only behavior, aliases, directory collisions and Windows deny-delete error.
- UI `presenter::rename_decisions::tests`: exact token/version dispatch, failed
  sends, replay rejection, deferral/Jobs resolve, cancellation and retirement.

Scoped rustfmt and `git diff --check` passed. The first targeted engine run stopped
all five cases during fixture preparation: its `../..` root was correctly rejected
by native path validation. The owned root was canonicalized; production validation
was unchanged (`_build/qa/dialog-polish-20261111/engine-rename_conflict.log`). The
pinned toolchain's `AtomicU64::try_update` spelling replaced its deprecated alias.

The corrected v2 engine run passed 4/5 (`engine-current-rename_conflict.log` in the
same QA directory): explicit replacement/recovery, changed-destination fresh
consent, Skip/Cancel/shutdown, and native permission failure all passed. The fifth
case reproduced Windows ordinary no-replace Rename reporting success for a
distinct hard-link alias before the conflict path could run. Ordinary Rename now
uses the existing metadata name-availability guard while holding its pinned
source handle that denies delete/rename sharing. That guard additionally requires
locale-free ordinal case equivalence to the selected basename before treating a
same-file target as a case-only rename; this also rejects differently-cased aliases
of another hard-link name. Exact native-name enumeration remains for ambiguous
multiple links, and final publication stays no-replace.

Before that last guard correction, all 24 Windows mutation tests passed
(`ui-v2-mutations.log`), including the three new replacement/cancel/stale/source
identity cases. All 3 Rename decision unit tests passed (`unit-v2-rename_decisions.log`),
and all 11 native Jobs-window tests passed (`ui-v2-native_job_windows.log`), including
Decide later → Jobs Resolve → shared Replace with exact `DecideRename` dispatch.

The final v3 run passed all 5 engine `rename_conflict` cases
(`_build/qa/dialog-polish-20261111/v3-engine-rename_conflict.log`) and all 26 Windows
mutation cases (`v3-windows-mutations.log` in the same directory). These include
`rename_rejects_distinct_hardlink_aliases_even_when_target_case_differs` and
`rename_case_only_with_another_hardlink_keeps_native_unicode_spelling`: both
names/bytes survive rejected aliases, exact same-name behavior stays valid, and
Unicode case-only rename keeps actual directory spelling with another hard link
present. All 3 Rename decision tests also passed after the shared batch-history
retirement helper change (`v3-ui-rename_decisions.log`). This provides native
Windows filesystem evidence for the corrected guard; Linux execution and an
interactive desktop run are not claimed by these Windows/software-window logs.
