# Custom Commands backend

The backend accepts immutable command configuration and panel context, prepares
native arguments on workers, and either returns a preview or dispatches the
configured processes. UI configuration, toolbar interaction and activation are
described in [the settings contract](USER_COMMAND_SETTINGS.md). No viewer/plugin
ABI or additional dependency is used.

## Configuration and persistence

`UserPreferences.user_commands` is an ordered `Arc<Vec<UserCommand>>`, empty by
default. Each command has a stable ASCII ID, name, enabled flag, optional toolbar
label/icon path, working-directory template, Single/PerItem launch policy, and a
Program, Terminal or inert Separator invocation. Duplicate labels are allowed; duplicate IDs are
invalid. Executable/icon literals remain native `PathBuf` values.

Program mode stores an explicit absolute executable plus at most 64 argument
templates. Terminal mode stores literal user-authored script, shell flavor and an
explicit scalar `variables` list, empty by default. The picker must add a required
variable declaration in the same accepted edit as its script reference. Manual
script authors declare required context too; the backend never scans or rewrites
shell source to infer requirements.

Separators retain stable ID, name, order and the same configuration count/byte
budgets. They require no executable or working-directory template, register no
action, and cannot be previewed or executed. Runtime admission and preparation
both reject them before native capture or launch, including stale shortcut requests.
They never become `UserCommandProcessInvocation` values.

Schema 17 uses the existing strict, bounded, atomic settings service and native
path codec. Older settings default to an empty command list. A schema17 payload
includes checksum algorithms even when SHA-256 is the default, preserving the
schema16 contract. Default preferences still encode as version10. Invalid/null/
unknown fields, duplicates and malformed templates fail validation instead of
being silently dropped. Separators serialize as an invocation with only
`mode: "separator"`; existing Program/Terminal records remain readable. On load,
stale shortcuts pointing to known separators are pruned before final preference
validation without removing unrelated overrides. No real user preference file is
used by backend tests.

Apply a list edit through `UserPreferences::replace_user_commands`. It validates
the entire candidate and removes shortcut overrides for deleted IDs and IDs
converted to separators
before replacing the accepted value. A failed edit preserves the previous value.
`ShortcutMap::with_user_commands` adds only executable configured `user-command.<id>` actions
in file-idle scope; `definitions()` exposes that exact catalog. Dynamic actions
have no defaults. `ShortcutMap::new` continues to provide the static catalog.

## Literal data and templates

Supported variables are `${active.path}`, `${other.path}`, `${file.path}`,
`${file.name}`, `${file.stem}`, `${file.extension}` and `${selected.paths}`.
`$$` encodes a literal dollar. Inserted native values are never recursively
expanded, normalized through display text or joined into a shell command.

Single file scalars use the captured focus. PerItem binds them to each captured
selected item, using focus only when selection is empty. Nonempty selection is
authoritative: vanished/inconsistent selected rows cause failure, without adding
the focused item. The group variable occupies a complete Program argv element
and cannot be combined with PerItem. Empty extension is an available empty value;
missing required context is an error.

Working directory expands to one native path and is passed via the process API.
There is no generated `cd` source. Program arguments are separate `OsString`
values, preserving invalid Unix UTF-8 and native Windows UTF-16. Windows direct
launch rejects batch files; it uses an explicit executable without association or
browsed-directory PATH lookup. Unix executable scripts retain OS exec semantics.

Terminal data is exported only through declared fixed keys: `ME_ACTIVE_PATH`,
`ME_OTHER_PATH`, `ME_FILE_PATH`, `ME_FILE_NAME`, `ME_FILE_STEM` and
`ME_FILE_EXTENSION`. Undeclared inherited values of these keys are removed.
Missing declared values fail before any spawn. Script source stays literal;
environment values cannot become recursively expanded template source. Windows
uses an owned, sorted Unicode child environment, including per-drive cwd records;
Linux uses native environment values on the existing terminal builder. Parent
environment and process cwd are never changed. Script text is not automatically
added to command history or logs; process debug output redacts source/argv/env.

Windows supports PowerShell and Command Prompt; Linux uses supported installed
POSIX terminal profiles. Custom launches do not add an elevation/credential path.

Custom Command Prompt uses the trusted system Cmd with `/D /U /E:ON /V:ON /S /K`.
Its source is one literal command line: use explicit Cmd operators between
commands, or choose PowerShell for multiline scripts. CR/LF in source is rejected
because native command-string mode otherwise ignores subsequent physical lines.
This restriction does not affect control characters inside captured data.
The ordinary command-bar Cmd profile remains unchanged with `/V:OFF`.

Each declared raw `ME_*` value also receives a backend-created `ME_*_ARG` companion.
The picker inserts an unquoted delayed reference such as `!ME_FILE_PATH_ARG!` as
one complete argument for a directly invoked external program using Windows
C-runtime argv parsing. Do not add quotes around that reference. Companions handle
empty arguments, quotes, trailing backslashes, Unicode and raw UTF-16 without
inserting filenames into script source. Caller configuration cannot supply or
replace these companion keys; undeclared inherited raw/companion keys are removed.

This is an external-argv contract, not universal Cmd quoting. Builtins such as
`echo`, `type` and `cd` use raw values with their own syntax, for example
`type "!ME_FILE_PATH!"`. Empty raw references follow Cmd's own behavior; companions
preserve an empty native argument. Do not pass companions through `CALL`, a batch
file or another shell that parses expanded text again. `CALL` can re-expand percent
variables and alter carets; arbitrary pipeline/grouped reparsing is not qualified.
Keep delayed expansion enabled. Program mode is preferable when shell syntax is
unnecessary.

