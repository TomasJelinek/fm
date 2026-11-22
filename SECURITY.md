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

The Chromium HTML viewer has a separate browser-process profile. CEF's
renderer sandbox remains enabled; the decoder's single-process policy is not
weakened. HTML starts offline with scripts disabled. Enabling online mode revokes
all local relative resources and recreates the browser/context; scripts require
explicit online permission. Captured main-document content can then be read and
transmitted by scripts. Local paths, shell execution, downloads and permission
prompts are unavailable to web content. The backend owns private session state,
resource containment, process-tree budgets and cleanup. See the
[interactive viewer contract](docs/developer/WEB_VIEWER_API.md).
When the dynamic loader stops the Linux browser before its first message (exit status
127: a shared library it needs is missing), the viewer names the missing libraries
instead of a stopped transport. The host finds them by reading, never executing, the
ELF dynamic sections of the viewer executable and of the libraries it needs, looked up as
glibc's loader looks them up (DT_RPATH/DT_RUNPATH, `/etc/ld.so.cache`, the default
folders): no `ldd` or loader runs, only regular files are opened (without blocking on a
FIFO), every read is bounded and at most 1,024 objects are read. A library name from
those files appears in the message only as letters, digits and `._+-` (anything else is
masked) and in the suggested install command only when it consists of them. The check
runs only after such a failure, so it can never keep a working browser from starting.
On Linux systems where AppArmor restricts unprivileged user namespaces, the optional
`midnight-explorer-html-viewer` profile grants only `userns` to one installation's
viewer executable and confines nothing else; there is one such profile per machine, so
installing it for another installation moves the permission there (the installer says
which installation lost it). Without it, or a setuid-root `chrome-sandbox`, the viewer
reports the missing permission and never starts Chromium without its sandbox. Where the
kernel itself denies user namespaces to programs without administrator rights
(`kernel.unprivileged_userns_clone` or `user.max_user_namespaces` is 0, confirmed by a
refused namespace probe), no profile could help: the viewer names that setting, says what
an administrator can change, and offers no profile. Its message suggests `sudo` with the
installation's `install-html-viewer-apparmor.sh` only when that script is no link and it
and every folder above it are owned by root and not group- or other-writable; for any
other installer, which a program running as the user could have replaced, it asks for an
administrator to install the package's profile instead. The script follows the same rule
for the `sudo` command it suggests itself, and the package README never suggests `sudo`
with a copy that the user can change. Installing the profile for a path that can be
changed without root extends that permission to any program placed there; the script
refuses a path through a folder that users other than its owner can write to (even a
sticky one such as `/tmp`), where another user could place a program at the viewer's path,
now or once the installation is gone. A group-writable folder still counts as writable
by its owner alone when its group is the owner's private group: the owner's primary
group, named like the owner, which no other account in `/etc/passwd` has as its primary
group and which lists no other member in `/etc/group` (as on distributions with user
private groups and a umask of 002, such as Debian, Ubuntu and Fedora). Only those local
files decide: a user known only to a directory service, or files that include one
(lines starting with `+` or `-`), have no private group. **Install…** below applies the
same rule (`scripts/install-html-viewer-apparmor.sh` and
`crates/midnight-elevation/src/sandbox_profile_linux.rs`).

The viewer can also install that profile itself, but only after an HTML document's
browser was refused for exactly this reason (a typed refusal, never message text) and
only when the user chooses **Install…** there. Nothing is checked, offered or installed
at startup, by packaging or for any other document. The button is offered only when
`/usr/bin/pkexec` and every folder above it are no links, belong to root and are neither
group- nor other-writable (the check the authorized listing uses; it is repeated right
before pkexec runs) and the installation's canonical path qualifies (below), including
that no folder on the way to the viewer (its own folder up to `/`, or a checkout's
`_build` up to `/`) can be written by users other than its owner, as the script requires;
otherwise the viewer shows the manual instructions above. Before anyone is asked, the
notice names the viewer path the permission is for and that one installation at a time
has it, and, where someone other than root can change a folder on that path (a per-user
installation, an extracted package or a checkout), warns that a program put at that path
could use user namespaces too. A backend thread, never the UI thread,
then runs exactly `/usr/bin/pkexec --disable-internal-agent /bin/sh -c '<script>'`
with `LC_ALL=C` as its only change to the environment, so that pkexec's own messages are
English, where the script is a constant compiled into the application
(`crates/midnight-elevation/src/sandbox_profile_linux.rs`). As root, that script only:
sets `PATH=/usr/sbin:/usr/bin:/sbin:/bin` and `umask 022`; reads the profile text from
its standard input into a `mktemp` file in `/etc/apparmor.d` (a dot name, which AppArmor
skips when it loads the folder); refuses an empty text; makes the file mode 0644; checks
it with `apparmor_parser -QTK`; renames it over `/etc/apparmor.d/midnight-explorer-html-viewer`
(`mv -T`, so a folder there is never written into); and loads it with `apparmor_parser -r`.
Its own failures exit 1, so 126 and 127 come only from pkexec: a dismissed, refused or
unobtainable authorization, which changes nothing. Install… then asks again, except when
pkexec found no authentication agent in the session (status 127 with its message "No
authentication agent found"): asking again could never work there, so the viewer
withdraws Install… and says why, with the manual instructions. Closing the window stops
an authorization that is still pending; a command that pkexec already runs as root cannot
be stopped, is waited for, and its own result counts. No file of the installation or source checkout is
executed, sourced or read as root, because the user may be able to change them: neither
`install-html-viewer-apparmor.sh` nor the viewer runs there. The unprivileged application
generates the text with the script's rules (a test compares it with the script's
`--print` output) from the canonical installation path, which may contain only letters,
digits, spaces and `._+-@:`; the attachment is quoted and the only `*` is the one the
checkout pattern `<checkout>/_build/*/viewers/…` adds, so a path can neither end the
quoted attachment nor add rules. Polkit authorizes that constant command, not the text,
and a program running as the user could ask polkit for any root command itself, so the
text gives such a program nothing beyond what the administrator authorizes. Unlike the
script, the button does not remove other files that define the same profile.

