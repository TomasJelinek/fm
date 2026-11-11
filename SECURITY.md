# Security and integrity requirements

Security, lossless Unicode and explicitly verified copying are product requirements.
Full SHA-256 verification is enabled by default. The user may explicitly choose
size plus three 64 KiB samples (first/middle/last; files up to 192 KiB fully compared).
This weaker mode can miss corruption between samples; it never disables identity,
source-change, I/O-error, sync, overwrite or publication checks. Jobs freeze the
selected policy at admission and report which guarantee was checked. Native
same-volume Move renames without reading contents. See the
[current transfer contract](docs/superpowers/specs/2026-11-10-transfer-verification-and-native-move.md).
UI code handles input and presentation only. Filesystem
access, large sorts/indexes, checksum work, large-object reclamation and diagnostic
file encoding/writes execute in backend workers.

Copy stages are owned temporary objects on the destination filesystem: Windows uses
an exclusive, directory-handle-relative `.me-` plus sixteen random hex characters;
Linux uses unnamed `O_TMPFILE` inodes. Names are not operation authority. Cancellation
waits for in-flight native I/O, then explicitly removes unpublished partial data
before terminal progress. Cleanup failures must remain visible with their residual
path, including when cancellation or construction failure was the original cause.
Existing destination originals, unfinished sources and earlier completed files are
preserved; a filesystem refusal cannot be reported as successful cleanup.

The optional Chromium HTML viewer has a separate browser-process profile. CEF's
renderer sandbox remains enabled; the decoder's single-process policy is not
weakened. HTML starts offline with scripts disabled. Enabling online mode revokes
all local relative resources and recreates the browser/context; scripts require
explicit online permission. Captured main-document content can then be read and
transmitted by scripts. Local paths, shell execution, downloads and permission
prompts are unavailable to web content. The backend owns private session state,
resource containment, process-tree budgets and cleanup. See the
[interactive viewer contract](docs/developer/WEB_VIEWER_API.md).

Checksum manifests are untrusted input. The checksum worker bounds encoded input,
decoded text and retained rows, validates the whole document, and opens only named
regular native files beneath the manifest's pinned directory. Absolute paths,
parent traversal, aliases, malformed hashes and duplicate entries fail validation.
Windows/Linux readers reject link ancestry and cross-device descendants and check
file identity/revision around reads; changed or unreadable sources never become
Verified. Calculation and verification remain cancellable, bounded background jobs.
Native Open/Save choices only fill a draft. Saving uses exclusive staging and
independent SHA-256 readback; it never silently replaces an existing manifest.
SFV/CRC32, MD5 and SHA-1 support compatibility, not authenticity. A matching checksum
does not establish the origin or trustworthiness of the manifest itself.

## Captured inline-tree transfers

F5/F6 capture exact acknowledged tree occurrences, an immutable Copy/Move intent
and provider registrations before the dialog. Display paths and row numbers are never reinterpreted as operation
authority. Four shared capture permits bound live drafts/in-flight ownership;
confirmation separately enters the ordinary 32-job copy queue. Native identity
is pinned during normal preparation and retained across Retry, not frozen at
dialog creation. Archive revisions are checked through retained provider sources.
Duplicate/overlapping roots and native destination ancestry are rejected before
writes. The frozen verification policy and ordinary publication guarantees apply
to copied data; same-volume native rename reads no file contents.
Only successful root completions may remove their exact session's marks; delayed
events cannot affect another navigation. Rejection, cancellation and shutdown
release final provider ownership on backend workers. F6 accepts only native
captures and uses native rename when qualified on the same volume. Cross-volume
or merged children use verified per-file Move and postorder empty-directory cleanup. Mixed/virtual
Move roots fail before source preparation or destination mutation. Native pins
are dropped before their retained provider lease, including failed preparation;
stage-cleanup tickets retain the provider without retaining source handles.

Terminal transfers request read-only reconciliation of the native root and its
already loaded native branches. Each listing and its application share the
mutation gate, so temporary Move staging cannot become a published tree. Original
admission snapshots remain immutable; current root file actions use a separate
backend-built snapshot with validated identities. Failed listings retain known
rows and show an error. This does not grant authority to display paths or enable
unimplemented tree deletion or other destructive consumers. Native tree clipboard
publication uses the separate captured-source contract below; virtual exports
remain unsupported.

