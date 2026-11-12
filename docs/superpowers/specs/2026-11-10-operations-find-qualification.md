# Real operation qualification and advanced Find

The user requests actual copy/move/delete exercises on owned test files, including
all application entry points, interruptions and recoverable failures, and a full
Find usability, correctness and performance pass. Add recent modification and
duplicate-content searches, using Salamander behavior as reference.

## Decisions and constraints

- Continue in the existing shared checkout, preserving unrelated changes. Keep
  generated fixtures, reports and binaries under `_build/qa/operations-find` or
  `_build`; toolchain caches remain in `.tools`. No new dependency is planned.
- Use the existing production Engine/native services and actual UI callbacks;
  shell copy/delete commands are not evidence that application operations work.
- Destructive tests touch only uniquely created, owned fixtures. Never fill a
  real disk, empty the system recycle bin, alter user ACLs or change antivirus
  settings. Recycle tests are explicit opt-in tests. Real external-volume tests
  require a separately checked owned root. Fault injection is labeled separately
  from actual native-I/O failures and never presented as hardware qualification.
- Keep Windows/Linux implementations separate. Common query/matching logic and
  backend contracts are portable. Do not claim a native platform run from mocks.
- Preserve checksum defaults, same-volume rename behavior, overwrite authority,
  cancellation cleanup, source identity/revision checks and parent-entry guards.

## Approaches

Extend the existing streaming Find service rather than introducing a second
search service or a persistent index. Its bounded worker, session/generation
identity and read-only result validation already serve modeless searches. A
separate indexing database would add stale-state and lifecycle costs outside this
request. Use staged duplicate comparison rather than hashing every discovered
file, reducing I/O for unique file sizes.

These implementation choices follow the user's requested features and existing
architecture; they are builder decisions rather than a new product scope.

## Find behavior

- `modified_within: Option<Duration>`: UI accepts a positive integer and
  minutes/hours/days. Backend freezes one reference time at request acceptance,
  including queue delay; match the inclusive interval ending at that time.
  Unknown timestamps do not match an enabled time filter. Directories outside
  the interval are still traversed. Restart captures a new reference time.
- `duplicates: bool`: only ordinary files matching the other filters become
  candidates. Deduplicate captured native identities so overlapping roots and
  hard-link aliases do not create false duplicates. Prune unique sizes, stream
  SHA-256, then compare bytes exactly within matching hash buckets. Reopen and
  revalidate identity/revision before and after reads. No whole file is retained.
- Candidates have a separate 100,000-item cap plus the existing retained-byte
  budget. Hashing/comparison reads count against total content-I/O budgets.
  Reached budgets, changed files and inaccessible files disclose incompleteness.
  Never publish a singleton merely because the result cap split a group.
- Results retain stable IDs and explicit duplicate group ID/count. Sorting must
  not lose this context. Progress distinguishes scanning, hashing and exact
  comparison, includes bytes/groups, and stays cancellable. No automatic deletion
  or selection of an alleged redundant original is introduced.
- Content matching remains literal with case and whole-word options. Support
  UTF-8 (optional BOM) and BOM-marked UTF-16LE/BE. Strict incremental decoding
  rejects malformed tails even after an early match. Read in bounded 64 KiB
  chunks; compile query state once per search and reuse buffers across files.
  Whole words use Unicode letters/numbers/connector/extending characters; matches
  must begin and end at original character boundaries even after case folding.
- Keep Masks/Regex/Expression and content readily accessible. Disclose extra
  date/duplicate/word controls compactly, with a visible indication when active.
  Preserve drafts on resize, progress, sort, Stop and panel navigation. At
  320×200 content scrolls and Close remains reachable. Results stay virtualized;
  add PageUp/PageDown without changing stable-token activation.

## Reference findings

Salamander `src/filter.cpp:228–255` freezes the relative time interval;
`src/find.cpp:1006–1025` removes singleton size/name candidates before hashes;
`:819–885` reads checksum chunks and checks cancellation; `:1109–1120` assigns
duplicate group numbers. `src/finddlg1.cpp:460–466` preserves grouping during
sorting. Adopt behavior, not source/assets, ANSI paths or short-read assumptions.

## Evidence required

Actual disk fixtures contain empty files, many small files, Unicode names,
long paths, nested/empty directories, large streamed files, read-only targets,
same-size differing files and repeated content under different names. Exercise
copy/move through keyboard, toolbar, clipboard and drag/drop routes; overwrites,
queueing, skip/retry/cancel, permanent delete and opt-in recycle. Check final
bytes/identities, untouched sentinels, source retirement, residual staging names
and progress/selection state. Every failure found gets a regression test.

Find tests cover names/modes, content chunk boundaries/encodings/case/words,
time boundaries/future/unknown values, duplicate grouping/hard links/changing
sources, concurrent sessions, Stop/restart/Close and stale events. Record native
search times, scanned bytes, cancellation latency and UI model/render costs on
large result sets; compare before/after where a change claims improvement.

Update developer/design/feature-status documentation with measured evidence and
remaining limitations. The larger application goal is not completed by this pass.