Checksum manifests are untrusted input. The checksum worker reads a manifest as a
stream of lines (its memory follows the entries, a short name and a fixed-length digest
each, never the file's size), validates every entry before opening any file, and opens
only named regular native files beneath the manifest's pinned directory. Absolute
paths, parent traversal, aliases, malformed hashes and duplicate entries fail
validation. Windows/Linux readers reject link ancestry and cross-device descendants and
check file identity/revision around reads; changed or unreadable sources never become
Verified. Calculation and verification remain cancellable background jobs without size
limits: a walk is iterative, enters a folder reached again (by identity) once, and
keeps compact rows (about 200 bytes per SHA-256 row).
A native Open choice only fills the Verify page's file field; choosing a Save destination
requests that save. Saving uses exclusive staging and independent SHA-256 readback.
It never silently replaces an existing manifest: only an explicit Replace answer
does, through recoverable replacement that keeps the old file beside it and reports
where.
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
path. Parent-first reconciliation includes already-loaded provider-resolved
native directory links and their loaded descendants; foreign mounts stay excluded.
Watches require visible, expanded occurrences from the exact acknowledged view.
Known physical browse identities deduplicate native notification slots; logical
aliases remain separate display, polling and action occurrences. Accepted identity
changes rebind watches; before/after polling identity mismatches request maintenance.
Two backend workers retain bounded native batches, fair serial polling and paced
failed retries. A failed parent suppresses descendant work and retains known rows.
Successful reconciliation and its exact Commit settle only captured dirty epochs,
preserving later events. Navigation or terminal failure invalidates the old
generation before asynchronous teardown; old callbacks and receipts cannot act on
newer navigation. This remains read-only maintenance, with no new writes, ACL
changes or elevation authority.

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

Rename/F2, empty-file creation/Shift+F4, create-directory/F7 and deletion are
explicit, confirmed operations. Empty-file creation accepts one basename and uses
exclusive creation. A name the panel lists as a file is asked about first, and only
the dialog's Replace file button (never a repeated or held Enter) replaces it: an
empty file is published through verified staging and recoverable replacement in the
folder pinned at preparation, the original is kept in a private recovery folder and
the completion reports its path. It never truncates a file or replaces a folder,
link or special entry.
Preparation pins native identity before showing the confirmation; navigation never
retargets a queued job. The backend permits at most 32 preparing/prepared/queued/
running mutation jobs and 32 copy jobs, two mutation preparation workers and one
active mutating job across both queues. Copy admission is FIFO, including native
drop reservations awaiting UI handoff. Existing-entry mutation confirmations wait
for all copies still outstanding at confirmation before acquiring the shared gate;
an incomplete predecessor rejects the mutation for explicit review. Later copies
and new-entry creation jobs are independent; an empty file replacing an existing
one waits like any other existing-entry change. Native handles
scale with the selected root count and traversal depth; resource failure is an
error, never permission to substitute a target. Duplicate operation IDs cannot
execute twice. Rename collisions require the shared, versioned overwrite prompt; explicit
replacement retains the displaced destination for recovery. Without consent, the
existing destination stays unchanged. F7 accepts explicit absolute or panel-relative
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

Deleting changes permissions only inside the tree the user confirmed. On Linux, a
folder of that tree (a selected folder or any folder below it) that the current user
owns and that lacks owner write or search permission gets both before its entries
are removed, or before a selected folder moves into staging or the Trash (moving a
folder changes its `..` entry). The change goes through a descriptor opened beneath
the parent without following links or crossing mounts and checked against the
traversal's identity, never through a path, so a link to a folder elsewhere never
changes that folder. A folder that stays (it still holds skipped items, its removal
failed, or the deletion stopped) gets its original mode back once it is where it
stays (a selected folder after it has moved back out of staging, which needs the
permission too), and a folder moved to the Trash keeps its original mode. If setting
the mode back fails (the filesystem turned read-only or failed), the folder keeps
owner write and search permission and the failure names it: it pauses the job like
any other failure, or is added to the reason the deletion stopped. A folder with the
set-group-ID bit whose group the user is not a member of is never changed, because the
kernel would clear that bit with any change of its mode: it is reported as
write-protected. A read-only file needs nothing (unlink depends only on its folder).
The folder that contains the selection, anything outside the
confirmed tree and anything owned by another user is never changed: a removal they
refuse pauses the job with Retry, Skip, Skip all errors and Cancel, and says why
("write-protected" or "owned by another user"). On Windows, NTFS's POSIX delete
ignores the read-only attribute without changing it, so read-only files and folders
inside the confirmed tree are removed and an item that stays keeps the attribute. A
filesystem without POSIX delete (FAT32, exFAT, some redirectors) cannot ignore it:
Delete clears Read-only on the entry itself, through a second handle to that entry
and never by path, and sets it again if the entry still cannot be removed; if setting
it again fails too, the entry stays without Read-only and the error that pauses the job
says so after the reason the entry stays. A file with other names (hard links), which
may lie outside the confirmed tree, is not changed: its removal pauses the job with the
same choices. Move applies the same
rule to the tree it moves (below).

A same-volume Move on Linux renames a folder into another folder, which changes its
`..` entry and so needs write permission on the folder itself. A folder the current
user owns that lacks owner write gets it only for these renames, through the
descriptor retained when the Move was prepared (its `/proc/self/fd` entry, since
`fchmod` refuses that path-only descriptor), never through a path, and gets its
original mode back as soon as the native Move returns, wherever the folder then is.
Entries leave a folder only while it is writable, so a Move, like Delete, changes
permissions only inside the tree the user moves: a folder of that tree (the selected
folder or a folder below it) that the current user owns and that lacks owner write
gets owner write through the descriptor the Move retained for it, never through a
path, only for each change that takes one of its entries out or puts one back (its
private stage, the capture of an entry after a cross-volume copy, the stage's and an
emptied folder's removal, an entry's own rename on one volume, a restoration), and
gets its original mode back as soon as that change returns. If setting it back fails
(the filesystem turned read-only or failed), the folder keeps owner write, and the
change reports that with the folder's name instead of passing over it: a capture keeps
its source for recovery and puts it back, and a moved folder or entry asks whether the
job goes on. A folder with the set-group-ID bit whose group the user is not a member
of is never changed, because the kernel would clear that bit with any change of its
mode: it is reported as write-protected, as before. The folder that contains the
selection is never changed: a Move out of one that does not let the item leave stops
at preparation, before anything is copied. Folders owned by another user, links and
any other item are never changed. On Windows a folder's read-only
attribute does not stop entries from leaving it, so nothing is changed there. A
prepared Move source is rechecked by identity, size and modified time before each
attempt; its permissions and change time may differ, because the user may fix them
to retry.

The Windows Shell recycle API remains path-based inside its native call; a
concurrent namespace substitution there cannot be excluded. Linux private staging
narrows pathname races but is not a boundary against the same UID, root or previously
opened descendant handles. Where a filesystem takes its permissions from mount options
(vfat, exfat, NTFS, CIFS without Unix extensions, FUSE), the staging directory must
still be owned by the user (the mount must map ownership to the user, as desktop
automounting does). It keeps the mount-provided permissions only if no other user can
write to it (group write only for the user's private group, without a POSIX access
ACL), or if they are exactly the permissions, owner and group of the folder that
contains it, when other users can already change that folder (for example a share
mounted with `dir_mode=0777`) and neither folder has a POSIX access ACL, so staging
never lets other users change more than that folder itself does; otherwise the
operation is refused. Permission bits protect only where the kernel enforces them:
FUSE mounts with `allow_other` but without `default_permissions`, and CIFS mounts
with `noperm`, exclude other local users only as far as their server does. A staging
directory refused after creation is removed again; one that cannot be removed is
reported with its path.
See [Windows guarantees](crates/midnight-platform-windows/MUTATION_SAFETY.md)
and the Linux mutation module's safety comments. No claim of universal race freedom
is made. Tests mutate fresh, owned fixtures only; they never empty a user's Trash. The
acceptance suite's Trash checks run in a child process whose `XDG_DATA_HOME` is a
private folder, so they neither use nor read the account's Trash, and a run deletes only
inside the run folder it created itself (docs/developer/ACCEPTANCE.md). On Windows the
suite never uses or reads the Recycle Bin (its Trash scenario is skipped there);
staged children get a private profile inside a private installation that
`scripts\acceptance.ps1` creates under the user profile for owner, SYSTEM and
Administrators only and removes afterwards, and the deny entries the suite adds with
icacls name only the account's own SID, only on entries below its run folder, and are
removed after each scenario.

When a Linux rename, Trash move or permanent deletion completed but its staging
directory cannot be removed afterwards (for example because another program created an
entry in it, or a network or FUSE filesystem keeps a pending name there), a Rename or
Delete job reports the target as completed with one issue that names the leftover
directory; a Properties rename still applies the item's other changes but counts the
item as failed (see [operation progress](docs/developer/OPERATION_PROGRESS.md)). The
directory is said to remain there only while it still has the identity it was created
with, and its contents are never removed. A change that did not complete and leaves its
staging directory behind names it in an error that is never treated as a name conflict
or cancellation. After a capture request that returned an error, the selected item
found at the staged name is restored to its name without replacing anything (if another
entry has taken that name, the item stays in the staging directory, whose location the
error gives); a staging directory whose staged name holds another entry is reported as
holding an entry that is not the selected item, and one whose staged name cannot be
inspected as possibly holding the item; neither is described as unneeded.

- Ordinary browsing does not copy, rename, delete, execute files, invoke a shell,
  request elevation or interpret filenames as code. A failed explicit native folder
  navigation can offer **Retry with authorization**. Only that explicit action
  consumes a backend-retained denied-navigation receipt and invokes the OS prompt
  for one read-only directory listing in a separate helper. Refresh, metadata and
  background tree probes never prompt. The helper loads no plugins, changes no
  ACLs, stores no passwords, and grants no file mutation or content-read authority
  to the host. Native operation capture/revalidation remains unelevated. It runs
  only from an installation that standard users cannot change; see
  [the directory authorization helper](#directory-authorization-helper).
  Explicit navigation into a symbolic
  link/junction follows OS directory-navigation semantics; enumeration and metadata
  inspection do not recursively follow links or open special-file contents.
- Explicit Enter or an unmodified left double click is a separate activation
  action: directories stay internal; ordinary files use the host association,
  while executable activation can run programs. Selection, hover, right click
  and refresh never
  activate files. Native metadata/launch and helper reaping stay on bounded
  backend workers, separate from navigation. Missing/special files fail with
  context; native symlinks are resolved for activation only after explicit
  activation. Linux listings label a link to a directory as a directory link
  from one target type lookup that opens nothing, does not mount an automount
  target and uses cached network attributes; automounts or unresponsive servers
  along the link's path can still delay that listing.
  Windows passes raw UTF-16 to `ShellExecuteExW(open)` with no command parameters;
  Linux passes ordinary documents and desktop launchers as literal raw paths to
  a fixed system desktop opener, without inherited standard streams. Files with
  execute bits are classified using at most four bytes from a nonblocking regular
  descriptor, with identity/revision checks before and after reading: ELF programs
  run directly; shebang scripts and `.sh` files run in an ordinary-user terminal.
  A fixed wrapper invokes the script's raw path as one positional argument, never
  filename-derived shell source, then opens the configured interactive shell.
  Legacy executable `.sh` files without a shebang use the normal POSIX command
  fallback. Unknown document types retain associations even if a mount supplies
  execute bits. If an executable's header cannot be read, execution permissions
  remain the kernel's decision. Execute bits are never added automatically and
  no automatic elevation is requested. Host association/path limits are reported
  faithfully.
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
  The terminal button's Ctrl+click (Ctrl+Enter or Ctrl+Space on the focused button,
  or the unbound "Open terminal as administrator" command) opens the selected shell
  elevated in the active pane's folder, with no command. Midnight Explorer itself
  never elevates for it; only the launched shell runs elevated, after the system's
  own consent. On Windows the Shell's `runas` verb (`ShellExecuteExW`) receives the
  absolute path of the system's PowerShell or cmd (cmd from the system directory,
  never from `PATH`), the shell's fixed arguments and the folder as separate fields,
  never a command line built from a path or name; Windows starts it only after UAC
  consent, and a declined prompt is a status message. A folder on a network or
  `subst` drive of the user's own sign-in, which elevated programs do not see, is
  refused before any prompt. On Linux the system terminal runs `/usr/bin/sudo` (or
  `/bin/sudo`, never from `PATH`) as an argument vector in that folder, and sudo asks
  for the password in that terminal; without sudo nothing starts.
  No terminal is started by selection, navigation, icons or path insertion.
  Native Properties is also an explicit host-UI action. Unsupported native
  properties fall back to read-only metadata rather than opening/executing files.
  On Linux that summary reads the entry itself without following a final link
  (`statx` with `AT_SYMLINK_NOFOLLOW`): owner and group, whose names come only from
  parsing `/etc/passwd` and `/etc/group` (no NSS module or service is loaded; other
  accounts show their numeric id), mode and times; a birth time is shown only when
  the filesystem reports one. A single folder's contents are then counted on a
  backend worker with the read-only listing interface: links inside are counted and
  never followed, other filesystems are not entered, ancestry identities stop
  cycles, and the pending-path frontier is bounded (16 MiB). The count is a live
  estimate that marks what it could not read; closing or replacing the summary
  cancels it, and it never blocks the UI thread.
