# Recursive metadata traversal (META-03) — 2026-11-18

## Scope

Recursive Attributes, Times and Case edits capture each folder's child names through
its pinned directory handle and pin each child only when the traversal processes it
(`PreparedMutation::metadata_children` returns a `MetadataChildren` snapshot; the
engine takes the names in order with `pin_next`). This record covers the change in
integration batch 10b: source `93d36411` plus commits `30118b08`, `08b6bbd3`,
`cecd6ddf` and `7aede8d1`, merged into the integration branch as `54ed7b78`.

## Environment

- Windows 11 Enterprise 10.0.26200, NTFS, Microsoft Defender real-time protection on.
- WSL2 Ubuntu 24.04, ext4 (build label `l-10d`).
- Rust toolchain 1.99.1, debug test profile. Windows runs used `TEMP` on the
  qualified D: volume (see [Targeted test execution](../developer/test-execution.md)).

## Before the change (`93d36411`)

All four new real-adapter tests failed:

- a 1,000-entry recursive edit: "more entries than the 255 that could be pinned";
- other pending metadata jobs holding the shared targets: "none of this folder's
  entries could be pinned, because other pending metadata jobs hold 255 of the 256
  shared targets";
- a preview prepared during a running recursive edit: "The shared metadata handle
  budget is in use";
- two queued 1,000-entry edits: "more than the 65 that could be pinned".

## After the change

All four complete. Mutant checks (environment-gated, never committed) confirmed that
the budget refund, the folder stop, the collision rule and the snapshot drop order are
each covered: removing any of them fails two to seven tests.

Windows (`cargo test` through the build-slot wrapper):

- engine `operations::metadata_tests` and `operations::tests`: 277 passed, 0 failed
  (34 metadata tests);
- `midnight-platform-windows` lib `mutation`: 8 passed, 3 ignored; integration
  `metadata_edit` 12 passed, `mutations` 26 passed and 1 ignored,
  `mutation_permissions` 5 passed;
- `midnight-core`: 93 passed;
- isolated repeats (`--exact --test-threads 1`): the 1,000-entry edit, the running
  edit leaving the shared budget to previews, and queued jobs holding every shared
  target each passed 20 of 20;
- `cargo clippy -p midnight-core -p midnight-engine -p midnight-platform-windows
  --all-targets -- -D warnings` and `cargo fmt --check`: clean.

Linux (WSL):

- `midnight-platform-linux` lib `mutation metadata`: 74 passed, 2 ignored; integration
  10 passed;
- engine `operations::metadata_tests` and `operations::tests`: 272 passed, 0 failed
  (36 metadata tests);
- isolated repeats: 20 of 20 each;
- `cargo clippy -p midnight-core -p midnight-engine -p midnight-platform-linux
  --all-targets -- -D warnings`: clean.

Wall times (debug, isolated, `--test-threads 1`; Windows numbers vary with Defender
scanning on a loaded machine):

| Test | Entries | Windows NTFS | WSL ext4 |
| --- | ---: | --- | --- |
| 1,000-entry Lower + Times | 1,306 | 1.8-9.7 s | 1.08 s |
| Two queued recursive jobs + a flat job | 2 × 1,001 + 253 | 4.7-17.1 s | 0.28 s |
| Hidden attribute | 1,012 | 2.9-8.0 s | - |
| Permissions | 1,012 | - | 0.13-0.15 s |

Each folder is listed once natively (formerly twice when it exceeded the free
reserve).

## Known limits and observations

- The real-adapter 1,000-entry test changes only file-name case: Windows refuses to
  rename a folder while a file below it is open, and Defender scans newly created
  files. Folder case renames on the real adapters are covered by the 3-entry
  `recursive_case_and_times_are_postorder` test, which failed 3-4 of 40 runs on this
  machine with "Access is denied" for the same reason.
- A case-only rename of a hard-linked file still reads its whole directory, in the
  preview probe and again in the edit, on Windows and on Linux where the new spelling
  resolves to the same file (casefold ext4/f2fs, CIFS).
- The Linux pin still accepts a Btrfs/bcachefs subvolume root on another `st_dev`;
  `RESOLVE_NO_XDEV` refuses only mount points, and recursive Delete traverses
  subvolumes the same way.