Native manual refresh and batched filesystem notifications share that maintenance
path. Only exactly acknowledged provider-resolved root/expanded-branch targets
enter the scope; links and foreign providers are excluded. Two backend workers
own bounded native batches and fair serial metadata polling. Unchanged commits
reuse admission results, with paced failed retries. Successful reconciliation and
its exact Commit acknowledge only captured dirty epochs, preserving later events.
Navigation and terminal failure invalidate the old generation before asynchronous
teardown; old callbacks, receipts and failures cannot act on newer navigation.

## Network Neighborhood discovery

Discovery runs only on bounded backend workers through a separately installed
filesystem plugin. Device captions, service URIs and opaque discovery addresses
never become native operation paths. Listing, metadata, restore and hover do not
activate shares or prompt for credentials. Explicit activation must return a
validated, OS-supplied absolute native path before ordinary file operations are
enabled; failed activation preserves the discovery view. Known native share
paths do not depend on discovery availability.

No subnet/port scans or stored passwords are used. Cache entries, leases, worker
slots and result pages have shared resource limits. Canceled or timed-out native
calls retain their worker permit until they really return; cancellation does not
spawn replacement workers or terminate OS threads. See the
[network contract](docs/developer/NETWORK_FILESYSTEM.md) for platform and current
authentication limitations.

## Current browsing and explicit-operation milestone

Rename/F2, create-directory/F7 and deletion are explicit, confirmed operations.
Preparation pins native identity before showing the confirmation; navigation never
retargets a queued job. The backend permits at most 32 preparing/prepared/queued/
running mutation jobs and 32 copy jobs, two mutation preparation workers and one
active mutating job across both queues. Copy admission is FIFO, including native
drop reservations awaiting UI handoff. Existing-entry mutation confirmations wait
for all copies still outstanding at confirmation before acquiring the shared gate;
an incomplete predecessor rejects the mutation for explicit review. Later copies
and creation jobs are independent. Native handles
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

- Ordinary browsing does not copy, rename, delete, execute files, invoke a shell,
  request elevation or interpret filenames as code. A failed explicit native folder
  navigation can offer **Retry with authorization**. Only that explicit action
  consumes a backend-retained denied-navigation receipt and invokes the OS prompt
  for one read-only directory listing in a separate helper. Refresh, metadata and
  background tree probes never prompt. The helper loads no plugins, changes no
  ACLs, stores no passwords, and grants no file mutation or content-read authority
  to the host. Native operation capture/revalidation remains unelevated.
  See [the bounded helper contract](docs/verification/2026-11-11-directory-authorization.md).
  Explicit navigation into a symbolic
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
  pane's captured working directory. At the user's explicit request, path/name
  shortcuts insert raw text with no generated quotes, escaping or separator
  spaces. Actual filename spaces and shell characters are preserved. This is
  editor insertion, not a shell-literal guarantee: the whole draft is intentional
  shell syntax only when explicitly submitted. Names that cannot be represented
  exactly or contain control characters are rejected unchanged. Command text
  is not logged. Editing never launches a terminal.
  Nonempty drafts are bound to their shell grammar: a late preference or picker
  change cannot reinterpret an inserted PowerShell path as cmd code. Clear the
  draft before selecting an incompatible grammar. Explicit POSIX elevation keeps
  the same grammar and still requires Run/Enter; passwords belong only to sudo's
  own terminal interaction. No password or elevated-mode preference is stored.
  No terminal is started by selection, navigation, icons or path insertion.
  Native Properties is also an explicit host-UI action. Unsupported native
  properties fall back to read-only metadata rather than opening/executing files.
- Read-only native size scans follow directory links through OS-resolved targets
  and stable volume/file identities. An active ancestry set stops cycles before
  recounting contents; independent noncyclic aliases retain logical totals.
  File links/special-file contents are skipped. Resolution acquires the existing
  bounded I/O capacity, and ordinary directories retain the compatibility path
  for providers without identity resolution; such providers cannot follow links.
  Size scans use checked totals and
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
  Supported schemas through version 25 include window geometry, monitor identity
  and a validated integer panel split ratio,
  typed Network tabs, panel modes, native and archive bookmarks, fonts, command history, shortcuts,
  filters, editor, viewer, archive, preview size and filename-color choices. Earlier
  schemas migrate with explicit defaults; unsupported fields and versions fail
  atomically. Viewer options store presentation choices, never query text,
  document contents or per-file reading positions.
  Shortcut settings accept only known semantic action
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
Adopted native trees capture the exact acknowledged projection and selection,
or the focused occurrence when no items are marked. Parent/context rows never
grant clipboard authority. Copy/Cut share four capture permits with F5/F6, retain
provider registrations and pass an opaque `CapturedClipboardFiles` source to the
bounded native clipboard worker. That worker resolves and revalidates every
native target after opening the native clipboard, checks cancellation and intent
before publication, and retains ownership through the OS write. Navigation cancels
pending work; it cannot retract a native write that already completed. Rejection,
stale replies and shutdown retire captured owners on backend workers.

