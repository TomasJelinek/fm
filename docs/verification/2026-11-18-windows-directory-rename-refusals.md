# Windows directory renames refused while files below are open — 2026-11-18

## Scope

Windows refuses to rename a directory while a file below it is open. Antivirus
scanners open newly created files, so a rename of a folder whose contents were just
written (a recursive case change right after its children were edited, or a tree or
panel rename of a freshly copied or extracted folder) failed transiently with
"Access is denied". The native rename in `midnight-platform-windows`
(`mutation_native.rs`) now waits out that specific refusal for a bounded time, only
after confirming that the caller still has the access the rename needs; a paused job
waits paused and a cancellation ends the wait at once. Other errors are reported at
once. Same-volume Move and Move capture, which hold the process-wide namespace turn,
and recycling are not waited out; they stop with Retry/Skip/Cancel. See
[native mutation safety](../../crates/midnight-platform-windows/MUTATION_SAFETY.md).

Source: integration batch 10d, commits `796a6be8`, `d165f939` and `02b9ff5e` on
`60257c4b`, merged as `26bb126b`.

## Environment

Windows 11 Enterprise 10.0.26200, NTFS on D:, Microsoft Defender real-time and
on-access protection on (AMProductVersion 4.18.26100.4); debug builds on a shared,
loaded machine.

## Results

Previously flaky engine tests, isolated runs (`<test> --exact --test-threads 1`):

| Test | Before (`60257c4b`) | After |
| --- | --- | --- |
| `operations::metadata_tests::recursive_case_and_times_are_postorder` | 97/100 (reviewer: 80/100) | 100/100, twice |
| `operations::tests::tree_rename::native::native_captured_tree_rename_files_and_directories_keep_unselected_siblings` | 96/100 (reviewer: 98/100) | 100/100, twice |

Every failure before the change was "Access is denied. (os error 5)" renaming the
parent folder or the folder `selected 資料`. After the change the wait fired in 11 of
100 and 1 of 100 runs, one 10 ms wait each. Mutation checks (removing the access
confirmation, the bound, the pause behaviour or the cancellation step) each failed
the new deterministic tests in `mutation_native_tests.rs`, which deny access through
a shared test DACL fixture and inject each step's sleep.

## Limitations

- The wait covers native directory renames only; recycling a folder while an entry
  below it is open still fails, and its Shell error (COPYENGINE `0x80270021` or
  `0x80270027`) is shown with its code.
- Same-volume Move is not waited out (it holds the namespace turn); it stops with
  Retry/Skip/Cancel.
- Measured on one machine with Defender; other scanners may hold files longer than
  the bounded wait.
