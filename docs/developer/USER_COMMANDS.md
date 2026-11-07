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
Program or Terminal invocation. Duplicate labels are allowed; duplicate IDs are
invalid. Executable/icon literals remain native `PathBuf` values.

Program mode stores an explicit absolute executable plus at most 64 argument
templates. Terminal mode stores literal user-authored script, shell flavor and an
explicit scalar `variables` list, empty by default. The picker must add a required
variable declaration in the same accepted edit as its script reference. Manual
script authors declare required context too; the backend never scans or rewrites
shell source to infer requirements.

Schema 17 uses the existing strict, bounded, atomic settings service and native
path codec. Older settings default to an empty command list. A schema17 payload
includes checksum algorithms even when SHA-256 is the default, preserving the
schema16 contract. Default preferences still encode as version10. Invalid/null/
unknown fields, duplicates and malformed templates fail validation instead of
being silently dropped. No real user preference file is used by backend tests.

Apply a list edit through `UserPreferences::replace_user_commands`. It validates
the entire candidate and removes shortcut overrides for exactly the deleted IDs
before replacing the accepted value. A failed edit preserves the previous value.
`ShortcutMap::with_user_commands` adds only configured `user-command.<id>` actions
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

Windows supports the existing PowerShell profile; Linux uses supported installed
POSIX terminal profiles. Cmd Custom Commands return explicit Unsupported until
hostile native environment-reference qualification is implemented. This does not
change the existing ordinary command-bar Cmd behavior. Custom launches do not add
an elevation/credential path. Windows loader errors use a scoped thread error
mode, restored on return, to report errors without blocking loader dialogs.

## Engine protocol and lifecycle

- `PreviewUserCommand(Box<UserCommandRequest>)`
- `ExecuteUserCommand(Box<UserCommandRequest>)`
- `CancelUserCommand(UserCommandRequestId)`
- `Event::UserCommand(Box<UserCommandEvent>)`
- `Engine::with_user_commands(Arc<dyn UserCommandService>)`

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
runtime shutdown may abandon external-child tracking. Terminals, external
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
