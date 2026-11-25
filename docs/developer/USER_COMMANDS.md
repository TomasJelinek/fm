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

Commands are stored from settings schema 17, through the strict, bounded, atomic
settings service and native path codec (`settings/user_commands.rs` in
`midnight-platform-common`). Older settings load an empty command list. A schema 17
payload includes checksum algorithms even when SHA-256 is the default, as schema 16
does. Default preferences encode as version 10. Invalid/null/unknown fields,
duplicates and malformed templates fail validation instead of being silently dropped.
Separators serialize as an invocation with only `mode: "separator"`; Program and
Terminal records keep their format. On load,
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
values, preserving invalid Unix UTF-8 and native Windows UTF-16. On Windows, Run directly accepts absolute
.exe, .com, .bat and .cmd paths. Rust's process API launches batch files with the
Windows command interpreter and quotes their arguments. The captured-output
helper inherits the command's working directory, streams and cancellation job. Unix executable scripts retain OS exec semantics.

Terminal data is exported only through declared fixed keys: `ME_ACTIVE_PATH`,
`ME_OTHER_PATH`, `ME_FILE_PATH`, `ME_FILE_NAME`, `ME_FILE_STEM` and
`ME_FILE_EXTENSION`. Undeclared inherited values of these keys are removed.
Missing declared values fail before any spawn. Script source stays literal;
environment values cannot become recursively expanded template source. Windows
uses an owned, sorted Unicode child environment, including per-drive cwd records
(one `=X:` record for every drive letter that resolves; a drive without a recorded
cwd yields its root). Hidden process-private records inherited from a launching
Command Prompt or the Shell, such as `=ExitCode`, `=ExitCodeAscii` and `=::`, are
dropped; caller-supplied records are never exempt from validation.
Linux uses native environment values on the terminal builder. Parent
environment and process cwd are never changed. Script text is not automatically
added to command history or logs; process debug output redacts source/argv/env.

Windows supports PowerShell and Command Prompt; Linux uses supported installed
POSIX terminal profiles. Custom launches do not add an elevation/credential path.

Custom Command Prompt uses the trusted system Cmd with `/D /U /E:ON /V:ON /S /K`.
Its source is one literal command line: use explicit Cmd operators between
commands, or choose PowerShell for multiline scripts. CR/LF in source is rejected
because native command-string mode otherwise ignores subsequent physical lines.
This restriction does not affect control characters inside captured data.
The ordinary command-bar Cmd profile uses `/V:OFF` (`desktop.rs` in
`midnight-platform-windows`).

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
variables and alter carets; arbitrary pipeline/grouped reparsing is not supported.
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
script variables remain subject to Cmd's own limits. The aggregate 8 MiB child
environment limit applies (`MAX_UNITS` in `user_command_environment.rs`); no value is
truncated. Evidence: [Cmd record](../verification/2026-11-12-cmd-custom-commands.md).
Windows loader errors use a scoped thread error
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
through the native launch call and uses the backend retirement domain.
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
before aborting unrelated tasks. The desktop reaper retains each direct
child's slot until confirmed exit; an uncertain wait error retains the child and
slot for another timed poll, including when the normal reaper has closed. Normal
runtime shutdown retains external-child ownership through the bounded native
tracker until exit is observed; it does not kill those children. Terminals, external
editors and Custom Commands share eight direct-child slots (`MAX_PROCESSES` in
`user_command_process_tracker.rs`). Associations retain
their separate helper pool.

## Program output

A Run directly (Program) command has an output choice, `UserCommand.output`
(`UserCommandOutput { mode, close_on_success }`); terminal scripts and separators keep
the default, and validation refuses anything else. Settings schema 39 stores it as
`invocation.output` of a Program record (`{"window": "hidden" | "output" | "console",
"close_on_success": true}`, written only when it differs from Hidden); every command
saved before schema 39 reads as **Hidden**, its earlier behavior. New commands start
with **Output window** (`UserCommandOutput::NEW_PROGRAM`): a program run directly
otherwise has no visible result, and the window adds no console of its own.
All three choices pass one argument per expanded row to the process API. Native
executables receive these as argv. Windows interprets explicitly selected `.bat`
and `.cmd` files using its batch syntax.

