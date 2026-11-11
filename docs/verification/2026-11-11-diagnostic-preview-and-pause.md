# Diagnostic preview and cooperative job pause verification — 2026-11-11

Authoritative workspace: `D:/experiments/dev/FM`. These checks cover backend behavior;
final native window interaction and packaged application checks are recorded separately.

## Captured diagnostic preview

`ReadDiagnosticText` carries the immutable captured `GeneratedTextSource`, a desktop
request ID and cancellation. `DiagnosticTextReady` returns the same ID with either a
UTF-8 preview or a typed error. The existing warning-report pool handles both request
kinds: two blocking workers and eight queued requests. Reads use at most 64 KiB total,
in chunks of at most 8 KiB. A truncated final Unicode scalar is omitted; malformed
interior text, short reads and changed source lengths are errors. A cancelled request
never publishes a late diagnostic result. Source reads and final ownership retirement
stay off the caller/UI thread. The UI retains the captured source for full-copy, subject
to the existing 4 MiB clipboard limit, and labels a truncated preview explicitly.

Commands (with the existing Windows build environment, explicit offline SDK config,
`--target-dir _build --locked --offline`):

- `cargo test -p midnight-engine --lib diagnostic_text`: **6 passed**.
- `cargo test -p midnight-engine --lib size_warning_requests`: **7 passed**.

Coverage includes Unicode boundaries, an advertised `u64::MAX` source length without
whole-source reads, cancellation before/during reads, exact request correlation,
queue saturation, off-caller destruction, shutdown, and a source panic followed by
successful reuse of the worker pool. Logs are in
`_build/qa/viewer-regressions-20261111/diagnostic-*-tests.log`.

## Pause, resume and cancellation

`SetCopyPaused` and `SetOperationPaused` retain separate exact operation-ID spaces.
Only Copy/Move and Delete jobs accept these controls. `pause_requested` acknowledges
command admission; `paused` becomes true only at a safe backend checkpoint. Neither
field means an OS call was suspended. Copy/Move wait between existing bounded job
steps and before preparation attempts. Native deletion checks between traversal and
mutation steps. Windows recycle waits before the final path identity revalidation;
a running indivisible Shell recycle call finishes before a pause can be acknowledged.

`Cancellation::is_cancelled` remains nonblocking. Only explicit worker checkpoints
wait. Resume and ancestor cancellation are checked on a 10 ms polling cadence.
Cancellation wins over pause; cleanup and terminal publication proceed, and
terminal progress clears both pause flags. Transfer sampling stops while paused and
restarts at the actual resume boundary. Discovery retains the same job cancellation
and pause control. Paused preparation retains its existing worker and I/O permit until
resume/cancellation; this may defer jobs sharing that budget and never starts replacement
workers. No UI thread waits and no application thread is suspended or killed.

Targeted Windows commands:

- `cargo test -p midnight-core --lib provider::pause`: **2 passed**.
- `cargo test -p midnight-engine --lib pause`: **6 passed**, including four new job
  control tests and two existing related tests.
- `cargo test -p midnight-platform-windows --lib mutation::pause_tests`: **1 passed**.

Native Linux command, using the existing WSL Ubuntu-24.04 Rust 1.99.1 toolchain,
`_build/linux`, and the explicit Linux SDK config:

- `cargo test -p midnight-platform-linux --lib mutation::pause_tests`: **1 passed**.

The native deletion test runs both resume and cancel cases. It pauses after exactly
one child is removed, checks that the count does not advance while paused, then either
finishes all four nodes or cancels with the two remaining files preserved and no
staging residue. Engine tests also cover a blocked in-flight copy (Pausing, not a false
Paused acknowledgement), cancellation cleanup of its partial target, Copy and Move
resume, queued Delete, and stale/wrong job IDs. Logs are in
`_build/qa/pause-resume-20261111/`.

## Linux authorization transport

The Linux helper uses the fixed system `pkexec --disable-internal-agent` executable,
a random exclusive 0700 socket directory, kernel `SO_PEERCRED`, exact child/parent
PIDs, parent creation time, invoking UID, and a nonce handshake before the request.
Two child reservations include pending authorization, active helpers, and cancelled
children not yet reaped. Disconnect/deadline monitoring exits only the dedicated
read-only helper process, including when its native directory read is blocked.

Cancellation inside framed I/O uses `ConnectionAborted`: `read_exact` and `write_all`
retry `Interrupted`, which otherwise caused a demonstrated cancellation loop.

Native Linux `cargo test -p midnight-elevation`: **13 passed**, with one ignored
subprocess-only test entry explicitly exercised by its parent test. Strict
`cargo clippy -p midnight-elevation --all-targets -- -D warnings`: **passed**.
Tests cover raw non-UTF-8 names, kernel peer credentials, wrong PID/UID/start time,
nonce rejection, private socket cleanup, buffered replies after peer close, truncated
frames, read cancellation/deadline, bounded child reaping, and helper termination
on parent disconnect. Logs: `_build/qa/elevation-linux-20261111/`.

