# Native viewer host

This crate is the backend boundary for trusted, in-process viewer plugins. It has
no UI dependency and contains no concrete parser or statically linked viewer.
Native code has the application's privileges; ABI checks are not a sandbox.

`Runtime::discover(resolved_executable_directory)` reads only direct packages under
`viewers/`. Call discovery, `CapturedFile::capture`, `preferred_basic_mode`, and
`Runtime::open` on backend workers. Captures preserve native paths and stable
identity/metadata without retaining a handle for every panel entry. Display names
are lossy display-only strings; they never authorize access to a path.

Native manifests may provide an optional `name`: at most 128 UTF-8 bytes,
nonempty after trimming and without control characters. This short presentation
name is separate from the functional `description`; neither changes plugin IDs,
modes, extension routing or ABI identity. Older manifests remain valid. The app
provides readable legacy names rather than using descriptions as list titles.

Discovery delegates a package containing only `image-viewer.json` or only
`web-viewer.json` to its corresponding process host. Those hosts retain manifest
and dependency validation, including errors. Any combination of multiple native,
image or web manifests is unavailable and remains diagnostic evidence. Catalog
discovery does not load or launch a plugin.

Application services first call `installed_executable()` on a backend worker.
Windows can resolve an explicitly registered private copy for an identical shared
host build; trusted portable installations retain adjacent discovery. The trusted
portable check uses the launch file's native final spelling, so letter-case, 8.3,
SUBST and mapped-drive aliases of a protected installation are accepted (and the
canonical path is returned), while reparse points in the entered spelling are
still refused first. Other platforms retain the actual executable's location.
The cached result does not authorize any package or bypass the checks below. See
the [runtime discovery contract](../../docs/developer/RUNTIME_DISCOVERY.md).

