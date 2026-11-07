# Security and integrity requirements

Security, lossless Unicode and verified copying are product requirements, not
optional fast/slow modes. UI code handles input and presentation only. Filesystem
access, large sorts/indexes, checksum work, large-object reclamation and diagnostic
file encoding/writes execute in backend workers.

## Current browsing and explicit-operation milestone

Rename/F2, create-directory/F7 and deletion are explicit, confirmed operations.
Preparation pins native identity before showing the confirmation; navigation never
retargets a queued job. The backend permits at most 32 preparing/prepared/queued/
running mutation jobs and 32 copy jobs, two mutation preparation workers and one
active mutating job across both queues. Each queue is FIFO; cross-queue ordering
is acquisition order at the shared mutation gate. Native handles
scale with the selected root count and traversal depth; resource failure is an
error, never permission to substitute a target. Duplicate operation IDs cannot
execute twice. Rename never overwrites. F7 accepts explicit absolute or panel-relative
paths, including `..` outside the captured panel. It walks components through native
handles without following entered links or lexically collapsing an untraversed
component before `..`. Explicit creation paths may cross drives/mounts; this does
not relax recursive destructive traversal. The final directory must be new, and
partial intermediate creation is retained on failure or cancellation. Empty-file
creation still accepts only one basename. See the [F7 path contract](docs/superpowers/specs/2026-11-06-create-any-path.md).

Delete/F8 uses only system Trash; Shift+Delete/Shift+F8 explicitly requests
permanent deletion after a distinct confirmation. Unavailable Trash never triggers
permanent deletion. Windows uses the dedicated Shell recycle method, which can
reject paths beyond its own limit even though rename/create/permanent operations
support long paths. Linux initially supports same-device home Trash only and
requires Linux 5.6+ openat2. Destructive traversal never intentionally follows
symlinks/junctions/mount boundaries. These are live operations, not atomic snapshots;
cancellation retains completed changes. Normal window close requests cancellation
and waits responsively for terminal job events before exiting.

The Windows Shell recycle API remains path-based inside its native call; a
concurrent namespace substitution there cannot be excluded. Linux private staging
narrows pathname races but is not a boundary against the same UID, root or previously
opened descendant handles. See [Windows guarantees](crates/midnight-platform-windows/MUTATION_SAFETY.md)
and the Linux mutation module's safety comments. No claim of universal race freedom
is made. Tests mutate fresh, owned fixtures only; they never empty a user's Trash.

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
  pane's captured working directory. Inserted paths are literal arguments:
  simple Unicode paths remain bare, other tokens use shell-specific escaped
  double quotes, without surrounding apostrophes. PowerShell numeric names and
  smart quotes are preserved literally. CMD refuses unsafe percent/quote/line
  break text and quoted paths ending in backslash (builtin/CRT ambiguity);
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
  mark access/resource failures as partial. Completed partial values retain `≈`
  but lose the active scan's amber/bold style, so denied access cannot look like
  endless computation. They are live estimates, not atomic
  snapshots: the current path-based read-only interface cannot exclude races
  with concurrent path replacement. This interface must not be reused as a
  security contract for future destructive traversal.
- Folder scans share weighted capacity across panes. Only a positive local SSD
  hint allows up to four I/O streams; HDD/network/unknown takes exclusive scan
  capacity. Hints are conservative and do not prove immutable physical topology.
- Preferences use bounded strict versioned decoding and opened-file validation.
  Version 8 includes tabs, panel modes, bookmarks, fonts, command history, shortcut
  overrides and panel filters, with v1–7 migration. Shortcut settings accept only known semantic action
  IDs and bounded key chords, never executable commands. Duplicate, malformed and
  overlapping bindings fail validation atomically. Native editor/IME chords cannot be
  reassigned to broad application actions.
  Its 4MiB aggregate bound is a settings resource limit, not a path-length limit.
  Native path encodings are lossless and platform-specific; malformed/foreign
  encodings, duplicate IDs/slots and NUL paths are rejected. Late initial loads
  must preserve unrelated saved data when local edits already occurred.
  Native readers reject symlinks/reparse points and special files rather than
  blocking on FIFOs or opening their targets. Own exclusive temp creation and
  atomic publication preserve existing settings on failure. File sync is used;
  no power-loss durability of the parent directory entry is claimed.
- Command history intentionally persists exact user-submitted text only after a
  successful terminal launch, never command output or passwords collected by Midnight Explorer.
  Commands can contain secrets; the user authorized persistence and can disable
  it or clear history in Options. Keep at most100 distinct strings and1MiB of
  aggregate text without truncation; this is a retention budget, not an execution
  limit. Deduplicate by exact text across shells; recall restores compatible
  grammar and standard privileges, never executes and never restores elevation.
  History is not written to diagnostic logs.