The WSL installation has no `pkexec`/desktop polkit agent, so a live authorization
prompt and elevated directory listing were **not** exercised there. The implementation
uses the documented [polkit launch contract](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html),
[Unix peer credentials](https://www.man7.org/linux/man-pages/man7/unix.7.html), and
[process creation-time field](https://www.man7.org/linux/man-pages/man5/proc_pid_stat.5.html).


## Final affected Linux suites

The five affected backend packages were compiled together with `--locked --offline
--tests --no-run`, then all 110 unit/integration test executables ran with two test
threads. The existing native Linux target and SDK configuration above were reused.
The initial run produced:

| Package | Passed | Failed | Ignored | Execution seconds |
| --- | ---: | ---: | ---: | ---: |
| midnight-core | 183 | 0 | 0 | 1.184 |
| midnight-platform-common | 183 | 2 | 0 | 4.499 |
| midnight-platform-linux | 209 | 0 | 14 | 8.528 |
| midnight-engine | 1077 | 6 | 11 | 61.616 |
| midnight-elevation | 13 | 0 | 1 | 0.100 |

All eight failures were test fixture assumptions, corrected without changing production
behavior or weakening assertions:

- The archive color expectation now matches the intentional yellow default.
- The version-26 file-list fixture encodes a genuinely nondefault template, ensuring all
  required legacy fields are present. Its strict malformed-input check also passed.
- Linux rename tests use native temporary storage under `_build`, because WSL's Windows
  mount cannot satisfy the existing required 0700 mutation storage permissions.
- File-action tests skip the unrelated startup `NetworkAvailability` event while retaining
  exact request/result assertions and rejecting other unexpected events.

Only the affected failures were rerun: preference roundtrip **1 passed** (0.108 s),
strict preference validation **1 passed** (0.092 s), archive color **1 passed** (0.099 s),
rename decisions **4 passed** (0.203 s), and file actions **2 passed** (0.287 s).
The final engine `discovery` filter passed **8 tests** (0.228 s), including the two
additional current-file progress regressions added during the initial build.

Reconciled coverage is **1675 passed, 26 existing ignored, no unresolved failures**.
This combines the full run with focused correction checks; the full suite was not repeated.
The ignored elevation entry is the subprocess-only helper exercised by its parent test.
Other existing ignores cover isolated desktop/locale/trusted-installation fixtures and explicit
performance probes.
These execution times are test-run durations under concurrent build load, not benchmarks.

Commands, per-target logs, and machine-readable original/focused summaries are in
`_build/qa/backend-final-linux-20261111/` (`run.sh`, `retest.sh`, `summary.json`, and
`retest-summary.json`).


## Windows backend qualification and Shell environment regression

The final affected Windows build supplied 124 test executables for core, engine,
platform-common, platform-windows, plugin-host, thumbnail-host, and elevation. They ran
with four concurrent targets, one test thread each, isolated temporary directories,
a protected `HOST_TEST_INSTALL_ROOT`, and the pinned absolute `RUSTC` path.
The first batch completed in **328.695 seconds**:

| Package | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| midnight-core | 187 | 0 | 2 |
| midnight-engine | 1109 | 0 | 12 |
| midnight-platform-common | 185 | 0 | 0 |
| midnight-platform-windows | 257 | 1 | 14 |
| midnight-plugin-host | 59 | 0 | 6 |
| midnight-thumbnail-host | 50 | 0 | 13 |
| midnight-elevation | 7 | 0 | 0 |

All eight Linux-discovered fixture corrections also passed in this Windows build.
The remaining failure was independently reproduced by running native Shell context-menu
initialization immediately before the custom PowerShell environment test. The terminal
test passed alone. Exact-name bisection isolated
`shell_menu_real_shell_resolves_unicode_multiselection_without_invoking` as the trigger.

A temporary test-only diagnostic identified an added UTF-16 key `=::` (units 61, 58, 58),
with no NUL in its value. No environment value was printed or assigned undocumented
semantics. The diagnostic has been removed. Snapshot capture now omits only that exact,
NUL-free inherited record before fixed caller overrides are added. The block encoder
remains strict: it rejects ordinary names containing `=`, unknown private names, NUL,
and case-insensitive duplicate names; the fixed override whitelist is unchanged.
Canonical drive records such as `=C:` remain preserved.

Microsoft documents ordinary `name=value` records and the explicit handling of canonical
per-drive records in [CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw).
This fix does not broaden that encoder contract to arbitrary hidden names or assume a
meaning for the observed `=::` value.

Two new environment regression tests exercise omission during capture, strict direct
encoding, prohibited overrides, retained drive/ordinary values, malformed names, NUL,
and ordinal duplicate rejection. The final focused Windows build passed, followed by:

- Full `midnight-platform-windows` library suite: **174 passed, 12 existing ignored**.
  This includes the real Shell-menu initialization followed by custom PowerShell.
- `user_command_environment` integration target: **4 passed**.

Both targets used fresh isolated temporary directories. The batch took **17.999 seconds**;
per-target process times were 17.455 and 2.036 seconds. The library test harness itself
reported 10.98 seconds. These are verification durations under concurrent build/test load.
No duplicate separate Shell-to-terminal run was needed after the full library passed.

Reconciled Windows backend coverage is **1857 passed, 47 existing ignored, no unresolved
failures across 124 targets**, combining the original batch with these focused correction
checks. Final per-target logs, exit codes, and counts are under
`_build/qa/viewer-settings-20261111/environment-final-20261111-225301-*`.

Initial results and durations are preserved under
`_build/qa/viewer-settings-20261111/backend-final-20261111-221414-*`.
The minimal failing sequence and key-only diagnostic are preserved in
`environment-shell-terminal-before.log`; bisection and isolated-pass evidence use the
`environment-*` files in the same QA directory.