Native clipboard entries are raw names, not frozen native identities or mutation
proofs. Duplicate/ancestor occurrences remain subject to ordinary Paste preflight.
Archive or mixed captures fail as a whole before publication; neither archive
display strings nor temporary extraction paths are exported. Native tree Paste
uses the captured panel root, not its focused child, and preserves the clipboard's
Copy/Move intent and ordinary Move confirmation. This grants no archive writes.

Copying is serial and conservative on all storage classes in this milestone;
one copy job and one native I/O step run at a time across the copy queue, including
source reads, destination writes and independent verification reads. Cancellation
retains capacity until the in-flight native call and cleanup actually return.
Future parallel Copy/Move is permitted only when **both source and destination
are positively identified as local SSDs**, with a shared physical-device scheduler.
HDD, network, unknown or failed classification on either endpoint requires serial
copying; an SSD at the other endpoint does not override this. SSD-to-SSD copying
also remains serial today.

Ordinary Copy leaves source files untouched. Untrusted Move clipboard intent
through the ordinary Copy route is rejected. Explicit native Move accepts at most
64 ordinary file/directory roots with a conservative 4 MiB active proof-path
reservation. Links, archive members, duplicate/overlapping sources and unsupported
locations are rejected before mutation. Qualified same-volume native Move uses
held source/destination identities and no-replace rename without content reads,
CRC or SHA computation. Whole-directory rename does not enumerate descendants.
Same-volume errors never silently fall back to copying. Proven cross-volume moves
and explicit directory merges use the per-file proof and postorder directory
contract below, with explicit live-namespace and durability limits. F6 consumes native tree
captures as well as flat selections; native tree Cut publishes Move intent through
the clipboard capture contract above. Cut itself removes nothing. Copy remains
available where Move is unsupported, without silent fallback.
Copy supports files and ordinary directories; links/special files are rejected.
Conflicts show source and target metadata and accept versioned Skip, Skip all for
this job, another name, explicit recoverable replacement, directory merge, or
Cancel when the provider supports that choice. All choices apply to this job;
directory merge still prompts for conflicting children. Every selected source
identity is protected against replacement, and Move protects its earlier verified
outputs too. Conditional identity replacement remains Unsupported: recoverable
replacement is a separately declared policy, never a silent weakening of CAS.
Waiting for a decision holds no blocking thread; Jobs can reopen a deferred dialog.
Already published files/directories remain after cancellation or partial failure.
The private unfinished file is cleaned by its native handle/descriptor.

Recoverable replacement qualifies local ext filesystems on Linux and local fixed
NTFS on Windows. Linux exchanges names and inspects the displaced object in a
private recovery directory; an unexpected object is retained and reported.
Windows captures the exact guarded original, then publishes without replacement:
this has an explicitly declared namespace gap. A new winner is never removed;
original and staged replacement data remain in protected recovery after a partial
commit. Structured outcomes preserve actual publication, recovery paths and known
identities even on errors. No recovery object is automatically deleted on Drop.

Linux uses anonymous O_TMPFILE staging and descriptor-based no-replace publication;
filesystems without that capability return Unsupported. New copy directories are
private (0700). Creation rejects destination parents owned by an untrusted UID,
or shared-writable parents lacking sticky protection. mkdir/open is not an atomic
operation; same-UID/root interference remains outside this isolation boundary.
Windows uses exclusive staging and a retained handle for no-replace rename.
ContentOnly verification does not promise preservation of ACLs, alternate streams,
xattrs, sparse allocation, timestamps or links. Windows file sync does not promise
power-loss durability of the parent entry; Linux separately reports parent-sync
failure even if publication succeeded.

