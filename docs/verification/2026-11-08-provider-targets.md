# Retained provider targets

The typed-target prerequisite for inline trees was qualified separately from
panel actor/UI integration. Native targets use retained raw paths; archive
targets use complete raw ancestry and exact parent/member identities. Target
resolution does not extract files, parse displayed paths or grant mutation
authority. Tree source-entry resolution distinguishes a container file from
its entered root and rejects hidden, root and foreign occurrences.

The selected change was built on `54033d2` in an isolated source tree. It contains
13 code/test files and focused documentation. Only the metadata-localization
helper is taken from the mixed engine root file. The optional locale method is
additive. Three test lines referring to the separate, unpublished archive format
field are omitted from the candidate; they remain in the full working tree.
Production clones the complete captured address, preserving both model shapes.

## Tests and review

- Four tree target cases first failed against the unsupported stub; exact nested
  paths, source/entered-container separation and hidden/foreign rejection then
  passed. A fifth case verifies enclosing-provider revocation.
- Nine archive target cases cover raw ancestry, captured base immutability,
  unreadable/link handling, stale source, cancellation, invalid names, cycles,
  budgets and mount retention.
- Review found that cloning an archive handle could escape the original target
  wrapper. The regression fails without the leased archive wrapper and passes
  with it; it verifies denied list/read/validate after revocation, no underlying
  calls, and archive-before-provider destruction.
- Four blocked-resolution cases verify cancellation and registration loss while
  a provider call runs, successful retry, and rejection of foreign/unissued nodes
  before invoking the provider. Native capture cancellation discards late success
  without cancelling the caller or revoking its mount.
- Native capture is explicitly injected, with no built-in read fallback. Locale
  tests cover complete/incomplete metadata and avoiding unnecessary stat calls.

Fresh gates on 8 November 2026:

| Source set | Windows | Linux |
| --- | --- | --- |
| Full working-tree provider tests | 78 passed | 78 passed |
| Full working-tree engine library | 356 passed, 1 existing ignored | Not repeated in this gate |
| Full working-tree engine all-target strict Clippy | Passed | Passed |
| Isolated candidate provider tests | 78 passed | 78 passed |
| Isolated candidate engine library | 330 passed, 1 existing ignored | Not repeated in this gate |
| Isolated candidate engine all-target strict Clippy | Passed | Passed |

The candidate formatting check passed after formatting only the archive import
list; Linux's pre-format inputs are retained. Source-size checks cover 1226 files,
all within 100,000 bytes. No dependency, plugin API, manifest or lockfile change
is needed for the target change.

The first Windows candidate compilation reused an older `midnight-core` artifact
from another snapshot in the shared target directory. Its source already had the
required method. Cleaning that package's build outputs and rebuilding unchanged
candidate sources passed. Linux explicitly compiled the candidate core and
passed without cleanup. No source change concealed that cache discrepancy.

Evidence: `_build/goal/inline-tree/target-selected-files.json`,
`target-commit-closure.md`, `targets-red.log`, `target-clone-red.log`,
`target-context-green.log`, `targets-engine-full.log`, `targets-clippy.log`,
`targets-linux-*`, `targets-isolated-windows*`, `targets-snapshot-linux*` and
`targets-isolated-format.log`. The source manifest distinguishes current-worktree
and selected-candidate hashes. This is backend qualification; production inline
tree selection, actions, watches and Slint rows remain to be integrated.