Cmd does not accept UNC/device working directories. A verbatim DOS directory can
use its ordinary DOS spelling only after comparing both live native directory
identities; normalization to a different trailing-dot/space name is refused.
Captured raw environment values keep their original spelling. No cwd is
truncated, and a native long-cwd startup error is returned rather than launching
elsewhere. This is a shell capability boundary, not a limit on file-manager paths.

The adapter refuses initial Cmd command lines, declared/encoded environment
records and the conservative expansion of literal fixed references above 8191
UTF-16 units. The reference budget is ASCII case-insensitive and includes repeated
occurrences; it does not rewrite or parse shell syntax. Dynamically constructed
script variables remain subject to Cmd's own limits. The existing aggregate
8 MiB child environment limit remains; no value is truncated. See the
[native qualification](../verification/2026-11-12-cmd-custom-commands.md). Windows loader errors use a scoped thread error
mode, restored on return, to report errors without blocking loader dialogs.

## Inline-tree activation

Native inline trees enter the same preparation, preview and launch pipeline using
an opaque `CapturedUserCommandSources` owner. The tree worker captures selected
occurrences separately from focus; only backend resolution creates native `Entry`
values. Once, PerItem, grouped arguments and literal commands preserve their flat
panel semantics. The original active/other directory contexts stay frozen and the
captured native root must match the active directory. Archive/provider targets and
context-only rows are refused without exporting or guessing native paths.

The common preparer validates the entire capture and creates ordinary native pins.
Provider leases establish registration/path context, not a historical filesystem
snapshot. Directory listing size/mtime may be cached and describe mutable contents;
they are not compared as identity before pinning. Exact kind is still required,
regular-file size/mtime checks remain, and later launches revalidate the retained
native identity and revision for directories as well as files.
Per-process revalidation uses immutable captured entry IDs for that process's pin
scope, including an unmarked focus when Once uses it. Provider ownership survives
through the native launch call and uses the existing worker retirement domain.
See [the tree contract](TREE_PANEL.md#native-custom-commands).

## Engine protocol and lifecycle

- `PreviewUserCommand(Box<UserCommandRequest>)`
- `ExecuteUserCommand(Box<UserCommandRequest>)`
- `CancelUserCommand(UserCommandRequestId)`
- `Event::UserCommand(Box<UserCommandEvent>)`
- `engine.configure_user_commands()?.install(service)`

Requests contain the stable command ID, captured configuration and native panel
context. Native/virtual locations are explicit `Option<PathBuf>` values; do not
manufacture archive-member filesystem paths or native identities in UI code.
Expansion, metadata pinning, native validation, process creation and cleanup run
on backend services. Preview and execution are distinct intents; Execute prepares
the captured configuration again, rather than replaying a displayed string.

Request IDs are nonzero, strictly increasing during an engine session, and not
reused after rejection. Allocate the submission ID at actual submission; earlier
asynchronous selection capture can finish out of order. Rejected admission is
distinct from the lifecycle of an original duplicate-ID request.

Updates are Queued, Preparing, Preview, Started, Rejected and Finished. Successful
**Preview is terminal**: it has no subsequent Finished. Failure/cancellation uses
Finished. Preview returns `Arc<UserCommandPreview>` with guarded rows; each
`preview.processes[i].process` contains raw resolved data. Row Arcs retain their
quota even after the parent preview is dropped. Retire prior previews/rows when
replacing them; the UI must not accumulate old preview payloads as job history.

Started.index is zero-based. Finished summarizes dispatch: planned, started,
not-started, cancellation and an optional error. It does not wait for external
applications to exit. Native preflight validates every plan before the first
PerItem spawn. Later external changes or OS startup failures can still produce a
partial dispatch, reported with exact started/not-started counts, without retrying
or killing programs already started.

Cancellation stops preparation and future spawns. Both launch/process permits
and metadata pins stay owned through an in-flight blocking native call. Normal
engine shutdown signals cancellation immediately and awaits actual native cleanup
before aborting unrelated tasks. The existing desktop reaper retains each direct
child's slot until confirmed exit; an uncertain wait error retains the child and
slot for another timed poll, including when the normal reaper has closed. Normal
runtime shutdown retains external-child ownership through the bounded native
tracker until exit is observed; it does not kill those children. Terminals, external
editors and Custom Commands share eight direct-child slots. Associations retain
their separate helper pool.

## Bounds and practical limits

Configuration is bounded by 128 commands, 64 stored Program templates and 1 MiB
aggregate command data inside the existing 4 MiB settings envelope. Captures
contain at most 4096 targets. Expanded argv is separately capped at 4160 entries;
expansion cardinality is checked before allocating an oversized vector. Native
vector storage and raw data count toward the 8 MiB expanded-process/plan budget.
Native Windows command-line and Linux ARG_MAX/per-string limits are additionally
validated; these OS limits can reject a workload below the application ceiling.
The limits are workload policies, not path truncation or RSS guarantees.

The actor has at most 32 admitted/retained work leases and a 64 MiB target/result
budget. Each request conservatively reserves 16 MiB for bounded pins/plans before
queueing; four retained previews can therefore exhaust memory admission first.
Full admission returns a visible rejection. One preparation/activation worker
uses existing shared I/O/process admission, rather than creating a private pool.
PerItem checks its own relevant guards before spawn, avoiding quadratic whole-job
metadata revalidation.

Native target/directory pins detect changes after backend preparation; captured
kind/size/modified facts reject known earlier changes. The first backend open
establishes native identity. Neither panel listings nor arbitrary external tools
are promised immutable path snapshots. Archive/provider argument exports and
native macOS implementation/validation remain outside this backend increment.

Implementation and evidence are recorded in
[backend verification](../verification/2026-11-07-user-commands-backend.md).
