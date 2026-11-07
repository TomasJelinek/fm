# Startup failure during thumbnail status publication

The release executable at `_build/release/me.exe` exited with code 101 before a
usable window appeared. Its captured stderr identified `Rows::row_count` in
`midnight-ui/src/lib.rs`: `RefCell already mutably borrowed`.

The thumbnail timer retained a mutable presenter borrow while calling the UI
refresh function. Slint can synchronously evaluate its presenter-backed models
during refresh, which requires a shared borrow of that same presenter. A plugin
discovery failure triggered the refresh even with detailed panels. Preview target,
frame and animation changes used the same unsafe borrow lifetime.

The fix releases the mutable presenter borrow before refreshing with a shared
borrow. Plugin trust validation remains enforced. An unavailable preview provider
must produce an ordinary error notice and leave the file manager running.

Regression coverage uses the actual thumbnail service initialization, 100 ms timer,
presenter-backed panel models and Slint rendering. An invalid executable identity
produces a deterministic asynchronous discovery error without touching user data.
The test expects a visible main window and the normal Previews notice.

The earlier release smoke run used a trusted private installation and default
in-memory preferences. That successful run did not exercise the failing discovery
path from the build directory. Qualification now also includes ordinary release
launch from the reported build directory with the user's existing preferences.

Validation completed:

- Before the fix, the new regression failed at the exact reported location:
  `Rows::row_count`, with `RefCell already mutably borrowed`.
- After the fix, all 12 composition tests passed, including the real thumbnail
  timer regression. Strict Clippy for the application and all its targets passed.
- Workspace rustfmt, whitespace checks and the 683-file source-size guard passed.
- The optimized Windows release rebuilt successfully. A normal launch from
  `D:\experiments\dev\FM\_build\release\me.exe` retained a responsive process and
  a visible window titled Midnight Explorer 0.1 — File Manager, using the native
  saved settings. Its redirected stderr remained empty.
- Focused independent review confirmed the shared redraw boundary fix and test;
  no further actionable findings were reported.
- Release SHA-256: 32b3c4bcd044f395ad34b3b3fcd6a8f775ac2d025434bad01e42618cec7eeb03.