- Read-only native size scans never follow links met inside the measured tree:
  file links, directory links and junctions are counted as skipped without
  resolving their targets, so aliases cannot double-count or escape the measured
  folder. Only a scanned root that is itself a directory link (a selected row,
  or any row of a full-pane scan without a selection) is resolved through its
  OS-resolved target. Stable volume/file identities and an active
  ancestry set still stop non-link cycles (for example bind mounts) before
  recounting contents. Special-file contents are skipped. Resolution acquires the
  existing bounded I/O capacity, and ordinary directories retain the
  compatibility path for providers without identity resolution; such providers
  cannot follow a selected root link.
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
  Linux recognizes local LVM/device-mapper and md stacks only when every backing
  leaf passes the existing physical ATA/PCIe NVMe checks. Partition normalization,
  cycle detection, cancellation, depth 32 and 128-node/128-slave budgets bound
  the sysfs walk. Any missing, remote, virtual, removable or unknown leaf keeps
  the result unknown; network preview opt-in is unchanged.
- Preferences use bounded strict versioned decoding and opened-file validation.
  Supported schemas through version 35 include window geometry, monitor identity
  and a validated integer panel split ratio,
  typed Network tabs, panel modes, native and archive bookmarks, fonts, command history, shortcuts,
  filters, editor, viewer, archive, preview size, archive-preview and filename-color choices,
  file comparator rules (version 33), the text rendering choice (version 34) and the
  hidden-files and selection-by-mask choices (version 35). Earlier
  schemas migrate with explicit defaults; unsupported fields and versions fail
  atomically. Viewer options store presentation choices, never query text,
  document contents or per-file reading positions.
  Archive previews have a separate default-on preference, while their network
  admission still follows the actual container storage classification and the
  network opt-in. Disabling archive previews immediately hides retained member
  pixels and rejects late archive frames; the next normal scheduling pulse removes
  their targets and requests backend cancellation. Source access and decoding stay
  on backend workers.
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
  it or clear history in Options. Keep at most 100 distinct strings and 1MiB of
  aggregate text without truncation; this is a retention budget, not an execution
  limit. Deduplicate by exact text across shells; recall restores compatible
  grammar and standard privileges, never executes and never restores elevation.
  History is not written to diagnostic logs.
- Status-field clicks copy only the current value of the clicked field, with raw
  names (see below), via a backend clipboard service. A dedicated lazy worker retains
  native clipboard ownership; the UI never opens the OS clipboard. Copied text has no
  size budget: the worker reserves its memory fallibly (also before the native copy),
  so a text too large for the memory left fails with an error instead of ending the
  process; NUL is still refused. On X11 a text longer than one request is served in
  incremental (INCR) chunks by the session's own owner window. Pasting text (a path)
  still reads at most 4 MiB.
  Clipboard managers may retain copied data. Breadcrumb/history/detail tokens
  reject stale clicks after navigation or updates;
  displayed paths are never parsed to construct a clicked filesystem identity.
- Raw `PathBuf`/`OsString` identities are kept separately from visible text.
  Valid Unicode is not converted through an ANSI code page or normalized into a
  different filename. Names that are not valid Unicode on their host remain
  addressable by their original raw paths. Names and paths shown in the main window
  (rows, tabs, the path bar, dialogs, errors and status text) and in viewer and
  comparator captions and status lines display the explicit bidirectional controls
  U+202A-U+202E and U+2066-U+2069 as `\u202E`-style escapes, so a name cannot
  reorder its shown extension. Operations,
  search, comparisons, copy actions for paths, details and errors, and the name and
  destination fields keep the raw name, while text selected and copied from a shown
  field, such as the path editor, keeps its escapes (see
  [dialog ergonomics](docs/developer/DIALOG_ERGONOMICS.md)).
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
  scoped native ownership; Linux theme PNG and SVG loading retains its trusted-root,
  encoded-size, decoded-allocation and lookup-attempt bounds, and theme SVG
  artwork has the structural bounds of chosen command artwork (UTF-8 only, no DTD,
  4,096 elements, bounded `<use>` expansion, no external images or fonts).
  Context discovery coalesces refreshes behind one native worker; typed drive
  letters come from the platform adapter, never UI path-prefix inference.