- Status-field clicks copy only the current displayed value via a bounded backend
  clipboard service. A dedicated lazy worker retains native clipboard ownership;
  the UI never opens the OS clipboard. Clipboard managers may retain copied data.
  Breadcrumb/history/detail tokens reject stale clicks after navigation or updates;
  displayed paths are never parsed to construct a clicked filesystem identity.
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
  coalesced to 250ms without throttling input or error delivery.
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

## Verified clipboard copying and the copy contract

Ctrl+C publishes lossless native paths; Ctrl+V captures the destination path and
queues verified copying. Native identities are opened and pinned when that job
starts. An external rename before then is not a snapshot of the original namespace.
Clipboard payloads are untrusted: at most 4096 paths and 4MiB, with whole-request
rejection of malformed/NUL/relative data. No shell command parses a filename.
Copying is serial and conservative on all storage classes in this milestone;
one copy job and one native I/O step run at a time across the copy queue, including
source reads, destination writes and independent verification reads. Cancellation
retains capacity until the in-flight native call and cleanup actually return.
Future parallel Copy/Move is permitted only when **both source and destination
are positively identified as local SSDs**, with a shared physical-device scheduler.
HDD, network, unknown or failed classification on either endpoint requires serial
copying; an SSD at the other endpoint does not override this. SSD-to-SSD copying
also remains serial today. Move remains unavailable.

Source files remain untouched. Move/Cut intents are explicitly unsupported.
Files and ordinary directories are supported; links/special files are rejected.
Existing items are never replaced or merged. Conflicts show source and target
metadata and accept versioned Skip, Skip all for this job, another name, or Cancel.
Waiting for a decision holds no blocking thread; Jobs can reopen a deferred dialog.
Already published files/directories remain after cancellation or partial failure.
The private unfinished file is cleaned by its native handle/descriptor.

Linux uses anonymous O_TMPFILE staging and descriptor-based no-replace publication;
filesystems without that capability return Unsupported. New copy directories are
private (0700). Creation rejects destination parents owned by an untrusted UID,
or shared-writable parents lacking sticky protection. mkdir/open is not an atomic
operation; same-UID/root interference remains outside this isolation boundary.
Windows uses exclusive staging and a retained handle for no-replace rename.
Content verification does not promise preservation of ACLs, alternate streams,
xattrs, sparse allocation, timestamps or links. Windows file sync does not promise
power-loss durability of the parent entry; Linux separately reports parent-sync
failure even if publication succeeded.

1. Only supported ordinary file types are copied. Opened file identities and
   source/destination alias checks are used; no unchecked traversal of recursive
   links, junctions, named pipes or devices is permitted.
2. Use an exclusively created temporary destination on the target filesystem.
   Handle short reads/writes, propagate errors and retain ownership through
   outstanding I/O and cancellation. Never delete the existing target first.
3. Hash the exact source bytes read, using SHA-256 in this implementation. Complete and
   synchronize the writes, then perform a separate full read of the destination.
   Compare both lengths and cryptographic digests. Hashing the outgoing buffer
   twice, checking timestamps or comparing sizes alone is insufficient.
4. Reject any detected source change. Source metadata checks do not prove a
   point-in-time snapshot against concurrent writers; a stronger snapshot promise
   requires supported write exclusion or filesystem snapshots. In particular,
   same-length rewrites within one filesystem timestamp quantum can retain both
   modification and change times and evade metadata revalidation. The digest
   proves the destination matches the source byte stream that was read, not the
   source's later contents. Report what was
   actually verified, never an unsupported stronger guarantee.
5. Publish only after successful verification, using the target platform's tested
   no-replace/replacement contract. Unsupported atomicity must cause an explicit
   failure, not a silent destructive fallback. Preserve the existing destination
   when any pre-publication step fails.
6. Distinguish verified content, namespace publication and power-loss durability.
   A cached readback is not necessarily a physical-media reread. Hashes do not
   establish ACL/ADS/xattr/link preservation; those have separate declared rules.

Multi-file Copy/Delete requests share the engine's mutation exclusion. Capture
selection and destination paths when accepting a request; later navigation
must not retarget it. Enqueueing stays nonblocking while another job runs.
Serialize conflicting paths/aliases and prevent a queued destructive dependency
from deleting a source after its preceding copy failed. Per-job cancellation,
phase progress and partial results must stay truthful; a completed write is not
a completed verified copy.

Windows context menus run on a bounded STA with an independent native owner and
message loop. The owner never joins the UI thread's input queue. Canonical copy,
cut, paste, delete and rename verbs return to host policy before executing; other
explicitly chosen extension verbs remain native Shell actions. Shell extensions
are in-process OS integrations, not sandboxed plugins. Linux has no equivalent
universal context-menu API and uses an application action menu.