Explicit RequireModifiedTime accepts at most 64 distinct native regular-file roots
within the 4 MiB proof-path reservation. It captures metadata from retained source
handles before data reads and preflights every source pin and owned destination
stage before content writes. After the selected content-verification gate, the exact stage
modified time is set, synced and independently read back without rounding
tolerance. Value-specific precision failures happen on unpublished stages after
data verification; earlier completed files may remain, but such a failure prevents
removal of that source and stops subsequent Move work. Unsupported directories, links and virtual sources never
silently downgrade to ContentOnly. Source write attributes are not requested;
Windows adds WRITE_ATTRIBUTES only to owned required-metadata stages.
Source/destination times supplement, never replace, the selected content checks
and fresh native read guards. Published targets are never rewritten to hide a mismatch;
late pre-deletion errors restore originals without replacement. Access/creation time, attributes,
permissions, ACLs, xattrs, streams and link preservation remain separate features.

Copy-fallback Move preflight obtains source mutation authority separately from Copy,
retains every actual source reader and empty stage, and verifies native source
parents are protected before copying the batch. Windows requests DELETE upfront
only for Move and rejects incompatible sharing; recovery/copy-fallback qualification
also rejects unsafe parent DACLs. Metadata-only native rename preparation does not
require content-read or directory-list rights. Linux
requires owned/trusted-sticky source parents and effective mutation permissions.
Shared inherited directories can be unsupported even on a supported filesystem;
the failure suggests Copy and retains every source rather than weakening rights.

The following proof rules concern copied source removal, not native rename.
Only complete successful publication with every required durability step yields
a Move proof. Per the explicit user policy, each file finishes before the next
copy: full mode uses a bounded buffer to independently rehash its held source and destination;
explicit quick mode instead compares exact length and three 64 KiB samples,
or the whole file at or below 192 KiB. Both modes check native revisions/required
metadata and exact destination binding before temporarily staging that source
under its raw basename in a protected sibling.
Windows refuses hardlinks, readonly files and any alternate stream not verified
by Copy. A read-oplock probe rejects existing writable mappings; the original
Move handle continues denying write sharing. The final source/destination
validation retains fresh read guards through the explicit finalization check;
full mode hashes all bytes, while quick mode uses the same declared samples.
Exact-handle POSIX delete disposition uses image-section checks and no readonly
override. All application DELETE/source owners close, then empty-stage cleanup
runs before the next file. External readers may retain bytes after namespace
removal; no measured physical
space or parent-entry power-loss durability is promised.

A skip retains that source. Every pre-deletion failure or cancellation after
staging attempts a no-replace restore with an independent cleanup token. A new
original-name winner is never clobbered. Earlier completed moves stay completed and
published destinations are never rolled back. Drop never finalizes retained data.
Success retains no source copy or empty temporary stage. Failed restoration or
empty-stage cleanup preserves all remaining data and reports exact exceptional
paths. Cleanup never recursively deletes unexpected entries. Structured outcomes
distinguish removal from residuals and durability uncertainty. Linux uses no-replace
temporary staging and live-validated directory-relative unlink for owned,
single-link regular files on qualified local ext filesystems with protected
parents. A read lease is retained from preflight through final content validation
and removal. Existing writers/mappings or observed lease breaks, revisions or
binding changes refuse removal. A bounded dedicated thread blocks only its own
SIGIO and owns thread-directed lease notifications; no process-wide handler is
installed. The kernel can forcibly break a lease after its timeout, so arbitrary
descheduling between the final check and unlink across that timeout is not an
atomicity guarantee. See [the native race evidence](docs/verification/2026-11-08-ordinary-move.md).
Unsupported Linux locations fail before publication without fallback. No platform promises an
immutable snapshot or batch rollback. Windows ADS sharing/oplocks apply per stream;
late qualification refuses unseen streams but does not claim whole-file CAS or
isolation from malicious same-user/admin mutation. Raw residual mappings and
changed parents are reported with bounded display samples and exact counts.
There is no selectable source recovery product, catalogue or journal. Recursive
copy-fallback/merge Move grants each child separate source-removal authority through its
held parent. Verified regular files are removed individually; directories are
removed postorder only after fresh source/destination identity checks and an
empty-directory-only OS operation. Skips retain the affected source ancestors.
Overlapping selected roots are rejected before writes. Traversal is bounded to
64 roots, 128 active directory frames, 100,000 entries and a 4 MiB active path/proof
budget. Links, mount crossings and generic provider Move remain unsupported.
Directory identity validation is live on Linux, not atomic compare-and-unlink;
see [recursive Move evidence](docs/verification/2026-11-08-recursive-move.md).

1. Only supported ordinary file types are copied. Opened file identities and
   source/destination alias checks are used; no unchecked traversal of recursive
   links, junctions, named pipes or devices is permitted.