- Windows icons of every size come from the Shell's system image lists by entry:
  synthetic type names with `SHGFI_USEFILEATTRIBUTES`, stock ids, known-folder PIDLs
  and validated discovered fixed drive roots, so a type lookup opens no file, probes
  no media and touches no network path. The one exception that reads a listed file
  for an icon is a program's tile: in the Thumbnails view, a visible `.exe` tile
  shows the program's own icon where previews are allowed for its storage (the
  network preview opt-in applies). It is read between two revalidations of a
  native capture, through a handle opened without following links that denies
  writing and deletion meanwhile, by mapping the file as a data file and image
  resource: no code runs and no import is resolved. Icon groups and frames are
  parsed with bounds (64 frames, 4 MiB per frame, 1,024 px per side) and decoded
  by the system. Rows keep the type icon; Linux reads no listed file for artwork
  ([native icons](docs/developer/NATIVE_ICONS.md)).
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
- Tests and development scripts follow the same literal-argument policy. Windows
  junction fixtures are created natively through the Win32 reparse-point API
  (`FSCTL_SET_REPARSE_POINT`, no shell); the remaining ones use a fixed trusted
  PowerShell script via `-File`. Temporary paths are parameter data, never `cmd /C`
  command text.

The directory model still uses memory proportional to the number of entries.
Worker/front-queue bounds are not a promise that arbitrarily large directories
fit in RAM. Precise latency and memory targets require measured workloads.

## Directory authorization helper

**Retry with authorization** runs `midnight-elevated-listing` as a different,
more privileged identity: Administrator through UAC (`ShellExecuteExW` with the
`runas` verb, on its own COM single-threaded apartment) or root through
`/usr/bin/pkexec --disable-internal-agent` and the desktop polkit agent. The host
therefore takes the helper only from beside its installed executable and only from
a location that the requesting user cannot change; the plugin trust policy, which
also trusts the current user, does not decide this.

- Windows: the helper and every ancestor are ordinary objects on a local fixed drive,
  owned by SYSTEM, Administrators or TrustedInstaller. They carry no link or other
  reparse point, except the storage-only tags of WOF compression (CompactOS,
  `compact /exe`) and Data Deduplication, whose content stays under the object's own
  permissions; Cloud Files placeholders, whose content comes from a user-mode
  provider, app execution aliases and unknown tags are refused. No other
  identity holds delete, rename, write, attribute, permission or ownership rights on
  them, and none may add entries to the helper's own folder, which the elevated
  loader searches first. The checks run on handles that pin every component against
  rename or deletion until the elevated process has started; the folder pins still let
  other processes add entries. The Shell receives the
  plain drive spelling of the path only when it opens the same pinned file.
- Linux: the helper is an executable regular file; it and every ancestor are owned by
  root, are not symbolic links and are writable by neither group nor others, and its
  filesystem is not mounted `nosuid` (removable or user-mounted storage).

Any other location, such as build output, the per-user Windows private runtime or
`$HOME/.local/bin`, refuses authorization; the retry is still offered there. The
Cannot open folder message after the retry states the requirement, both when no
helper is installed beside the executable (per-user deployments omit it) and for a
helper in a refused location.

Builds before this check accepted a helper beside their executable through the plugin
trust policy alone, and earlier versions left the helper in folders that their user
can change: the deployment scripts copied it into Windows private runtimes, the Linux
per-user instructions installed `$HOME/.local/bin/midnight-elevated-listing`, release
packages contain it beside `me` (so an earlier Windows ZIP extracted into the user
profile, as its README suggested, or an earlier Linux archive extracted into a
user-owned folder and run as `./me` has it there), and build output holds it beside
`me`. In each such installation any process running as that user can replace the
helper that the older `me` then runs as Administrator or root. Current builds refuse
such a copy, so deleting it loses nothing: replace each such installation with a
current build, deployment or package, or at least delete its
`midnight-elevated-listing` helper. No script does this for them: `build.bat`
retires only build deployments that it marked itself, so installations made directly
with `scripts/deploy-private.ps1` (without `-ReplaceBuildDeployment`) and build
deployments made before that retirement existed are never removed, and nothing
touches extracted packages or per-user copies.

The channel is bound by process identity. On Windows the parent checks the pipe
client PID against the launched process; the helper checks the pipe server PID and
the parent's creation time, and opens the pipe with identification-only quality of
service, so no pipe server can impersonate the elevated helper. On Linux the private
`/tmp` socket's SO_PEERCRED must be the launched pkexec PID with root UID; the helper
verifies its original parent PID/UID, `/proc` creation time and `PKEXEC_UID`. The
256-bit nonce in the pipe name, socket path and command line is public: matching it
only rejects a stale or foreign helper and authenticates nothing.

The helper lists one directory from enumeration metadata without opening any entry;
Windows classifies links from their own attributes, and Linux follows a symbolic link
only with `stat` to classify a directory link. Frames are bounded (64 MiB, 250,000
entries, 256 KiB names, 16 KiB errors) and truncation is an error. Both ends wait on
I/O readiness, observing cancellation and the five-minute deadline within a bounded
slice. The helper ends its own process when the requester disconnects, sends further
data, exits or the deadline passes, even while a directory read is blocked. At most
two helpers can be pending or live per process. Actual UAC/polkit consent is not
exercised by automated tests; see the
[first qualification record](docs/verification/2026-11-11-directory-authorization.md).

## Verified clipboard copying and the copy contract

Ctrl+C publishes lossless native paths; Ctrl+V captures the destination path and
queues verified copying. Native identities are opened and pinned when that job
starts. An external rename before then is not a snapshot of the original namespace.
Clipboard payloads are untrusted: at most 4 MiB of names, any number of paths, with
whole-request rejection of malformed/NUL/relative data. Paste, Copy, Move and drops
take at most 4,096 native items at once, because each stays open until it is copied;
larger requests are refused with that reason before anything starts. No shell command parses a filename.
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

Copying is conservative on all storage classes in this milestone. Each copy job
runs one native I/O step at a time, including source reads, destination writes and
independent verification reads; running jobs share the configured disk-I/O limit step
by step. A Copy or Move starts at once unless the user chose **Wait for other
operations to finish**: its namespace admission is an immediate check-and-claim of the
job's own pins against the committed facts of the jobs already granted, never a wait
for another job's turn. Only real overlaps wait, item by item: a selected item that a
granted job claims or has deferred waits alone until its first change that no earlier
job refuses, while the rest of the job goes on. A job waits as a whole only when a
footprint it changes is unknown, when it would read what another job changes, or
behind an older waiting job that shares an item. A followed directory keeps its own
claim like any source, so another job's writes into it still wait; a link nested in it
can lead elsewhere, and reads there are not protected against another job's writes (a
followed source is never changed). A root a job only reads whose geometry is unknown
(a followed link, a provider without native geometry) is left unclaimed instead of
holding other jobs back, and reads through it are unprotected. Cancellation retains
capacity until the in-flight native call and cleanup actually return.
Future parallel streams within one Copy/Move job are permitted only when **both
source and destination are positively identified as local SSDs**, with a shared
physical-device scheduler. HDD, network, unknown or failed classification on either
endpoint requires one stream per job; an SSD at the other endpoint does not override
this. SSD-to-SSD copying within one job also remains serial today.