Windows file clipboard uses CF_HDROP and Preferred DropEffect without lossy
canonicalization. Linux X11/XWayland uses raw-byte file URIs with GNOME/KDE intent,
bounded incremental transfers, and an owner pumped by the backend worker.
Native Wayland file-copy ownership is unavailable until a bounded writer is
implemented; a usable XWayland bridge is preferred. Native Wayland file-list reads
remain bounded. No per-copy detached ownership threads may accumulate.

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

Prefix search always uses Unicode case folding on valid original names, never
lossy display replacement text. Search is literal, without regex/glob/markup
interpretation. Matching and projection/index construction run on bounded backend
workers against an immutable accepted source; navigation, source and query
revisions are checked after output backpressure and again in the UI. Filtering
retains the canonical directory for watches and size scans. Selection/Properties
use the visible projection so hidden entries cannot become action targets.
Native IME preedit is not submitted as a completed query. Filesystem paths and
names are never reconstructed from highlighted text.

Compression indication reads native metadata only, without opening contents or
following the final link. Windows uses the compression attribute; Linux statx
requires the compression support mask and avoids final-link/automount traversal.
Unknown support does not mean uncompressed. Sparse allocation, archive suffixes
and apparent compression ratios are not evidence; reported sizes remain logical.

Directory links and junctions retain a separate entry kind. Native Windows link
icons use system overlays on cached generic types, without sending untrusted
link paths to the Shell or opening targets. Linux does not infer a directory
target by an implicit stat/network probe for icon presentation. Recursive sizing
continues to accept only ordinary directories; a folder-looking icon must never
turn a link into a traversable directory.

Tests include Czech, Chinese/Japanese, Arabic/Hebrew, supplementary-plane emoji,
combining sequences and long Windows paths, plus raw non-UTF-8 Linux names.
Storage/navigation tests do not establish visual shaping: the supported renderer
and available font fallback must also be checked with multilingual native output.
FemtoVG-WGPU is the normal renderer. Slint's documented western-script limitation
means its separate software renderer remains a developer diagnostic path until
it is independently qualified; it must not be advertised as a fully Unicode
production fallback.

## Directory comparison, filters and metadata editing

Directory comparison reads captured directory metadata on a bounded worker. It
does not verify file contents. Both selections are published together only while
their source identities, navigation, filters and selection revisions still match.
The resulting selection becomes the selection worker's authoritative input;
subsequent toggles or refreshes must not revive a pre-comparison selection.

Saved panel filters and temporary prefix searches share one backend projection.
Hidden rows cannot be operation targets, including the old caret when the result
is empty. Raw names that are not valid Unicode remain visible. Filter expressions
are limited to 4,096 UTF-8 bytes, 64 wildcard alternatives and a 2 MiB compiled
regex budget. Rust regex matching does not use unbounded backtracking.

Attributes, timestamps and filename case changes require an explicit preview and
confirmation of its exact revision. Preparation captures native object identities;
each OS adapter revalidates them and applies changes through pinned handles.
Recursive traversal does not follow links or cross mounts. It snapshots child
names before changing them and processes children before their parent. Selection
and traversal share a 256-target pin budget across pending metadata jobs; depth
is limited to 64 and each name snapshot to 4 MiB. Resource exhaustion is reported
as an error, not as a successful truncated operation. These are operation resource
budgets, not path-length limits. Preview and result samples contain at most 32 items.

Filename conversion uses full Unicode case mapping with the root locale while
preserving untouched native bytes/code units. An undecodable transformed component
is rejected. Existing destination entries, including another hard-link name, are
never replaced. Collision decisions are bound to a live job and monotonically
increasing token; stale replies cannot authorize a different item. Skip affects
one item, Skip all covers later collisions, and Cancel stops remaining work.

Metadata edits are not transactions: cancellation or an OS error can leave earlier
items or fields changed. Per-item results record actual changes, including changes
completed before cancellation. Results are delivered after the native worker
releases its mutation gate. Hard-link effects are disclosed in the preview.
Windows encryption is displayed only; this editor does not perform EFS mutation.
Linux creation timestamps are not editable. Unsupported operations fail explicitly.

## File drag and drop

Incoming and internal file drops enter the same copy service as clipboard paste,
with independent SHA-256 destination readback and no source deletion. Outgoing
drags offer Copy only; the receiving application owns the copy and its integrity
checks. A native "accepted" response means admission to the queue, not a completed
or verified copy. Drag/drop never grants overwrite, Move or Link permission.

The native callback validates its captured destination/navigation/geometry against
the current shared snapshot and immediately reserves actual capacity in the same
32-job copy budget before acknowledging success. The immutable request keeps that
permit until the UI supplies its normal operation ID and the backend enqueues it.
Queue saturation, stale destinations and failed delivery are rejected. Shutdown
and rejection release reservations; neither can leave an unbounded pending queue.

