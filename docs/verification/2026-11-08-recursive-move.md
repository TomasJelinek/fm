# Ordinary recursive Move — native qualification

This extends the [ordinary per-file Move](2026-11-08-ordinary-move.md) implementation to native directory trees. It adds no source-recovery mode or journal. The implementation plan is `_build/goal/ordinary-move-completion/recursive-plan.md`.

## Behavior and authority

The existing Copy traversal still owns merge, overwrite, Rename and Skip decisions. Move obtains each child through a separate optional `TransferSource::next_move_child` capability. Ordinary Copy sources refuse this method. A native child is opened relative to its held source directory, without following links, and receives its own file/directory mutation authority. The engine checks its direct-parent path, native identity and capture binding before admission.

Files use the existing complete SHA-256 copy, independent destination readback, safe publication, pre/post-staging full hashes and explicit native finalizer. The completed file's source/proof/finalizer handles and empty temporary stage are released before the next child is traversed. Native qualification is lazy per child: a later unsupported entry, writer, hardlink or I/O error stops unfinished work while earlier completed files remain moved.

At postorder, source and destination directory bindings are revalidated. The native source authority then removes only the held, identity-checked empty directory. Neither implementation enumerates and deletes leftover children. A deliberately skipped entry marks all its active source ancestors for retention; siblings can still finish. An unexpected remaining entry causes the OS empty-only removal to fail. Cancellation retains unfinished directories and invokes the existing no-replace restoration path for an active staged file.

Root identity duplicates and overlapping roots are refused before writes, including a file selected together with any selected ancestor directory, in either selection order. Native ancestry is checked using each selected root's parent against directory-root identities. Destination ancestry rejection remains in both admission and directory creation/merge. Traversal accepts at most 64 selected roots, 128 active directory frames, 100,000 admitted entries, and the existing 4 MiB active source/proof path reservation. Retained source handles follow traversal depth rather than total file count. Refresh paths stay bounded at 64 and prioritize selected-root parents.

`source_files_removed` continues to count only verified files. Accepted Windows disposition and unconfirmed source-removal durability counters include empty source directories as well as files, matching the UI's wording of "source removals". These are namespace/durability facts, not reclaimed-byte measurements. Directory removal never increments the file count.

## Platform and metadata scope

- Windows: local fixed NTFS and the existing protected source-parent DACL policy. Source directories hold native DELETE authority and deny incompatible write/delete sharing. Both admission and final removal refuse readonly directories and any directory data stream that was not copied and verified. A native EOF establishes an empty directory stream list; other query failures refuse removal. This matches Microsoft's documented [FileStreamInfo EOF result](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getfileinformationbyhandleex). Directory removal uses the retained handle and the kernel's nonempty-directory refusal. Normal disposition can remain pending until handles close; physical free-space reclamation is not promised.
- Linux: local ext filesystems and the existing protected source-parent policy. Regular-file read leases, observed-break refusal and full verification remain unchanged. Directory removal uses the held parent descriptor, a final no-follow identity check and `unlinkat(AT_REMOVEDIR)`. Linux does not supply inode-CAS `rmdir`; same-UID/root namespace interference and an arbitrarily descheduled process remain outside the live-validation guarantee. No atomic tree snapshot or whole-job rollback is claimed.
- Recursive admission is ContentOnly. RequiredModifiedTime continues to refuse directory selections before destination publication. Other directory timestamps, permissions/ACLs and extended metadata are not promised. Links, reparse points, special files, mount crossing and virtual/provider Move are explicitly unsupported. Their later discovery preserves unfinished sources and already completed destinations.

## Verification ledger

Initial Windows recursive success/cancel/skip tests failed causally at the old regular-file-only admission check (3 tests). Native tests use private owned fixtures. The fixture ACL adjustment makes only its existing token-user ACE inheritable; it changes no user profile, project ACL or production permissions. Linux fixtures use native ext storage under `/tmp/fm-interaction/_build/tmp`.

The Windows directory-stream test failed causally before its native admission/removal gate was added, then passed. A Linux overlap test failed causally for a selected directory plus its nested selected file; the corrected ancestry admission passed in either selection order.

Final backend gates:

| Gate | Windows NTFS | Linux ext |
| --- | --- | --- |
| Engine `--lib copy_operations` | 67 passed, including 14 recursive cases | 59 passed, including 15 recursive cases |
| Native platform `--lib transfer` | 28 passed | 30 passed; one unrelated X11 clipboard test ignored because it requires an isolated display |
| Common `--lib transfer` proof/copy tests | 24 passed | 24 passed |
| Native platform + engine Clippy, all targets, `-Dwarnings` | Passed | Passed |

Recursive coverage includes nested and empty directories, cancellation after one completed file, late hardlink refusal preserving earlier moves, explicit merge/skip retention, merge/replacement with old-target retention, directory Rename, raw UTF-16/raw-byte and long paths, 192 long child names crossing native enumeration pages, unexpected-child removal refusal, Copy-source non-escalation, selected file/directory overlap, destination-inside-source rejection and the 128-frame depth limit retaining unfinished contents. Linux additionally exercises source-directory substitution and link non-following. Windows additionally exercises refusal of directory alternate data streams. The existing per-file writer, mapped-writer, observed lease-break, changed-destination, restore-conflict and restore-durability regressions remain green.

All Cargo gates used `--locked --offline`, debug information disabled and incremental compilation disabled. Windows used `scripts/cargo.ps1`; Linux used the existing `_build/qa/interaction/linux-verification-env.sh`, two jobs and the native temporary-directory runner. Exact logs are `_build/goal/ordinary-move-completion/{windows,linux}-recursive-{engine,native,common,strict}.log`. Owned Rustfmt and whitespace checks pass. The source-size check covered 1,217 files, all at most 100,000 bytes.

Parent-agent review checked directory identity authority, held Windows DELETE handles and postorder empty-only removal; no further defect was found. No dependency, user profile, non-fixture permission, deployment, commit or push change belongs to this work. Packaged application and viewer qualification remain with the parent integration task; these are native backend completion results.