Ordinary Copy leaves source files untouched. Untrusted Move clipboard intent
through the ordinary Copy route is rejected. Explicit native Move accepts at most
64 ordinary file/directory roots with a conservative 4 MiB active proof-path
reservation. Archive members, duplicate/overlapping sources and unsupported
locations are rejected before mutation; links follow the link rules below.
Qualified same-volume native Move uses
held source/destination identities and no-replace rename without content reads,
CRC or SHA computation. Whole-directory rename does not enumerate descendants.
Same-volume errors never silently fall back to copying. Proven cross-volume moves
and explicit directory merges use the per-file proof and postorder directory
contract below, with explicit live-namespace and durability limits. F6 consumes native tree
captures as well as flat selections; native tree Cut publishes Move intent through
the clipboard capture contract above. Cut itself removes nothing. Copy remains
available where Move is unsupported, without silent fallback.
Copy supports files and ordinary directories; special files are rejected.

Links. The Copy/Move dialog's **Links** choice is the same on both platforms and for
Copy and Move: copy the files links point to (`FollowLinks`) or keep links as links
(`Preserve`); its default comes from what the destination's volume reports (statfs type
and sysfs on Linux, `GetVolumeInformationByHandleW`, drive type and the disk's storage
descriptor on Windows), never from a probe file. A Move that copies what a link points
to verifies the copy like any copy and then removes only the link: the target, which was
not selected, is never moved, removed or written. A link is transferred from its exact
representation (Linux `readlink` bytes; Windows substitute and print names and flags read
with `FSCTL_GET_REPARSE_POINT` through a no-follow handle), never by resolving or rebasing
its target: Copy without following recreates every link, Move moves the link itself, and
Copy that follows links opens a target only under the directory-link rules (no magic
links; no target on one of the kernel's interface filesystems, by `statfs` type: proc,
sysfs, devpts, cgroup, cgroup2, debugfs, tracefs, securityfs, apparmorfs, pstore, bpf,
configfs, fusectl, mqueue, autofs, efivarfs, binfmt_misc, selinuxfs, smackfs, nsfs,
resctrl, binderfs, nfsd, rpc_pipefs, xenfs and the USB gadget filesystems functionfs and
gadgetfs, so no process state such as /proc/self/environ is copied and no automount
point is mounted by a link; no link back
to a folder on the current path, by device and inode across filesystems; at most 128
links followed inside one another; a target this process can read and list) and
recreates any link it cannot follow, with the reason. A target on another filesystem,
local, network or FUSE, is followed as if its folder were selected. A text that stays
on its folder's mount is resolved within it (an absolute one name by name from the
mount's root: a mount point or `..` above the mount's root ends the walk, at most 40
links are read, and the mount's lineage is checked first); a text that leaves the mount
is resolved by the kernel across mount points without magic links, and the folders
below that link are revalidated by resolving the link again from its held folder to the
same device and inode. Copy has no fixed depth limit for ordinary folders
(Move stops at 128); instead every folder a transfer (or its metadata discovery) works
inside is one level of a budget shared by all transfers of the process: a quarter of the
open-file limit at three descriptors per level, between 32 and 4,096 levels. A folder
past it is refused with a clear error long before the process could run out of
descriptors (EMFILE) and fail whatever else opens a file. At start the application
raises its soft open-file limit to the hard limit, at most 65,536 (4,096 levels), and
gives the programs it starts for the user (an editor, a terminal, a custom command, a
document's application) the original limit back, as programs that use select() expect.
Reading a link revalidates its parent binding, identity, revision and text,
so a link replaced after preparation is refused. A recreated link is created only as
`item` in a fresh owner-only `.midnight-link-<random>` directory inside the protected
destination folder, read back without following it, compared with the source text,
revalidated (stage binding, identity and text) and then published with a no-replace
rename, or with an exchange into private `.midnight-recovery-*` when the user chose
Replace for an existing file or link (never a folder); the displaced entry is kept.
A published link is checked again; an unexpected result keeps everything and is
reported. Cleanup removes only the stage's own link, by identity, and the empty
stage; an unknown entry there is kept and reported. A Move across filesystems
removes the original link only after the recreated one is published and checked,
through the protected link removal (owned link with one name, staged privately,
revalidated, unlinked); a failed removal keeps both and reports it. A same-volume
Move renames the link through the same private capture as a directory link. No link
target is ever opened, written or deleted by these steps.
On Windows a link is followed only to an ordinary folder or file on a local drive (no
network target, mounted volume or reparse point on its way; the cycle check compares
volume serial and file ID); a followed file is held with write and delete sharing
denied. A recreated link is staged under a private `.me-<random>` name in the destination
folder, held through its own no-follow DELETE handle, read back from that handle,
revalidated and published with a no-replace rename; cleanup deletes only that held
object. A junction is written with `FSCTL_SET_REPARSE_POINT` on a folder this job just
created (exact names, a local absolute target only) and needs no privilege. A symbolic
link is created with `CreateSymbolicLinkW` and the unprivileged flag, which works only in
Developer Mode or for a process that already holds the symbolic-link privilege (an
elevated one); the application reads the Developer Mode setting and its own token's
privileges to decide, never requests elevation for it, and never uses the
`midnight-elevation` helper (a one-shot elevated directory reader) to create links.
`CreateSymbolicLinkW` takes a path: it is given the destination folder's path and the
private name that the namespace turn just checked absent, and the created object must
then open, relative to the held folder, as the planned link. Without the right, a
directory symbolic link to a local folder becomes a junction to the same folder and the
job details say so; any other symbolic link is left out with that reason. No link is
created at a network location or on a volume without reparse points.
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
filesystems without that capability return Unsupported, so Linux Copy and cross-volume
Move cannot write to FAT/exFAT/NTFS media, NFS/SMB, sshfs or GVfs FUSE shares. A
staged file stays an owner-only (0600) unnamed inode until it is published, and a new
copy directory stays private (0700) until its contents were copied; a cancelled copy
leaves its unfinished directories private. On publication the copy gets the source's
read, write and execute bits for user, group and other masked by the process umask,
as `cp` without `-p` does (a source without modes, such as an archive entry or a
new file list or checksum manifest, gets those of any new file or folder: 0666 or
0777 masked by the umask); setuid, setgid
and sticky bits are never copied (a new directory keeps a set-group-ID bit
inherited from its parent), and a folder that a copy merges into keeps its own mode. Generated outputs replacing an existing file
(file lists, checksum manifests, an empty file over an existing one) keep the
replaced file's mode, ACL and extended attributes. Where a filesystem cannot store modes, a new
directory keeps the mount-provided permissions only while no other user can write to
it (group write only for the user's private group, without an access ACL); otherwise
it is removed again and Copy fails. Creation requires a protected parent:
owned by the user or root, and either sticky or not writable by other users. Group
write is accepted only for the user's private group: the effective group, named
after the user in `/etc/group`, with no other member and no other `/etc/passwd`
account using it as its primary group, and only while the folder has no POSIX
access ACL (with one, the group permission bits are the ACL mask, which can let
named users or groups write). Shared-group, ACL-shared or world-writable parents
without sticky protection are refused. A selected mount point may be a Copy root;
Copy never enters another mount below a selected folder and reports such mount
points as unsupported. mkdir/open is not an atomic operation; same-UID/root
interference remains outside this isolation boundary.
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
require content-read or directory-list rights. After a committed native rename, Windows
reopens the source and destination folders (and a replacement's private recovery
folder) through their retained handles, never by path, with only the right to add
files or else subfolders, sharing everything, to flush them, and closes them at once;
no volume handle or administrator right is used. A Move keeps those retained folder
handles until it flushes the folders of its pending renames together (before it
removes a folder, after 64 renames, and when the job ends); the handles grant nothing
more than the Move's pins. A folder that cannot be opened so
leaves the Move's durability unconfirmed and never refuses or undoes the rename
([Move durability on Windows](docs/developer/TRANSFERS.md#move-durability-on-windows)). Linux
requires protected source parents (the Copy parent rule above) and effective
mutation permissions.
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
override. All application DELETE/source owners close (a directory link's capture closes
its own deleting handle right after the disposition), then the removal is confirmed:
its name is looked up through the held folder that contained it, never by path, and only
"no such file" counts as removed; that folder is then flushed as after a native rename.
This changes neither which object is removed nor any check before the removal; a
removal it cannot confirm is reported as accepted by Windows and unconfirmed.
Empty-stage cleanup runs next, before the next file. External readers may retain bytes
after namespace removal; no measured physical space is promised.

A skip retains that source. Every pre-deletion failure or cancellation after
staging attempts a no-replace restore with an independent cleanup token. A new
original-name winner is never clobbered. Earlier completed moves stay completed and
published destinations are never rolled back. Drop never finalizes retained data.
Success retains no source copy or empty temporary stage. Failed restoration or
empty-stage cleanup preserves all remaining data and reports exact exceptional
paths. A capture that cannot remove the empty owner-only `.midnight-move-*`
directory it created (for example because another program created an entry in it)
keeps it; a repeated capture reuses it, and otherwise the job's cleanup removes it
or reports its path (see
[bounded ordinary Move](docs/developer/TRANSFERS.md#bounded-ordinary-move)).
Cleanup never recursively deletes unexpected entries. Structured outcomes
distinguish removal from residuals and durability uncertainty. Linux uses no-replace
temporary staging and live-validated directory-relative unlink for owned,
single-link regular files on qualified local ext filesystems with protected
parents. A read lease is retained from preflight through final content validation
and removal. Existing writers/mappings or observed lease breaks, revisions or
binding changes refuse removal. A dedicated thread blocks only its own SIGIO, owns
the thread-directed lease notifications and releases a breaking lease at once, so
a program opening the source for writing is not stalled for the kernel's
lease-break timeout; that Move then refuses source removal. No process-wide handler
is installed. A writer opening the source between the final check and the unlink
is not detected: removal is live validation, not an atomic compare-and-unlink.
See [the native race evidence](docs/verification/2026-11-08-ordinary-move.md).
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
budget. Links move as links (above); mount crossings and generic provider Move remain
unsupported.
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
   streaming and compares those bytes after sync, plus exact length. Up to 192 KiB
   is compared entirely. Checking only size or hashing an outgoing buffer twice
   does not satisfy either policy. Native same-volume rename copies no data.
4. Reject any detected source change. Source metadata checks do not prove a
   point-in-time snapshot against concurrent writers; a stronger snapshot promise
   requires supported write exclusion or filesystem snapshots. In particular,
   same-length rewrites within one filesystem timestamp quantum can retain both
   modification and change times and evade metadata revalidation. Copy and Move
   also accept a pinned source, conflict target or Move capture whose change time
   alone moved (VIEW-05), so a same-length rewrite that restores the modification
   time goes undetected by them; a Windows source file's handle denies write sharing
   from when its job opens it to read it, before the copy is staged (while a Copy waits
   for admission it holds no handle on its selected items, and a change before that
   open is only detected through identity, size and times), Linux Move removal keeps
   its read lease and a replaced target stays in recovery. User commands, checksums and Rename keep the change time. The digest
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
blocking wait runs on the UI. Transfers hold a disk-I/O slot only while executing
native steps, never while paused or waiting for a user decision or another job,
and yield it between steps to queued call-scoped reads and other transfers, so
transfers sharing the limit interleave step by step. Per-job cancellation,
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
bounded incremental transfers, and an owner session that the backend worker serves
from an event-driven wait (`poll()` on the X11 connection and the session's eventfd)
and closes once it no longer owns clipboard content.
Native Wayland file-copy ownership is unavailable until a bounded writer is
implemented; a usable XWayland bridge is preferred. Native Wayland file-list reads
remain bounded. No per-copy detached ownership threads may accumulate.

Required corruption/failure tests include modified destination bytes, truncated
data, same-size edits, changed sources, short reads/writes, disk-full/disconnect,
flush/sync failures, destination creation/replacement races and cancellation at
every phase. No copy is marked successful before its complete verification.

## Unicode acceptance

Unicode covers filesystem names and paths, UI models and editing, volume labels,
errors, CLI/configuration, search, rename, clipboard and copy/move operations.
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

Directory links and junctions retain a separate entry kind. On Windows only
name-surrogate reparse points (symbolic links, junctions, mount points) are links;
Cloud Files/OneDrive placeholders, deduplicated, WOF-compressed and other
filter-backed entries are ordinary files/directories in listing, Find, Copy/Move,
deletion, creation and metadata editing. Their content is read only through a
filter-mediated reopen of the identity-pinned object (it may hydrate), never from
the raw reparse stub. That reopen evaluates the reparse point the object carries at
that moment: should another process with write access replace a filtered file's
reparse point with a symbolic link after classification, the open follows that link
before the identity check rejects the result. It therefore requests
identification-level impersonation only, so a named pipe reached that way cannot
impersonate the application. A reparse point whose tag cannot be read is treated as a
link. Native Windows link
icons use system overlays on cached generic types, without sending untrusted
link paths to the Shell or opening targets. Linux icons reuse the listing's
directory-link label (above) and open no target. Recursive sizing
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
Recursive traversal does not follow links or cross mounts. It captures each
directory's child names through the directory's pinned handle before changing any
of them, then takes them in that order and opens and pins each child by its
captured name, relative to that handle, only when it processes it, and processes
children before their parent. A child replaced after the names were captured is
processed under its new identity; a child that can no longer be opened, for example
because it was removed, is reported as failed without stopping its siblings;
entries added later are not visited. When a child fails and its directory then
fails revalidation, for example because another program moved it, that directory
stops with one error that counts the entries it did not process. Case conflicts are
found before any entry is renamed, among the selected targets and among each
directory's captured names: two names that would become equal, or a name that would
become another entry's current name. An entry whose own name stays is never a
conflict and keeps its other changes. Each child's own name check runs when it is
processed and sees the renames of earlier siblings, so with title case, which is not
idempotent, an outcome can depend on the processing order; every rename still
refuses to replace an existing entry. Selected targets of all prepared, queued and
executing metadata jobs share a 256-target pin budget. Traversal never takes or
waits for those pins: it keeps only the directories on its current path and the
child it is processing pinned, at most one target per depth level. Only one
mutation job executes at a time, because it holds every mutation-gate share, so at
most 320 metadata targets are pinned at once. Depth is limited to 64. The name
snapshots of the directories on the current path together hold at most 100,000
names and 4 MiB of names that they have not processed yet, each name counted as its
`OsStr::len` (its raw bytes on Linux, WTF-8 bytes on Windows) and released when its
entry is taken; a directory whose names do not fit is reported and left unchanged
with its contents. Resource exhaustion is reported as an error that names its cause
and keeps the file system's own report, not as a successful truncated operation.
These are operation resource budgets, not path-length limits. A metadata preview
lists every selected target, at most 256 rows; operation previews and metadata
result samples keep at most 32 items.

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
In flat native panels, hovering an ordinary file resolves to the current pane
root after identity/navigation checks; the file itself is not a write target.
Folder targets and tree occurrence authority retain their existing validation.
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
OLE delivers a drop only after the Windows target showed the Copy cursor. When the
target then refuses it (an unreadable or over-limit file list, failed admission, or
a destination, pointer or modifier change since that cursor), it still answers "no
effect" and also reports the reason as a drag/drop failure without a gesture ID.
Report texts are fixed: a drag source's own error description is never shown, only
its HRESULT code. The callback never waits for delivery: the backend keeps only the
latest undelivered refusal, outside the bounded event queue, and reports it once the
UI accepts events again, unless the drop window has closed by then. Linux X11 drops
report the same kinds of refusal, in fixed texts (the Windows ones for a changed
destination and a missing file list), once per drop that showed the Copy status: an
undecodable file list, a drop of more than 4,096 items, an over-4 MiB transfer, a failed validation
or admission, a destination change (at XdndDrop the drop goes to the panel target now
at the last position, as on Windows, and is refused when there is none; while the list
is read, only a republish of the same folder keeps it), and a source that does not
deliver its file list (a lost or refused selection, a wrongly typed transfer, a new
drag before the list arrived, or the 5-second read timeout). A Move-only offer or a
drop that never showed Copy is refused silently, and a closed window has nobody to
tell.

Pending drop admissions retain their ordering relative to later clipboard copies.
Cancelling one before backend enqueue rejects its token rather than sending a
CancelCopy that could arrive before the job exists. Transient native grab failures
must not permanently disable a healthy session or revive an ended gesture.

Payloads retain raw UTF-16 on Windows and raw filename bytes through X11 URI
encoding. Payloads hold at most 4 MiB of names, and a drop copies at most 4,096
native items; malformed and
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
synthetic release callbacks cannot cancel that handoff. The drag bridge
(`crates/midnight-ui/src/drag_bridge.rs`) hands an outgoing drag to the operating
system, and so to other applications, only once the pointer has left the window and
the press that started the drag is at least 400 ms old. Until then the window keeps
the pointer, so a release sooner than 400 ms after the press, inside the window or
outside it, cancels the drag and transfers nothing.

Linux uses a verified worker-owned XDND proxy. The host window's XDND advertisement
is removed while the proxy is installed, preventing normal legacy sources from
using winit's lossy UI-thread parser. The proxy advertises XDND v5; native Wayland
external transport is unavailable until same-connection seat/data-device support
exists. Internal panel dragging remains available. X11 incremental transfers are
bounded and timed out. Move proposals may negotiate Copy only when the source
explicitly offers Copy; a Move-only offer is rejected.

<a id="dynamic-viewers-and-planned-filesystem-plugins"></a>

## Dynamic viewers and filesystem plugins

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
- Plugin projects share one Git repository but depend only on the public API/SDK,
  except the in-repository path dependencies that the
  [developer guide](docs/developer/README.md) lists (PLUGIN-01 in the
  [backlog](docs/IMPLEMENTATION_BACKLOG.md)). Their independent Cargo workspaces
  contain their tests and have no write access to the host application's sources.
  CI checks the basic, source, Markdown, image and PDF viewers, the image thumbnailer, the viewer conformance fixture and the
  Windows/Linux native, network and archive filesystems from exported directories,
  without host sources: viewers and workers against the released public SDK
  bundles, filesystem libraries against the checked-in public filesystem API/SDK.
  The HTML viewer, the video thumbnailer and the macOS native filesystem have no CI
  job yet.
  Building, testing or installing a plugin must never patch host sources or the
  executable. Integration tests consume a released host binary in a temporary
  distribution; extending host API requires a separate host-owned change.
- Shared Windows host builds can resolve a private runtime published by explicit
  deployment. By default a protected per-user launcher pointer for the launch
  executable's native path selects the registration, but only while that executable
  keeps the length and last-write time recorded at deployment; startup hashes no
  executable. Deployment verified that the registered executable had the build's
  SHA-256, and `MIDNIGHT_VERIFY_RUNTIME_SHA256=1` instead requires the registration
  and its target executable to match the running host's SHA-256 at startup.
  Registration and target paths reject reparse points before canonicalization and
  retain the same ancestor/owner/DACL policy. Record reads use handles denying
  concurrent writes and renames, with bounded lengths and revision checks. No
  package is copied or trusted automatically at startup. The process caches the
  resolved location; existing discovery/load checks still validate code. The
  pointer and the hash only match builds; they do **not** authenticate a publisher
  or make an already launched shared executable safe. See [runtime discovery](docs/developer/RUNTIME_DISCOVERY.md).
- Host-owned leases scope content services; no Rust allocator, future, Tokio
  runtime or Slint object crosses the ABI. Buffers have explicit ownership and
  release functions. Loaded code outlives every worker, callback, session and
  buffer. Version 1 uses restart-based installation/removal, with no hot unload.
- Text viewers return bounded Unicode text and semantic style spans, never
  executable markup or source-directory grammars. Parsing and decompression run
  on bounded backend workers; cancellation is cooperative, not a promise to kill
  a blocked thread safely.
- File content cannot reorder displayed text: the basic viewer and the text
  comparator show the explicit bidirectional embedding, override and isolate
  controls (U+202A-U+202E, U+2066-U+2069) as `\u202E`-style escapes, the source
  and Markdown viewers show them as U+FFFD, and terminal controls appear as
  escapes or control pictures. Copy and search use the displayed text.
- The native content broker provides LiveValidated reads, not a filesystem snapshot:
  it prepares identity, opens without following links, and checks revision before
  and after reads. Replaced/changed files report an error. Viewer captures and,
  since 21 November, archive file captures accept a revision whose change time alone
  moved (VIEW-05: same volume, file ID, length and LastWriteTime on Windows; same
  device, inode, length and mtime on Linux), as endpoint protection moves it on
  freshly written files and on archives hours later; a same-length rewrite that
  restores the modification time is then not detected while a file is displayed or
  an archive is open, nor when a saved tab or Hot Path reopens that archive. A
  different file at the path, a link and every other change are still refused.
  Reads are bounded to 256 KiB and use u64 offsets. Four live sessions share the
  host limits; decoded pages, wire items, retained service replies and raster
  surfaces have separate explicit budgets. Selection/copy is limited to the loaded
  page.
- Markdown returns semantic text blocks and inert link metadata. The host does not
  execute HTML/scripts or load remote images, styles, fonts or links. Source syntax
  definitions are packaged code, never executable configuration from the opened folder.
- Archive browsing is read-only. Names, links and declared sizes are untrusted.
  Decompressed content has no artificial size ceiling: it is bounded by the free
  space of the host's private spool volume, and disk-full or short writes discard
  the staging and fail the request. Staging also stops with a disk-full error before
  that volume's free space falls below max(1 GiB, 2% of capacity), so a
  decompression bomb cannot fill the spool volume. The host queries the volume on
  each stage's first append and at least every 64 MiB; writes by other programs
  between two queries are not counted. A member whose declared length needs a spool
  file claims that space when its staging begins, beside the space that stagings still
  in progress have claimed: one that would not fit above the reserve is refused before
  anything is written, and under protocol 3 the refusal keeps the enclosing worker and
  session (an archive inside an archive stays at its enclosing folder).
  Decoder dictionaries/windows, buffered metadata records, the raw 7z next header
  (an eighth of the worker's memory) and retained verified 7z siblings have
  explicit bounds inside the 1 GiB worker memory limit; the metadata index (no
  member-count or name-bytes limit), 7z compressed-header decoding and ZIP
  central-directory parsing (including the duplicate-record check) are bounded only
  by that limit, and an index that outgrows it fails its archive. A failed read of
  the source is reported as such, never as a corrupt archive. Listing never
  extracts a member to its declared host pathname.
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
- Video thumbnails and playback use separately installed restricted helpers in the
  workers' capability-free AppContainer; the host brokers read-only source bytes.
  Installation grants that AppContainer read and execute on the five shared FFmpeg
  libraries and the two playback executables only, never a folder or user content; the
  build and deployment scripts compute its SID from the name instead of asking Windows
  inside PowerShell, which in a packaged PowerShell names a different AppContainer (see
  the [worker SDK](docs/developer/WORKER_SDK.md)). Every worker and browser broker
  starts with critical-error, crash and open-file dialogs off, so a helper that cannot
  load a library ends with the loader's status instead of waiting on a modal system
  dialog, and the host names the library and why.
- On Windows, video that the bundled FFmpeg profile has no decoder for is decoded by
  Windows' Media Foundation in two separate platform helpers (thumbnail and playback)
  that run in the same AppContainer and job and link no FFmpeg. Media Foundation sees
  only a read-only, seekable byte stream over the host-brokered capture, with no URL or
  path, so no source can reopen the file, follow a reference or reach the network; each
  read is a positioned broker request that the helper's pipe thread serves. Decoding is
  software only: no Direct3D manager, no hardware transforms and DXVA disabled, so no GPU
  driver code loads. Before reading any content, each helper turns on Windows' binary
  signature policy for Microsoft-signed images only (`SetProcessMitigationPolicy`,
  `MicrosoftSignedOnly`): Windows' own decoders and Microsoft's Store codec extensions
  load, an unsigned or third-party decoder or byte-stream handler registered system-wide
  (a codec pack's) cannot. The helpers convert, scale and rotate decoded pictures
  themselves, refuse HDR and BT.2020 video like the bundled profile, and send the host
  only frames, PCM and numeric failure causes; a missing decoder is a codec number that
  the host words. No capability is added to the AppContainer, and nothing is
  downloaded or installed when a decoder is missing.
- On Linux, such video is decoded by the distribution's own FFmpeg (libavcodec 58 to
  63), else by GStreamer 1.x, in two separate platform helpers that link neither and
  run under the same worker sandbox as the bundled helpers. The sandbox is unchanged:
  both families are loaded before the helper enters it, and after that every file
  open, network access, new process and new thread is denied, so nothing is loaded
  lazily and no library or plugin folder is readable afterwards. Read rules for what is
  loaded: the helpers start with a cleared environment and have no RUNPATH, so the
  system loader resolves a library only through its root-owned cache and default
  folders, never beside the helpers (where the bundled `libavcodec.so.63` lies) or from
  `LD_LIBRARY_PATH`; every object then mapped must have a canonical path whose file and
  folders are owned by root and writable by nobody else, or both families are refused.
  GStreamer plugins come only from a fixed list, from the folder beside libgstreamer,
  checked the same way before loading; device and hardware-decoder (VA-API, V4L2,
  vendor GPU codec), network, display and capture plugins and the user's plugin folders
  are never loaded. GStreamer runs without its plugin registry (`GST_REGISTRY_DISABLE`, no
  `gst-plugin-scanner` process, nothing written into the user's home). A process maps
  one FFmpeg series: gst-libav is loaded only when no distribution FFmpeg was, and the
  bundled FFmpeg never shares a process with either family. FFmpeg reads only through a
  custom AVIO context over the brokered capture with an empty protocol whitelist and a
  refusing `io_open`; the GStreamer graph has no URL source, is driven on the helper's
  one thread by pushes of brokered bytes, and seeks through byte seeks the helper serves
  within the capture. Pictures and colour follow the bundled profile's rules (HDR and
  BT.2020 refused). Two consequences of the unchanged sandbox are handled before
  lockdown: GLib's first shared thread pool reads the scheduler settings then (after
  lockdown it would abort trying to start a thread), and GStreamer buffer pools, which
  can no longer create their wake-up descriptors, are never waited on by a graph on one
  thread (their warnings are discarded with stderr). A missing decoder is a codec number
  and the families tried; the host words it with the distribution's package names and
  never installs anything.
- The built-in Office contents preview parses captured DOCX/XLSX/PPTX/ODT/ODS/ODP
  bytes on viewer backend workers into bounded inert HTML: no scripts, macros (VBA
  projects and ODF `Basic/`/`Scripts/` libraries are never read), formula
  evaluation, external links or resources. Before `zip` builds its index, a
  preflight checks every central record and local header: extra fields must be exact
  record sequences (unused records such as Office's 0xA220 padding are skipped,
  ZIP64 values are resolved, a Unicode path record naming another part is refused);
  local and central names, flags, methods, CRCs and sizes must agree; members must
  not overlap, checked before each header's variable part is read; names are unique
  and safe, with no symbolic links; an archive comment cannot hide another
  directory. `zip` then indexes a canonical directory without extra fields or
  comments. Encryption, OLE compound files, compression other than stored or
  Deflate, split and ZIP64 archives, DTDs and non-UTF-8 XML are refused. Limits: a
  256 MiB file, 10,000 parts, an 8 MiB central directory, 4 GiB per part, 256 MiB of
  reads and unpacked data per page, and XML tokens of 64 KiB nested at most 128
  deep. Markup the preview does not read is skipped and never shown as document
  text ([package rules](docs/developer/VIEWER_PREFERENCES.md#office-packages-what-opens-and-what-is-refused)).
- SVG images and thumbnails use the shared image decoder inside the restricted image
  workers. Scripts and event attributes never run; `foreignObject` is skipped (a
  `switch` draws its SVG fallback); links to files or URLs (images, `use`, paint,
  clip, mask, filter and marker references) are never opened, and there is no
  resource directory, system font or network access. Raster images embedded as
  `data:` URIs are decoded by the raster path with its limits and reach resvg only as
  PNGs the decoder encoded. Entity declarations (a DOCTYPE internal subset) and
  reference cycles are refused, so entities are never expanded. Before conversion the
  decoder bounds the source (4 MiB), XML nodes (262,144), nesting of elements and
  references (64), the converted tree after expanding `use`, marker, pattern,
  clip-path, mask and filter references (256 MiB, estimated as 1 KiB per element and
  4 bytes per byte of attributes and text) and style-rule checks (2^27); before
  rendering, at the drawing size, memory while drawing (448 MiB, tree estimate
  included) and weighted pixel work (2^33, about half a minute, calibrated by
  micro-benchmarks), following resvg's own layer, mask, filter, pattern, image and dash
  recursion, because resvg allocates without limits. A refusal names its reason and
  bound; the workers' memory/CPU limits and the host's deadlines remain the last bound
  ([decoder contract](plugins/image-decoder/README.md#svg)).
- File comparison captures native regular-file identities. Both text inputs
  have byte/line/deadline limits; image inputs use the restricted image broker.
  A text source whose revision moved only in its change time is accepted after a
  complete fresh read reproduces the retained text byte for byte; image sources
  and every other revision change report a change. Rendered comparison surfaces
  and retained frames are bounded. Commands and labels derived from content never
  execute.

## Archive passwords

Password-protected archive members (ZipCrypto, WinZip AES, 7-Zip AES; see
[archive formats](docs/ARCHIVE_FORMATS.md#password-protected-archives)) are decrypted
only inside the sandboxed archive worker, through the same verified staging as plain
members: no host, engine or UI code parses or decrypts archive bytes, and nothing is
published before its CRC-32 or WinZip AES authentication code has been checked.

A password is kept in memory only, by the worker of the one archive session it
unlocks. It is asked once and kept until that archive is closed: until no panel tab,
viewer or copy uses the archive any more, or the application exits. The host then ends
the worker process, whose memory goes back to the operating system without the worker
wiping it first. Opening the archive again starts a new session that asks again, and an
archive inside an archive asks for and keeps its own password. It is never written to
settings, history, logs, events, error or Copy-details text, or the clipboard. It
travels from the password dialog's callback straight into `Command::UnlockArchive`
(`midnight_core::ArchivePassword`), to the backend worker that calls
`ArchiveMount::unlock`, and through the protocol-4 `Unlock` frame to the worker
(`midnight_filesystem_api::Password`). Both types hold their bytes in `zeroize`
buffers that are wiped when dropped and print `<redacted>` in `Debug`; the API codec
wipes the frames that carry a password after writing and reading them, and the
answer (`Event::ArchiveUnlocked`) carries none of it. Known limits: the dialog's text
field is owned by the UI toolkit, which frees but does not wipe it when the dialog
closes; the 7z decoder keeps its own UTF-16 copy for the mount without wiping it;
operating-system pipe buffers are outside the process. A wrong password is refused by
the worker's verifiers without unlocking anything; local password attempts are not
rate-limited.

## Dependencies

Keep `Cargo.lock`, check advisories against the actual lockfile, and record the
database revision and warnings. A clean advisory scan means no matching known
vulnerabilities in that database; it does not prove the absence of unknown bugs.
Do not adopt source or dependencies merely because they look similar to reference
code. Salamander code/assets are not copied into this implementation.
