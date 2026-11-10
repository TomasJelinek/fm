# Built-in native content capture fixes — 7 November 2026

The shared viewer-host capture boundary now preserves literal Windows filenames
and qualifies Linux regular files before opening content. This also covers
existing Compare, thumbnail and archive-source users of `CapturedFile`; it does
not claim their complete product flows were requalified.

Windows converts accepted absolute drive/UNC addresses to verbatim OS paths,
retains the original raw provenance, and refuses device namespaces, NUL and dot
steps. File type/reparse and identity checks remain. Linux opens a metadata-only
O_PATH/no-follow probe through directory descriptors, qualifies a regular inode,
then reopens that retained inode through the internal proc descriptor namespace.
The source pathname is never reopened for data before qualification. The walk
preserves native bytes and supports paths beyond PATH_MAX.

Direct rustc prototypes include the actual production native/content modules.
Windows RED read sibling `source` when requesting literal `source.`; GREEN reads
the selected literal bytes. Linux RED rejected a substituted FIFO after inotify
observed IN_OPEN; GREEN rejects it without opening it for data. The owned fixtures
and logs are under `_build/goal/native-plugin/app-open-audit/`, with ext4 Linux
syscall fixtures under a dedicated `_build` temporary tree.

Host regressions additionally cover dotted/spaced/device-looking Windows names,
raw UTF-16 and >320-character paths; Linux checks no FIFO IN_OPEN during stale
capture revalidation, inode-pinned reopen after FIFO substitution and a >4300-byte
raw pathname. Focused Windows Cargo tests passed all three new cases and the
existing replacement-identity case. Linux passed 33 unit and one integration
test, including all three new native cases; five preexisting installed-plugin
fixture tests remain ignored. All-target strict Clippy passes for plugin-host
on both platforms. Logs are `host-focused-windows.log`, `host-tests-linux.log`
and `host-clippy-{windows,linux}.log` in the same audit directory.

The full cached Windows host suite was rerun with the explicitly supplied secure
`HOST_TEST_INSTALL_ROOT`: 46 unit and one integration test passed; five existing
installed-plugin fixture tests remain ignored. Logs are
`host-protected-windows.log` and `host-integration-windows.log`. The initial
unconfigured run's 22 installation/ACL fixture failures are retained in
`host-tests-windows.log`; the protected-root rerun resolves that environment gap.

No plugin trust checks, Copy/Move name validation, public APIs, dependencies or
independent native filesystem package files change. Retained readers remain
LiveValidated rather than immutable snapshots. Existing cancellation interfaces
and positioned broker reads are unchanged. No deployment or commit was performed.