Pending drop admissions retain their ordering relative to later clipboard copies.
Cancelling one before backend enqueue rejects its token rather than sending a
CancelCopy that could arrive before the job exists. Transient native grab failures
must not permanently disable a healthy session or revive an ended gesture.

Payloads retain raw UTF-16 on Windows and raw filename bytes through X11 URI
encoding. The existing 4,096-path / 4 MiB payload limits apply; malformed and
non-local URI lists are rejected as a whole. Native serialization/admission never
canonicalizes paths or probes filesystem objects on the UI thread. The copy worker
performs its normal identity and containment checks. Dragging leaves clipboard
ownership untouched and never submits a terminal command.

Source preparation captures a selection revision and runs on a bounded worker.
Delayed preparation cannot revive a released/cancelled gesture. Native window
lifetimes and shared drop regions are invalidated synchronously before hide/close;
navigation, modal UI, resize and DPI changes invalidate stale regions. Input-grab
release belongs to narrowly scoped, per-OS UI bridges; COM/X11 data transfer runs
on the thread-affine backend actor. Windows COM sources/targets are non-agile so
calls remain in that actor's message-pumped STA.

Linux uses a verified worker-owned XDND proxy. The host window's XDND advertisement
is removed while the proxy is installed, preventing normal legacy sources from
using winit's lossy UI-thread parser. The proxy advertises XDND v5; native Wayland
external transport is unavailable until same-connection seat/data-device support
exists. Internal panel dragging remains available. X11 incremental transfers are
bounded and timed out. Move proposals may negotiate Copy only when the source
explicitly offers Copy; a Move-only offer is rejected.

## Dynamic viewers and planned filesystem plugins

The [plugin interface design](docs/superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
defines the implementation boundary. Basic text/binary, source and Markdown viewers
are separate removable libraries under the installation's `viewers/` root. The
`filesystems/` providers and migration of the OS-native filesystem remain planned.

- Native plugins run with application authority. Neither a C ABI nor capability
  validation isolates malicious code, memory faults or a stuck native parser.
  A hard isolation guarantee would require a separate process/sandbox.
- Installing a package is a trust decision. Discovery reads bounded manifests
  before loading code, checks platform/ABI/identity, and never searches the active
  panel, working directory, PATH or an archive for executable extensions. Native
  dependency lookup is constrained by separate platform loader implementations.
- Discovery and lazy loading reject untrusted ownership/write permissions on the
  package and unsafe replacement permissions on its ancestors. Windows uses native
  DACL/owner checks; Linux uses native uid/mode and sticky-directory rules. A private
  child inside a replaceable parent is insufficient. This does not turn trusted
  native code into a sandbox or authenticate a publisher's identity.
- Plugin projects share one Git repository but only depend on the public API/SDK.
  Their independent Cargo workspaces and CI jobs
  contain their tests and have no write access to the host application's sources.
  Exported-plugin CI runs without host sources, using the released public SDK.
  Building, testing or installing a plugin must never patch host sources or the
  executable. Integration tests consume a released host binary in a temporary
  distribution; extending host API requires a separate host-owned change.
- Host-owned leases scope content services; no Rust allocator, future, Tokio
  runtime or Slint object crosses the ABI. Buffers have explicit ownership and
  release functions. Loaded code outlives every worker, callback, session and
  buffer. Version 1 uses restart-based installation/removal, with no hot unload.
- Text viewers return bounded Unicode text and semantic style spans, never
  executable markup or source-directory grammars. Parsing and decompression run
  on bounded backend workers; cancellation is cooperative, not a promise to kill
  a blocked thread safely.
- The native content broker provides LiveValidated reads, not a filesystem snapshot:
  it prepares identity, opens without following links, and checks revision before
  and after reads. Replaced/changed files report an error. Reads are bounded to
  256 KiB and use u64 offsets. Four live sessions share the host limits; decoded
  pages, wire items, retained service replies and raster surfaces have separate
  explicit budgets. Selection/copy is limited to the loaded page.
- Markdown returns semantic text blocks and inert link metadata. The host does not
  execute HTML/scripts or load remote images, styles, fonts or links. Source syntax
  definitions are packaged code, never executable configuration from the opened folder.
- Archive browsing is read-only. Names, links and declared sizes are untrusted;
  actual decompression, dictionaries, indexes, nesting and cache use share
  budgets. Listing never extracts a member to its declared host pathname.
  Future extraction/copy uses the common destination readback/hash/publish
  contract. ZIP/GZIP/7z checksums do not substitute for that verification.

## Dependencies

Keep `Cargo.lock`, check advisories against the actual lockfile, and record the
database revision and warnings. A clean advisory scan means no matching known
vulnerabilities in that database; it does not prove the absence of unknown bugs.
Do not adopt source or dependencies merely because they look similar to reference
code. Salamander code/assets are not copied into this implementation.
