# External editor verification

F4 captures the focused regular local file and submits one bounded backend request,
independent of multiselection. Directory, parent-row, loading and filtered-search
guards reuse the panel's activation rules. Saved settings must load successfully
before F4 can launch an editor. Text inputs retain their keyboard focus guards.

Options > Editors displays the backend-resolved default. An override consists of
an absolute executable path and a JSON array of literal argument strings. Only a
complete `{file}` argument becomes the raw native path; embedded placeholders,
quotes, spaces, Unicode and shell metacharacters remain literal. NUL, missing
placeholders and excessive argument counts or aggregate bytes are rejected. The
existing native-path settings codec preserves executable paths losslessly. Editor
settings were introduced in preferences version 9; versions 1 through 8 migrate to
the default editor.

Windows queries `GetSystemDirectoryW` and selects `notepad.exe` there. Custom
Windows executables must be absolute `.exe` or `.com` paths; batch scripts are
rejected. Linux checks executable regular files at fixed `/usr/bin` locations in
this order: gnome-text-editor, gedit, kate, kwrite, mousepad, xed, pluma, leafpad.
Neither platform searches the panel directory or PATH. Linux desktop-file command
parsing and terminal-editor wrapping are intentionally absent; users can select
another graphical editor explicitly. macOS remains a later platform.

Metadata validation, default discovery, spawning and child reaping run on backend
workers. Launch permits cover the native work and delivery of its result, then
release immediately; a separate bounded process slot remains until child exit.
Stalled UI delivery applies backpressure to rejected requests. A missing or invalid
explicit editor reports an error without a default fallback. Successfully spawned
editors own subsequent application-level error reporting; their later nonzero exit
status is reaped but is not presented as another file-manager notification.

The native file is revalidated as a regular file immediately before spawning. An
external editor receives a path, so the file manager cannot provide a transaction
across later path replacement or control the editor's own behavior. Archive members
and remote-provider resources need the future staged writeback contract.

## Evidence

Logs live under `_build/qa/viewer-implementation/editor/`.

- `core-red.log`, `settings-red.log`, `windows-red.log`: observed failures before
  literal substitution, settings persistence and native validation were implemented.
- `engine-red.log`, `engine-bounded-red.log`: observed dispatch and backpressure
  failures; `engine-green.log` passes the two corresponding tests.
- `ui-red.log`, `restore-red.log`: observed presenter/settings/draft-reset failures.
- `windows-suite.log`: 472 passing core, common, native Windows and engine tests.
- `ui-options.log`: actual Slint Options controls and F4 focus-guard checks.
- Native tests inspect prepared argv and launch only the test harness as a safe
  child, never a desktop editor. Linux additionally tests raw non-UTF-8 filenames
  and unavailable/nonexecutable/relative default candidates.
- The first broad run encountered a rustdoc 1.99.1 internal compiler error resolving
  core imports in the common crate. It did not reproduce in either a direct repeat
  (`rustdoc-repro.log`) or a clean isolated target (`rustdoc-isolated.log`); both
  completed successfully. No baseline regression claim is made.

Salamander source inspected for behavior: `src/fileswn5.cpp:1228` obtains the focused
file, rejects non-disk panels, selects an editor, and reports process-creation
errors. Its ANSI expansion/command-line construction is not copied; the new
service passes native executable and argv values directly.
