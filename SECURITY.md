# Security and integrity requirements

Security, lossless Unicode and verified copying are product requirements, not
optional fast/slow modes. UI code handles input and presentation only. Filesystem
access, large sorts/indexes, checksum work, large-object reclamation and diagnostic
file encoding/writes execute in backend workers.

## Current read-only milestone

- Browsing does not copy, rename, delete, execute files, invoke a shell, request
  elevation or interpret filenames as code. Explicit navigation into a symbolic
  link/junction follows OS directory-navigation semantics; enumeration and metadata
  inspection do not recursively follow links or open special-file contents.
- Explicit Enter or an unmodified left double click is a separate activation
  action: directories stay internal, regular files use the host association,
  which can execute programs. Selection, hover, right click and refresh never
  activate files. Native metadata/launch and helper reaping stay on bounded
  backend workers, separate from navigation. Missing/special files fail with
  context; native symlinks are classified only after explicit activation.
  Windows passes raw UTF-16 to `ShellExecuteExW(open)` with no command parameters;
  Linux passes the raw absolute path as a literal argument to a fixed system
  desktop opener, without a shell or inherited standard streams. No automatic
  elevation is requested. Host association/path limits are reported faithfully.
  Frozen row identity prevents sort/refresh from substituting another displayed
  entry between clicks; generation checks reject obsolete directory-link results.
  This ordinary path-based activation does not promise immutable inode identity
  against external filesystem replacement; future writes need stronger contracts.
- The command editor is a separate explicit shell action. Only submitting Enter
  starts the configured system terminal on a backend worker, using the active
  pane's captured working directory. Inserted paths are quoted literal tokens;
  typed command text is intentionally executable shell syntax and is not logged.
  Insertion requires a recognized standalone argument position; ambiguous quote
  or expansion contexts are rejected. Intentional eval/sh -c user commands can
  reinterpret arguments and are outside this literal-insertion guarantee.
  Nonempty drafts are bound to their shell grammar: a late preference or picker
  change cannot reinterpret an inserted PowerShell path as cmd code. Clear the
  draft before selecting an incompatible grammar. Explicit POSIX elevation keeps
  the same grammar and still requires Run/Enter; passwords belong only to sudo's
  own terminal interaction. No password or elevated-mode preference is stored.
  No terminal is started by selection, navigation, icons or path insertion.
  Native Properties is also an explicit host-UI action. Unsupported native
  properties fall back to read-only metadata rather than opening/executing files.
- Recursive size scans skip explicit links/special files, use checked totals and
  mark access/resource failures as partial. They are live estimates, not atomic
  snapshots: the current path-based read-only interface cannot exclude races
  with concurrent path replacement. This interface must not be reused as a
  security contract for future destructive traversal.
- Folder scans share weighted capacity across panes. Only a positive local SSD
  hint allows up to four I/O streams; HDD/network/unknown takes exclusive scan
  capacity. Hints are conservative and do not prove immutable physical topology.
- Preferences use bounded strict versioned decoding and opened-file validation.
  Version3 includes named native bookmarks and font choices, with v1/v2 migration.
  Its 4MiB aggregate bound is a settings resource limit, not a path-length limit.
  Native path encodings are lossless and platform-specific; malformed/foreign
  encodings, duplicate IDs/slots and NUL paths are rejected. Late initial loads
  must preserve unrelated saved data when local edits already occurred.
  Native readers reject symlinks/reparse points and special files rather than
  blocking on FIFOs or opening their targets. Own exclusive temp creation and
  atomic publication preserve existing settings on failure. File sync is used;
  no power-loss durability of the parent directory entry is claimed.
- Raw `PathBuf`/`OsString` identities are kept separately from visible text.
  Valid Unicode is not converted through an ANSI code page or normalized into a
  different filename. Names that are not valid Unicode on their host remain
  addressable by their original raw paths.
- Volume labels and tooltip content are plain Unicode text, never Markdown,
  links, shell commands or navigation identities. Label/capacity lookup runs on
  separately bounded backend workers; hover reads cached data. Slow/offline
  volumes do not hold discovery or pane-listing capacity. Unavailable values
  remain explicitly unavailable rather than appearing as zero free bytes.
- Tab/bookmark captions are not identities. Stable typed IDs and raw paths are
  authoritative; results are generation-tagged across tab switches. Inactive
  tabs release large data on backend workers. Drive icons use root-specific
  cache keys; Windows Shell root queries are restricted to validated local fixed
  drive roots, with stock icons for media/network classes to avoid probing them.
- Navigation buttons and dropdowns resolve revision-tagged choices to captured
  native targets. Display labels, duplicate-looking aliases and stale popup
  indices cannot retarget an action after discovery changes. Both surfaces keep
  the same available drives and semantic locations, including same-path aliases.
- Action/location icon roles are fixed semantic keys, not caller-controlled DLL
  indices or resource paths. Native lookup, bounded raster decoding and caching
  stay on backend workers. Unsupported semantic roles retain UI vector fallbacks;
  negative caching prevents repeated native misses. Known-folder PIDLs have
  scoped native ownership; Linux theme PNG loading retains its trusted-root,
  encoded-size, decoded-allocation and lookup-attempt bounds.
  Context discovery coalesces refreshes behind one native worker; typed drive
  letters come from the platform adapter, never UI path-prefix inference.
