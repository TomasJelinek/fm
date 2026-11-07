# Custom Commands backend verification

Backend/parser/persistence/native process/controller implementation, 7 November
2026. Frontend editor/toolbar/activation integration is owned separately; these
results do not by themselves mark the complete user-visible workflow implemented.

No new dependency, feature flag or plugin ABI was introduced. The implementation
uses std, existing Tokio, existing native Windows bindings and existing libc.
Native implementations and tests remain in separate Windows/Linux files. Test
helpers and all generated output live under `_build/goal/commands/`; no real user
preferences, application settings or ordinary file-manager launch is used.

## Observed RED and behavior checks

Initial parser tests failed on the missing parser module; model/shortcut/process
tests failed on missing contracts; settings failed on the missing schema field;
preparation and actor tests failed on missing worker/protocol hooks. Native tests
failed on missing process service/environment methods before implementation.

Core cases cover literal nonrecursive values, malformed/grouped variables, raw
Unix bytes, native Windows UTF-16, long values without truncation, missing/NUL/
oversized data, stable IDs, exact deleted-ID shortcut cleanup and static/dynamic
shortcut conflicts. Native preparation covers focus versus selected items,
vanished selections, metadata revalidation, literal script plus explicitly
declared environment values, disabled-preview versus disabled-execution policy,
and a 65-file grouped selection beyond the stored 64-template bound.

Schema17 tests round-trip ordered commands/native paths, preserve checksum
settings/defaults and reject null/unknown/duplicate/cross-version/malformed data.
Terminal source is not treated as a template; explicit scalar declarations round
trip and grouped terminal declarations are rejected.

Actor tests cover preview without launch, visible admission rejection,
nonreused request IDs, honest dispatch failure summaries, full-queue behavior,
retained row quota and cancellation/shutdown while a native call is deliberately
blocked. Both process and launch permits remain unavailable until native return.
Successful preview is a terminal update, with no later Finished event.

## Native process evidence

Owned Rust helper processes report the argv, cwd and fixed environment actually
received, using length-prefixed native byte/UTF-16 records. Program tests include
quotes, dollar/semicolon/ampersand characters, Unicode, long arguments and raw
Unix names. No source-like filename creates the owned sentinel file.

The Windows environment encoder tests case-insensitive replacement, removal of
undeclared inherited context, sorting, double-NUL termination, raw UTF-16 and
invalid key/NUL/duplicate refusal. An actual hidden PowerShell fixture starts an
owned helper and preserves environment data without evaluating its contents.
Cmd Custom Commands are explicitly rejected before process creation.

The Linux terminal fixture uses the existing terminal command builder, an owned
terminal helper and actual `/bin/sh`, preserving invalid UTF-8 environment data
and cwd. It does not qualify interactive GUI terminal discovery on every desktop.

One initial negative Windows fixture mistakenly created invalid bytes as
`missing.exe`, and the loader call hung in native bad-image handling. Only its
verified owned build-test process was stopped. The missing-path fixture now stays
absent. A bounded subprocess test exercises the same malformed image with scoped
`SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX`: it returns a native error promptly
and proves that the calling thread's original error mode was restored. The
subprocess is killed by its parent if the 10-second bound is exceeded. Its ignored
fixture entry is invoked by the parent test, not omitted from qualification.

## Qualification record

Windows and Linux each passed the final 20 engine integration cases: actor2,
contracts4, preparation8 (including three reused file-selection checks),
process-data1 and parser5. Each also passed four actor ownership/quota/lifecycle
units and three schema codec tests. Preparation tests exercise scoped
revalidation and preview conversion rather than suppressing unused-method lints.

Windows passed actual Program3 plus its owned fixture, environment encoder2 and
hidden PowerShell/Cmd-policy2. Linux passed actual Program1 and the owned POSIX
terminal/environment fixture1. Strict Clippy passed for all targets of core,
common, engine and each native platform crate. Exact command output is retained
in `_build/goal/commands/{final-engine,actor-units,settings-green,clippy}-*.log`,
`native-green-*.log`, `terminal-green-*.log` and `clippy-native-linux.log`.

A separately reviewed process-tracking fallback regression failed causally on
Windows: the first wait error returned the process permit while exit was unknown.
The corrected fallback retains child and quota on Err or Ok(None), polling at
100 ms like the normal desktop reaper; only Ok(Some(status)) returns quota. The
gated mock sequence Err/Ok(None)/Ok(Some) passed, and engine all-target strict
Clippy passed again (`runtime-wait-{red,green,clippy}-windows.log`). Normal engine
shutdown may abandon tracking external children; it does not kill them or wait
for their exit. The same final regression and engine all-target strict Clippy
also passed on Linux (`runtime-wait-{green,clippy}-linux.log`).

Primary native contracts consulted: Microsoft documents
[explicit process application/cwd/environment](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw),
[environment sorting and child blocks](https://learn.microsoft.com/en-us/windows/win32/procthread/changing-environment-variables)
and [thread-scoped error handling](https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-setthreaderrormode).
Linux's [execve limits](https://man7.org/linux/man-pages/man2/execve.2.html) describe
ARG_MAX and per-string limits used in native preflight. No reference implementation
code/assets were copied. Salamander reference review is recorded in the owned
implementation plan under `_build/goal/commands/`.

macOS has no implementation/native runner in this increment. Source identity
begins at backend preparation; no immutable-name guarantee is made for arbitrary
external applications. Only launch/dispatch is summarized; long-lived external
process completion is handled by existing reaping rather than delaying dispatch.