The thumbnail and image-viewer hosts pass their application executable path through
`installation_path` before they derive and validate their package root. It returns
an absolute spelling without resolving links; on Windows it uses extended `\\?\`
syntax, keeps the raw UTF-16 and refuses device namespaces.

Discovery publishes each package's modes and extensions; it routes no file. The
viewer engine chooses a viewer from them
([`routing.rs`](../midnight-viewer-engine/src/routing.rs)). No parser or plugin ID
is hardcoded.

An open session owns one read-only regular-file handle. Reads are at most 256 KiB,
use u64 offsets, and compare native handle identity, length and change/write times
before and after each read. This is **LiveValidated**, not an immutable snapshot.
The same-size/same-timestamp replacement test also checks stable handle identity.
Linux refuses final symlinks and special files with nonblocking/no-follow opens;
Windows refuses reparse files and uses native file IDs. macOS is explicitly
unsupported pending its loader/signing and identity implementation.

Windows loads an absolute entry path with DLL-directory/system-directory search
flags. Linux uses absolute `dlopen` with `RTLD_NOW | RTLD_LOCAL`. Package, manifest
and entry-library canonical paths must remain in their expected location. Discovery
and first load also validate native ownership and write/rename permissions. Linux
accepts current-user/root ownership and rejects group/other writes, including unsafe
ancestors; a trusted-owned sticky ancestor such as `/tmp` is permitted. Windows
checks owner and DACL trustees against the current process user, System,
Administrators and the OS-resolved TrustedInstaller service. Installation objects
must not grant other accounts modification rights. Ancestors must not grant rename,
delete-child, DACL/owner or reparse-relevant modification rights; ordinary permission
to create a new directory at `C:\` does not permit replacing existing `C:\Users`.
Conditional/object ACL forms are conservatively refused rather than misinterpreted.
These checks do not prevent trusted owners/administrators or native plugin code
itself from changing their own process or files.

Discovery and first load validate every local package object, including dependency
libraries in subdirectories, under a 4096-entry budget. Redirected paths and
nonregular objects are refused. Checking only the entry library would allow an
otherwise private package to load a shared-writable companion library. Native
plugins still retain application authority and must select trusted dependencies
outside their own package.

Validated libraries are cached by canonical entry path for the process lifetime
(maximum 256). They never unload when a viewer closes; installation/removal or
replacement is applied at the next startup. A manifest identity/version change for
already loaded code is refused. Instance destruction still waits for all accepted
jobs, services, buffers and SDK quiescence. ABI 1 SDK reports this with `destroy`
returning `NOT_QUIESCENT` until ready. No timeout forcibly unloads native code.

Each session has a serial dispatcher, one CPU worker, one I/O callback worker, and
one native read worker. The process permits at most four sessions, including those
closing. CPU/I/O queues each hold 32 jobs; command and read-service queues hold 32
requests. Nonzero affinity is explicitly unsupported. Accepted work is dropped
exactly once even when cancelled before running. QueueFull keeps caller ownership.

The output queue holds at most 64 events and 16 MiB of actual decoded vector/string
allocations (64 MiB over four sessions), with admission stopped early to reserve
terminal capacity. Native content leases have a separate 4 MiB/32-buffer budget,
and the bounded read-reply channel holds at most 32 chunks (8 MiB). ABI payloads
retain the public 1 MiB CBOR/item/depth limits. These are separate transport budgets,
not a claim to bound arbitrary trusted native allocations or total process RSS.

`Session::close` and Drop only signal cancellation. Backend workers release content
and retained output before returning the session quota. A QueueFull service reply
keeps its immutable bytes and admission ticket until retry succeeds or is rejected.
Cancellation discards queued nonterminal results; generation, revision, requested
mode, source bounds, binary byte count, and model structure are checked before
delivery. `Session::is_closed` reports completed backend instance cleanup.

A session-fatal failure (host setup or plugin creation, malformed or mismatched
plugin output, a failed plugin drain or read-service reply, the output budget)
closes the session at once. `drain` returns it first, as a `Failed` event with
`HostEvent::fatal` set (at most one per session) and the offending request's ID,
or `OPEN_REQUEST_ID` (the Open request, 1) for the session itself. Requests still
outstanding when the session failed never complete. Output queued before the
failure, including terminal events of requests that did complete, can still follow
it until the dispatcher discards the queue. When the dispatcher finishes, it returns
the session quota, sets `is_closed`, and then signals the `notify_on_ready` callback
once more even if earlier output was never drained, so a consumer can wait for a
closing session's cleanup without polling. The callback can run on any thread: on
backend threads, the dispatcher included, and on the thread that calls
`notify_on_ready` (a signal is already pending) or `drain` (output remains). It
must only enqueue work: never block, nor take a lock that the consumer holds while
it calls those methods.

## Verification

Normal host tests use the public API and generic native boundaries. Real-library
tests are opt-in so an application checkout does not build a concrete plugin:

```powershell
$env:HOST_TEST_INSTALL_ROOT = Join-Path $PWD '_build\private-viewer-qa'
.\scripts\cargo.ps1 test --offline -p midnight-plugin-host
.\scripts\cargo.ps1 build --offline --manifest-path crates/midnight-plugin-host/tests/fixtures/bad-descriptor/Cargo.toml --target-dir _build/qa/viewer-implementation/host/bad-descriptor
$env:HOST_TEST_PLUGIN = '<basic viewer package folder>\midnight_viewer_basic.dll'
$env:HOST_BAD_PLUGIN = Join-Path $PWD '_build\qa\viewer-implementation\host\bad-descriptor\debug\midnight_host_bad_descriptor.dll'
.\scripts\cargo.ps1 test --offline -p midnight-plugin-host '--' --include-ignored
.\scripts\cargo.ps1 clippy --offline -p midnight-plugin-host --all-targets '--' -D warnings
```

On Linux, the same tests use the independent basic `.so` and the generic wrong-ABI
`.so` in `HOST_TEST_PLUGIN` and `HOST_BAD_PLUGIN`. Use `cargo test ... --
--include-ignored`. The generic fixture has no API/SDK/application dependencies.
The independent basic plugin's test suite remains entirely under its own project.
Windows installation fixtures use `HOST_TEST_INSTALL_ROOT` when provided, then
protect only their freshly created children. A broad shared build-directory ACL
is intentionally unsuitable for native installation tests. Compile artifacts may
remain there; final native test copies must use a private installation and safe
ancestors. Linux CI similarly installs the SO fixture with private file modes on
a native filesystem, rather than treating drvfs's permissive mode bits as trusted.