2. Use an exclusively created temporary destination on the target filesystem.
   Handle short reads/writes, propagate errors and retain ownership through
   outstanding I/O and cancellation. Never delete the existing target first.
3. By default hash the exact source bytes using SHA-256, synchronize writes and
   independently read/hash the entire destination; compare length and digest.
   Explicit quick mode instead captures first/middle/last64 KiB during source
   streaming and compares those bytes after sync, plus exact length. Up to192 KiB
   is compared entirely. Checking only size or hashing an outgoing buffer twice
   does not satisfy either policy. Native same-volume rename copies no data.
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
from deleting a source after its preceding copy failed. Confirmation captures
bounded completion receipts for all currently admitted copies, including native
reservations. Only fully completed, verified predecessors permit the mutation;
failed, cancelled or skipped work requires review and a new confirmation. No
blocking wait runs on the UI or holds the I/O gate. Per-job cancellation,
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

Incoming native drops enter the same Copy service as clipboard paste. Internal
folder drops also support Move through the ordinary transfer service: the backend
resolves exact entry/provider captures and volume evidence, never UI path strings.
Automatic drops move on a proven same volume and otherwise copy; Ctrl forces Copy,
Shift forces Move. The verification policy is frozen at submission. Native rename
does not read file contents; other moves verify each copied file before deleting
its source. Link effects are unsupported and overwrite still requires consent.
Windows outgoing OLE offers Copy|Move. Only the receiving app performs an external
Move; bounded performed/logical feedback never authorizes this source to delete
files. Explicit unoptimized source-cleanup requests report retained sources;
missing or inconsistent completion reports remain unconfirmed. Positive reports
are target claims, not our independent verification. Linux X11/XWayland outgoing
transport and native incoming transport on both platforms remain Copy-only.
The receiving app owns its transfer and integrity checks. A native "accepted" response means queue
admission, not completed or verified work. See the
[current drag contract](docs/superpowers/specs/2026-11-10-folder-drag-and-native-handoff.md).

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
calls remain in that actor's message-pumped STA. During DoDragDrop only, the actor
attaches to the validated same-process window thread's input queue and detaches
on every exit. Arming the exact drag token precedes releasing UI pointer capture;
synthetic release callbacks cannot cancel that handoff.

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
`filesystems/` contains independent archive workers and separate Windows/Linux
native libraries. Native panel Browse/Stat/Storage and native F3 capture require
the installed native provider, with no built-in fallback on provider failure.
Writes, watches and other explicit host services have not yet migrated to generic
provider capabilities. The native host shares the same protected installation
checks as the viewer host; optional ABI1.1 volume details preserve unavailable
fields and conservative physical-device classifications.

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
- Shared Windows host builds can resolve a private runtime published by explicit
  deployment. Its protected per-user registration and target executable must match
  the running host's SHA-256. Registration and target paths reject reparse points
  before canonicalization and retain the same ancestor/owner/DACL policy. Reads use
  handles denying concurrent writes and renames, with bounded lengths and revision
  checks. No package is copied or trusted automatically at startup. The process
  caches the resolved location; existing discovery/load checks still validate code.
  The hash matches builds; it does **not** authenticate a publisher or make an
  already launched shared executable safe. See [runtime discovery](docs/developer/RUNTIME_DISCOVERY.md).
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
  Copying out uses the common destination readback/SHA-256/publish contract.
  Opaque provider IDs and raw member names are captured; names must be safe
  single destination components, never joined archive display paths. Links,
  cycles, stale identities and resource overflows fail explicitly. Existing
  destination entries are not silently replaced. ZIP/GZIP/7z checksums do not
  substitute for destination readback verification.
- PDF parsing/rendering uses a separately installed restricted worker with
  bounded source length, page count, viewport and process memory/CPU. The host
  brokers read-only source bytes; PDF scripts, external resources and automatic
  link execution are disabled. A malformed page cannot select executable code.
- File comparison captures native regular-file identities. Both text inputs
  have byte/line/deadline limits; image inputs use the restricted image broker.
  Rendered comparison surfaces and retained frames are bounded. Commands and
  labels derived from content never execute.

## Dependencies

Keep `Cargo.lock`, check advisories against the actual lockfile, and record the
database revision and warnings. A clean advisory scan means no matching known
vulnerabilities in that database; it does not prove the absence of unknown bugs.
Do not adopt source or dependencies merely because they look similar to reference
code. Salamander code/assets are not copied into this implementation.