- The command directory is complete plain text without elision. A scrollable
  presentation viewport bounds extreme layout height without limiting or
  modifying the native path. Viewing or scrolling it never submits a command.
- There is no application-wide path-length ceiling or `MAX_PATH` buffer.
  Provider/OS limits are reported without truncation. Future FTP/remote locations
  have provider-owned path semantics and separate credentials, not host-OS paths.
- Core, engine and composition forbid handwritten unsafe Rust. UI denies unsafe
  code with an exception scoped to generated Slint bindings. The small Win32
  adapter's unsafe calls require an explicit safety explanation.
- Potentially blocking calls are bounded and run on workers. Navigation results
  are generation-tagged, command/result queues are bounded, and cancellation does
  not pretend a still-running OS call has stopped. One pane reserves its own
  listing capacity. Large retired buffers are handed to backend reclamation with
  backpressure rather than destroyed on the UI thread.
- Bulk selection and Properties path expansion run on bounded backend workers.
  UI membership uses immutable bitsets and raw paths, with independent location
  contexts and ordered revisions across refreshes, tabs and failed navigation.
  Stale/rejected work cannot select a replacement entry by recycled numeric ID.
  Size scans retain a separate identity across display sorting and same-path
  refresh; each replacement scan owns a fresh cache. Size-only paints are
  coalesced to250ms without throttling input or error delivery.
  Progress and completion never trigger automatic row reordering.
- Runtime filesystem errors retain operation/path context. Unknown or failed
  metadata is not represented as successful zero-length data.
- The explicit developer `--snapshot` option is a file write, not a browsing
  operation. PNG encoding/writing runs in a backend worker and uses exclusive
  creation; an existing file or symlink is never overwritten.
- Tests and development scripts follow the same literal-argument policy. The
  Windows junction fixture uses a fixed trusted PowerShell script via `-File`;
  temporary paths are parameter data, never `cmd /C` command text.

The directory model still uses memory proportional to the number of entries.
Worker/front-queue bounds are not a promise that arbitrarily large directories
fit in RAM. Precise latency and memory targets require measured workloads.

## Mandatory copy contract for the next milestone

1. Only supported ordinary file types are copied. Opened file identities and
   source/destination alias checks are used; no unchecked traversal of recursive
   links, junctions, named pipes or devices is permitted.
2. Use an exclusively created temporary destination on the target filesystem.
   Handle short reads/writes, propagate errors and retain ownership through
   outstanding I/O and cancellation. Never delete the existing target first.
3. Hash the exact source bytes read, using BLAKE3-256 by default. Complete and
   synchronize the writes, then perform a separate full read of the destination.
   Compare both lengths and cryptographic digests. Hashing the outgoing buffer
   twice, checking timestamps or comparing sizes alone is insufficient.
4. Reject any detected source change. Source metadata checks do not prove a
   point-in-time snapshot against concurrent writers; a stronger snapshot promise
   requires supported write exclusion or filesystem snapshots. Report what was
   actually verified, never an unsupported stronger guarantee.
5. Publish only after successful verification, using the target platform's tested
   no-replace/replacement contract. Unsupported atomicity must cause an explicit
   failure, not a silent destructive fallback. Preserve the existing destination
   when any pre-publication step fails.
6. Distinguish verified content, namespace publication and power-loss durability.
   A cached readback is not necessarily a physical-media reread. Hashes do not
   establish ACL/ADS/xattr/link preservation; those have separate declared rules.

Multi-file Copy/Move/Delete requests share the engine's operation queue. Capture
selection and destination identities when accepting a request; later navigation
must not retarget it. Enqueueing stays nonblocking while another job runs.
Serialize conflicting paths/aliases and prevent a queued destructive dependency
from deleting a source after its preceding copy failed. Per-job cancellation,
phase progress and partial results must stay truthful; a completed write is not
a completed verified copy.

Required corruption/failure tests include modified destination bytes, truncated
data, same-size edits, changed sources, short reads/writes, disk-full/disconnect,
flush/sync failures, destination creation/replacement races and cancellation at
every phase. No copy is marked successful before its complete verification.

## Unicode acceptance

Unicode covers filesystem names and paths, UI models and editing, volume labels,
errors, CLI/configuration, and future copy/search/rename/clipboard operations.
Case mapping for sorting never changes the original path or merges identities.
Composed/decomposed spelling is preserved; filesystem-specific equivalence must
be handled explicitly rather than by globally rewriting names.

Tests include Czech, Chinese/Japanese, Arabic/Hebrew, supplementary-plane emoji,
combining sequences and long Windows paths, plus raw non-UTF-8 Linux names.
Storage/navigation tests do not establish visual shaping: the supported renderer
and available font fallback must also be checked with multilingual native output.
FemtoVG-WGPU is the normal renderer. Slint's documented western-script limitation
means its separate software renderer remains a developer diagnostic path until
it is independently qualified; it must not be advertised as a fully Unicode
production fallback.

## Dependencies

Keep `Cargo.lock`, check advisories against the actual lockfile, and record the
database revision and warnings. A clean advisory scan means no matching known
vulnerabilities in that database; it does not prove the absence of unknown bugs.
Do not adopt source or dependencies merely because they look similar to reference
code. Salamander code/assets are not copied into this implementation.
