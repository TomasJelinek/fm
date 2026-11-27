# Authorization sessions: one consent for a folder and its subfolders

Automated evidence for the
[authorization session](../../SECURITY.md#authorization-session): after **Retry with
authorization**, the helper stays as a session that lists the authorized folder and the
folders inside it without another prompt, refuses everything else, and ends when idle,
too old, closed or disconnected. No test triggers a UAC or polkit prompt or runs anything
elevated: the helper's containment and session loop run unelevated in the test process
against temporary folders, over a real named pipe (Windows) or Unix socket pair (Linux),
and the engine and presenter use fakes of the session service.

## What the tests show

- **Protocol** (`crates/midnight-elevation/src/protocol.rs`): version 2 frames carry a
  whole session in order (listings, refusals of each kind, close); oversized, truncated
  and malformed frames, unknown request kinds, trailing data and version 1 frames are
  refused.
- **Containment** (`windows_tests.rs`, `linux_tests.rs`, `helper.rs`,
  `windows_listing.rs`, `linux_listing.rs`): the first listing establishes the root; the
  root, a child, a grandchild and a link or junction whose target is inside the root are
  listed; the parent, `..` spellings, a sibling, a junction or symbolic link (absolute or
  relative) to a sibling, a folder below such a link, a junction to the system folder,
  another drive by name and, on Linux, the separately mounted `/proc` below `/` are
  refused as outside, each keeping the session; a file and a missing folder inside the
  root are refused for their own reasons; a failed first listing establishes no session.
- **Session over the channel**: many requests over one channel; the helper refuses an
  outside folder even when the requester asks directly; closing the requester's session
  ends the helper; the helper ends an idle session and an expired one by itself, and
  the requester learns it at its next request; the requester ends an idle session
  without asking, which ends the helper; a requester that disconnects without a close
  request ends it; a cancelled request whose reply arrives within the two-second grace
  keeps the channel in step, one without a reply gives up after the grace; a request
  waits for a slot that a just-closed helper frees (Linux).
- **Engine** (`crates/midnight-engine/tests/elevation_sessions.rs`): after one retry,
  subfolders at two depths, the root (the UI's Refresh) and a `Refresh` are listed
  through the session with `ListingAuthorized` before their rows and no further consent,
  each first asked with ordinary rights; a Resort without rows stays inside; going above
  the root ends the session and the next denial offers a retry; a session that ended
  yields the original denial with a retry offer, and that retry opens a new session; a
  link outside is reported as `outside_authorization` with its own offer while the
  session continues; rows listed through the session are neither subscribed nor checked
  by the watch; shutdown ends the session. The earlier tests in `tests/elevation.rs`
  (an implementation without sessions behaves as before) still pass.
- **Presenter** (`crates/midnight-ui/src/presenter/tests/elevation.rs`): entering a
  subfolder and a deeper one shows rows as an authorized listing without a dialog;
  Ctrl+R re-lists through the session without a dialog; Backspace up to the root stays
  authorized, above it the rows are ordinary and a later denial offers a retry; a link
  outside the root shows "Access to this folder was denied. This folder is outside the
  authorized folder …" with its own retry.

## Runs

Commit: the one that adds this record, on top of `ff51c211`. Windows 11 (x64), Rust
1.99.1, from a clean worktree:

| Command | Result |
| --- | --- |
| `cargo test -p midnight-elevation --locked --offline` | 41 passed |
| `cargo test -p midnight-engine --lib --test integration --locked --offline -- elevation authorized_sessions watch:: navigation_fallback` | lib 109 passed; integration 14 passed |
| `cargo test -p midnight-ui --lib --locked --offline -- presenter::` | 1,302 passed, 4 ignored |
| `cargo clippy -p midnight-core -p midnight-elevation -p midnight-engine -p midnight-ui --all-targets --locked --offline -- -D warnings` | clean |
| `cargo fmt -p midnight-core -p midnight-elevation -p midnight-engine -p midnight-ui -- --check`, `scripts/check-source-size.ps1` | clean |

Ubuntu 24.04 under WSL 2 (ext4), Rust 1.99.1, in a clone of the same tree:

| Command | Result |
| --- | --- |
| `cargo test -p midnight-elevation --locked --offline` | 55 passed, 1 ignored (the subprocess half of the disconnect test, which its parent test runs) |
| `cargo test -p midnight-engine --lib --test integration --locked --offline -- elevation authorized_sessions watch::` | lib 105 passed; integration 12 passed |
| `cargo clippy -p midnight-core -p midnight-elevation -p midnight-engine --all-targets --locked --offline -- -D warnings` | clean |
| `cargo fmt -p midnight-core -p midnight-elevation -p midnight-engine -- --check` | clean |

## Not verified

An accepted UAC or polkit prompt with a session: entering several subfolders of a
folder that the account may not read without further prompts, Ctrl+R, and going back up
above the folder (the session ends and the next denial asks again). This stays owed with
the other manual authorization checks
([implementation backlog](../implementation-backlog.md), PLAT-02). Full workspace suites
were not run for this change.