- **Hidden** — `launch_user_program`: no window, standard streams on the null device.
- **Output window** — `launch_user_program_captured` returns a
  boxed `CapturedUserProgram` (a trait object): no window, standard input the null device (`NUL`,
  `/dev/null`; reading it ends at once, so a program never waits for input), standard
  output and error in two pipes, and the process tree in one group. Windows starts the
  program suspended, assigns it to its own job object (no kill-on-close) and only then
  resumes it, with exactly the three standard handles inheritable
  (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`) under the shared process-creation lock; Linux
  makes it the leader of a new process group. Stop ends the tree: `TerminateJobObject`,
  or `SIGTERM` to the group followed by `SIGKILL` after three seconds
  (`STOP_GRACE`). Background processes still running after the program itself exited
  are left alone.
- **Console window** — `launch_user_program_console` starts the application's own
  launcher, `midnight-explorer --run-and-wait <program> [argument…]` (`RUN_AND_WAIT_ARGUMENT`),
  in a new console (Windows, `CREATE_NEW_CONSOLE`, without inherited handles) or in the
  terminal emulator the Run in terminal mode already uses (Linux, after its execute
  option). Each argument is one native argv element of the launcher, which starts the
  program with the same elements (`std::process::Command`), shows the command line,
  waits, shows the exit code and duration and waits for a key (Windows: console input
  without Shift/Ctrl/Alt alone; Linux: one byte with the terminal out of line mode).
  Ctrl+C reaches the program, not the launcher. The application sets the launcher at
  startup (`set_console_launcher`); without it the mode reports Unsupported. The
  program can read the console's input.

The engine runs an output window's runs on a task of their own, one after another (a
per-item command shows each item's run in turn in the same window); the actor takes
the next request meanwhile. Every run still takes the shared process admission
(`Runtime::reserve`), so the eight direct-child slots bound them too. A supervisor
thread per running program (`user_command_output.rs` in the engine) holds the
process permit until the program exits, polls its exit and Stop every 50 ms, and
two reader threads drain the pipes in 64 KiB reads as fast as the program writes: the
program never blocks on a full pipe because of the UI. After the exit the readers get
one more second (`DRAIN_GRACE`) for the last output. Cancelling the request (Stop,
Jobs › Cancel, closing the window while it runs, engine shutdown) ends the running
tree and starts no further run.

Output passes through `TerminalLines` (core): ANSI/VT escape sequences (colours,
cursor movement, OSC titles and links) are removed, a carriage return not followed by
a line feed starts the line again (progress counters), backspace removes a character,
other control characters are dropped, tabs expand to eight columns and a line longer
than 16 KiB is wrapped. Each complete line is decoded on its own: UTF-8 when valid,
else on Windows the OEM code page of the program's hidden console (`CP_OEMCP`) and on
Linux UTF-8 with replacement characters. The environment is not changed for this.

`UserCommandOutputSession` (core) is shared by the backend, which appends, and the UI,
which reads what changed after its last line with `read_since`. It keeps the last
50,000 lines and at most 8 MiB of text (`MAX_OUTPUT_LINES`, `MAX_OUTPUT_BYTES`),
counts dropped lines, and keeps each stream's unfinished line apart. The UI hears
about news through `Event::UserCommandOutput`: `opened` once, then at most one
notification until it has read (`take_notification`), at most every 50 ms; a full
event queue is retried. At most 16 output windows are open
(`MAX_OUTPUT_SESSIONS`); beyond that the command is stopped before it runs and
reported. Save as… sends `Command::SaveUserCommandOutput`; the settings-file worker
publishes the session's text through verified native staging under the namespace
gate, refuses an existing file with AlreadyExists and replaces it (keeping the
replaced file for recovery where the filesystem can) only when the request carries the
user's consent; `Event::UserCommandOutputSaved` answers.

Tests: `user_command_output_tests.rs` in core (terminal lines, bounds, session),
`user_command_output_tests.rs` in the engine (supervision with fake programs: streams,
a 64 MiB flood, Stop and its forced kill, notifications) and
the engine's `tests/user_command_output_run.rs` (a real program end to end, per-item runs, Stop,
Save as), and `user_command_output_tests.rs` in each platform crate (literal argv
with spaces, quotes, empty and Unicode arguments in both modes, empty input, exit
codes, interleaved streams, a 36 MiB burst, Stop ending a grandchild; on Windows
the hidden mode's null output device as the evidence that output was invisible).

## Bounds and practical limits

Configuration is bounded by 128 commands, 64 stored Program templates and 1 MiB
aggregate command data (`MAX_USER_COMMANDS`, `MAX_USER_COMMAND_ARGUMENTS`,
`MAX_USER_COMMAND_CONFIG_BYTES`) inside the 4 MiB settings envelope. Captures contain at
most 4096 targets. Expanded argv is separately capped at 4160 entries
(`MAX_USER_COMMAND_EXPANDED_ARGUMENTS`);
expansion cardinality is checked before allocating an oversized vector. Native
vector storage and raw data count toward the 8 MiB expanded-process/plan budget
(`MAX_PLAN_BYTES` in `user_command_prepare.rs`).
Native Windows command-line and Linux ARG_MAX/per-string limits are additionally
validated; these OS limits can reject a workload below the application ceiling.
The limits are workload policies, not path truncation or RSS guarantees.

The actor has at most 32 admitted/retained work leases and a 64 MiB target/result
budget (`MAX_JOBS`, `MEMORY_BUDGET` in the engine's `user_commands.rs`). Each request conservatively reserves 16 MiB for bounded pins/plans before
queueing; four retained previews can therefore exhaust memory admission first.
Full admission returns a visible rejection. One preparation/activation worker
uses the shared I/O and process admission, rather than creating a private pool.
PerItem checks its own relevant guards before spawn, avoiding quadratic whole-job
metadata revalidation.

Native target/directory pins detect changes after backend preparation; captured
facts reject known earlier changes: a tree capture compares kind, size and
modified time, and a flat panel selection compares its listed file size and
whether the row was a directory (a file replaced by a directory, or the reverse,
is rejected; links keep their target-independent tolerance). The first backend
open establishes native identity. One plan holds at most 256 native pins at a
time (`MAX_HELD_PINS`) (each is an open descriptor on Linux, whose common soft limit is 1024).
Later targets/directories are pinned once to capture their identity and facts,
released, and pinned again for each revalidation. A released target must find the
same native identity, kind, size, modification time, native attributes and Unix
mode, for a directory target as well as a file; a released working or context
directory must find the same identity, as a held one does. Unlike a held pin, a
released target cannot compare the change time (Linux ctime, Windows ChangeTime)
or the Windows creation time, so a change that restores every compared fact
before the check goes undetected for a released target. Preparation pins and
checks every target once; the actor then runs one complete revalidation after
native preflight, and each process revalidates only its own scope before
spawning. Neither panel listings
nor arbitrary external tools are promised immutable path snapshots. Known limits:
archive and provider arguments are not exported, and there is no macOS implementation.

Evidence: [backend record](../verification/2026-11-07-user-commands-backend.md).
