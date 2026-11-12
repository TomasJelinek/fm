# Confirmed permanent deletion and Windows Read-only attributes

The user reported error5 deleting
`C:\Users\tjelinek\OneDrive - NVIDIA Corporation\Pictures\Backgrounds\QQQ`
and explicitly requested a permanent-delete test of that exact directory.
Read-only was present alongside Directory/Archive/Compressed and a OneDrive
cloud reparse tag. No arbitrary files or sibling directories were deleted.

## Cause and change

Owned regular-file and empty-directory fixtures with the DOS Read-only bit both
reproduced `Access is denied (os error 5)` through the product MutationService.
The native `FileDispositionInfoEx` call lacked the flag allowing Read-only entries
to be removed. [Microsoft's disposition contract](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/2e860264-018a-47b3-8555-565a13b35a45)
documents both this flag and the continuing requirement for DELETE access.

Confirmed permanent deletion now uses that flag through a dedicated helper.
The existing helper used by Copy/Move cleanup is unchanged. Both paths retain
the same source/parent identity pins, no-delete-sharing guard and no-follow
traversal. No attribute clearing, ACL change, elevation or reparse following is
added. The hard-link regression checks that another name for the same file keeps
its contents and Read-only attribute after the confirmed name is removed.

## Verification

Windows, local NTFS, offline locked Cargo, two jobs, isolated output under
`_build/delete-followups/target`:

- Both new Read-only regressions failed with error5 before the implementation
  (`readonly-red.log`).
- Full `mutations` integration: 21 passed, including long Unicode paths, native
  replacement rejection, cancellation and junction target preservation.
- `mutation_permissions`: 4 passed; leaf operations retain their narrow rights
  and recursive traversal still requires directory listing rights.
- `transfers`: 14 passed; existing Copy/Move cleanup helper behavior is preserved.
- Total: **39 passed, zero failed** (`readonly-green.log`).
- Clippy for the same three integration targets passed with `-D warnings`
  (`clippy.log`).
- The real delete-confirmation UI test reproduced the missing single-folder
  name, then passed after the dialog included its Unicode name and modified date.
  Folder metadata size is not presented as a recursively computed byte total.
  Logs: `dialog-red.log` and `dialog-themes-green.log`.

A throwaway helper under `_build/delete-followups/probe` invoked the same product
MutationService, hardcoded to the exact user-requested QQQ directory. A read-only
preparation captured that target first. The explicitly authorized permanent
deletion then succeeded, and a separate filesystem check confirmed it absent.
The helper SHA-256 was
`82E9AB6795D5BB55B8520AABF96178B7DB39B5AC595F8100EDCBE9719932B67B`.
It is not a product executable, distribution file or general deletion utility.

This evidence establishes the backend and confirmation correction. A newly
packaged main app is a separate check. Nonempty cloud reparse directories and a
general provider-action audit are not claimed complete here.
