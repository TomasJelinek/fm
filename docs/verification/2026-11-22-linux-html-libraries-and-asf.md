# Linux follow-ups: HTML viewer libraries, setup packages, ASF/WMV and video dependency records

22 November 2026, on base commit `e5b8b887` (origin/main). Windows checks on Windows 11
Enterprise 26200 with the pinned Rust 1.99.1; Linux checks on WSL 2 Ubuntu 24.04.4
(kernel 6.6.87.2, glibc 2.39, distribution FFmpeg 6.1.1, GStreamer 1.24.2 with
plugins-base, -good, -bad and gstreamer1.0-libav, without gstreamer1.0-plugins-ugly).
Every new test was run red (behaviour removed) and green.

## HTML viewer: missing system libraries

Symptom: on a system without a library that the HTML viewer's Chromium (CEF) needs, the
dynamic loader stops the viewer's launcher (exit status 127) before its first message,
and the viewer showed "Chromium transport stopped: failed to fill whole buffer".

Now, when a browser fails before it reports Ready, the host session asks the platform
why once the watchdog has waited for the process tree (`BrowserProcess::startup_failure`,
`browser::Process::startup_failure`). On Linux, after the loader's status 127, the
worker platform reads the ELF dynamic sections of the launcher and of every library it
needs, looked up as glibc's loader looks them up (the DT_RPATH chain unless the needing
object has DT_RUNPATH, then DT_RUNPATH, `/etc/ld.so.cache`, the default folders; `$ORIGIN`
expanded, other substitutions left out, candidates of another machine skipped), and
returns `browser::LibrariesMissing` with the missing sonames and a message such as:

> The HTML viewer needs system libraries that are not installed: libnss3.so,
> libatk-1.0.so.0. Install them with: sudo apt install libnss3 libatk1.0-0t64

The command follows the running distribution family (os-release): Debian/Ubuntu package
names from a table of the CEF libraries, soname capabilities for dnf and zypper
(`'libnss3.so()(64bit)'`), none for other distributions. A development build in a source
checkout also names `scripts/setup-linux.sh --install`. Nothing is executed (no `ldd`, no
`ld.so --list`); only regular files are opened, without blocking on a FIFO, with bounded
reads and at most 1,024 objects; names are masked to letters, digits and `._+-` and only
plain names enter a command. A browser that reported Ready keeps its own error, and the
check runs only after such a failure. Windows reports no such reason. Chromium's sandbox
is unchanged: there is no `--no-sandbox` path.

Not verified: the real CEF package on a system that lacks the libraries (the WSL machine
has them all, and no Linux CEF package was available offline). The Debian package table
follows Chromium's Linux runtime dependencies; it was not compared with the DT_NEEDED list
of this CEF build's `libcef.so`. A library missing from the table is still named, with
"and the packages that provide …" in the apt case.

Tests (worker platform, Linux): synthetic ELF64 objects written at test time cover the
loader's lookup (`$ORIGIN` DT_RPATH inherited by CEF's own needs, transitive misses
reported once in the loader's order, sonames of loaded objects, DT_RUNPATH ending the
DT_RPATH chain, a synthetic `ld.so.cache` in the current and the compatibility layout,
damaged caches, candidates for another machine, FIFOs, directories, truncated and
malformed objects), the message forms, the os-release families and the checkout
detection. A synthetic program with a real `PT_INTERP` that needs a missing library is run
with an empty environment: the system's loader ends it with status 127 and the diagnosis
names exactly that library. An ignored test starts it through the real browser launcher
(`browser::spawn`, with its cgroup) inside `systemd-run --user --scope -p Delegate=yes`
and checks `Process::startup_failure` after the watchdog's kill and wait. Host session
tests (Windows and Linux): a scripted browser whose output ends before Ready gets the
platform's reason as its terminal error (no sandbox refusal), the stopped transport
stays the error without a reason and for a browser that was Ready, and `startup_error`
shows `LibrariesMissing` without the "Chromium startup:" prefix. Red runs: with the
transport message unchanged, the two host tests failed with "Chromium transport stopped:
failed to fill whole buffer"; with the dependency walk stubbed out, 7 of the Linux tests
and the end-to-end test failed.

## Setup script

`scripts/setup-linux.sh --install` now installs, after the build packages, the runtime
libraries of the application (fontconfig, FreeType, xkbcommon/-x11, X11, Xcursor, Xi,
Xrandr, XCB, Wayland client, udev, Vulkan loader, `xdg-utils`) and every library of the
HTML viewer's Chromium (NSS/NSPR, ALSA, ATK/AT-SPI, CUPS, D-Bus, DRM/GBM, Expat, GLib,
Pango/Cairo, X11/XCB and its extensions): Ubuntu 24.04 package names for apt (the `t64`
names), soname capabilities for dnf and zypper. A Rust test runs the script's dry-run plan
for Ubuntu, Fedora and openSUSE and checks that it installs every library the viewer's
message can name; two Python tests check the second install command of each plan and
that a failed build install stops before it. Red against the previous script: the Rust
test and both Python tests failed; green: all 24 setup tests.

## ASF/WMV on Linux (MEDIA-02)

Runtime profile r4 routes `.wmv` and `.asf` to the platform helpers. The Linux helper now
has `asf` in the distribution FFmpeg's format whitelist and gst-plugins-ugly's `asf` plugin
(ASF demuxer) in its fixed GStreamer list, loaded before the sandbox like the others;
Windows Media decoders were already mapped (WMV 1/2/3 and VC-1 to codec 7, WMA to 20).
The plugin's `asf` registration of an RTSP extension and an RTP depayloader adds nothing
that a graph selects; the registry test now also checks that no `rtspsrc` or `udpsrc` is
available. Fixtures are encoded at test time with the distribution's `ffmpeg`: Windows Media
Video 8 + Audio 2 in `.wmv` and version 7 + 1 in `.asf`. FFmpeg's ASF muxer starts the video
one audio frame (46 ms) after the audio, which the session check now allows for.

- Distribution FFmpeg: six thumbnail samples (red, green, green, blue, blue, white) and a
  playback session (30 frames 40 ms apart from 46 ms, 48,000 PCM frames, seek to 2.5 s
  landing on 2,526 ms) for both fixtures; the real thumbnail and playback helpers decode
  the `.wmv` fixture inside the worker sandbox (thumbnail helper ready after 44.7 ms).
- GStreamer: with FFmpeg hidden and gst-plugins-ugly absent, the helper reports exactly
  `NoDecoder { codec: Unknown, tried: GSTREAMER }`; decoding through the ASF demuxer and
  gst-libav was not run (install `gstreamer1.0-plugins-ugly` to run it).
- A test checks that every extension in `thumbnailer.json` has its container in the FFmpeg
  whitelist and a demuxer plugin in the GStreamer list.
- Red, without `asf` in both lists: the coverage test, the FFmpeg decoding test ("The
  source is not a recognized video container") and the helper test failed.

WMV 9 and VC-1 decoding was not exercised (FFmpeg has no encoder for them).

## libc pin and dependency records

The video plugin's Linux libc requirement is now `=0.2.191` (default features off, `std`),
as in the native filesystem plugins; its lockfile already resolved 0.2.191 and does not
change. `scripts/tests/test_plugin_dependency_pins.py` checks every independent plugin's
registry dependencies for exact pins matching its lockfile (red on the base manifest:
`libc = "0.2"`).

The `system` feature resolves, on Linux, only libc 0.2.191 (MIT OR Apache-2.0); on Windows
windows 0.62.2, windows-core 0.62.2, windows-collections 0.3.2, windows-future 0.3.2,
windows-numerics 0.3.1, windows-threading 0.2.1, windows-implement 0.60.2,
windows-interface 0.59.3, windows-result 0.4.1, windows-strings 0.5.1, windows-link 0.2.1,
windows-sys 0.61.2, syn 2.0.119, proc-macro2 1.0.107, quote 1.0.47 (all MIT OR Apache-2.0)
and unicode-ident 1.0.27 ((MIT OR Apache-2.0) AND Unicode-3.0), each with its license files
in the pinned crate. `scripts/thumbnail-notices.py` (run on WSL with the released worker SDK)
puts all of them into the video package's notices (46 records). The documented inventory
capture used default features, so it could not see them; the `thumbnail-video` captures
now use `--features system,playback-worker`, and that scope was recaptured (Windows on
Windows, Linux on WSL) and applied with the generator's own `inventory()` and `render()`
after the committed CSV reproduced the three committed tables byte for byte: windows and
its nine crates and syn 2.0.119 gain the scope, and `midnight-thumbnail-video` declares
windows, windows-core and libc. Totals unchanged (665 records, 546 names, 70 direct). The
other 44 capture pairs were not retaken.

## Commands and results

Linux (WSL clone of the change, `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`,
offline, `TMPDIR` inside the clone; `$sdk` is the released worker SDK's `registry.toml`):

| Command | Result |
| --- | --- |
| `cargo test -p midnight-worker-platform --lib --locked --offline` | 62 passed, 3 ignored (two earlier delegated-cgroup tests and the new end-to-end test) |
| the new end-to-end test, `--ignored --exact`, inside `systemd-run --user --scope -p Delegate=yes` | 1 passed; red (dependency walk stubbed): 1 failed |
| `cargo test -p midnight-web-viewer-host --lib --locked --offline` | 83 passed |
| `cargo clippy -p midnight-worker-platform -p midnight-web-viewer-host --all-targets --locked --offline -- -D warnings` | clean |
| `RUST_TEST_THREADS=1 cargo test --config $sdk --manifest-path plugins/midnight-thumbnail-video/Cargo.toml --no-default-features --features system,playback-worker --all-targets --no-fail-fast --locked --offline` | library 58 passed, `system_linux` 8 passed |
| the same with `cargo clippy ... -- -D warnings` | clean |
| `cargo fmt -p midnight-worker-platform -p midnight-web-viewer-host -- --check`, `cargo fmt --manifest-path plugins/midnight-thumbnail-video/Cargo.toml --all -- --check` | clean |
| `python3 -B -m unittest discover -s scripts/tests -p 'test_build_setup_linux*.py'` | 24 OK; red against the previous script: 1 failure and 5 subtest errors (the 2 new tests) |
| the Rust setup-plan test (`the_setup_script_installs_every_library_that_the_message_names`) | passed; red against the previous script: failed |
| `python3 -B -m unittest discover -s scripts/tests -p 'test_plugin_dependency_pins.py'` | 1 OK |
| red run of the library diagnosis (`missing_with` stubbed) | the 6 lookup tests failed (walk, cache, runpath, machine, special files, real loader); the message, os-release, `$ORIGIN`, checkout, soname and complete-program tests do not depend on the walk's findings |
| red run of the video plugin (`asf` removed from both lists) | 2 library tests and the helper test failed |
| `bash scripts/check-source-size.sh` | 3,223 files, all at most 100,000 bytes |
| `python3 -B scripts/thumbnail-notices.py --config $sdk --output <clone>/_build/thumbnail-notices-check` | 46 video records, every platform-helper package with its license files |

Windows (offline, through the shared build queue):

| Command | Result |
| --- | --- |
| `cargo test -p midnight-worker-platform -p midnight-web-viewer-host --lib --locked --offline` | web-viewer-host 83 passed; worker-platform 31 passed, 1 ignored (earlier test) |
| the same host tests with the transport message left unchanged (red) | 2 failed: "Chromium transport stopped: failed to fill whole buffer" and the "Chromium startup:" prefix |
| `cargo clippy -p midnight-worker-platform -p midnight-web-viewer-host --all-targets --target x86_64-pc-windows-msvc --target x86_64-unknown-linux-gnu --locked --offline -- -D warnings` | clean |
| `cargo test --config $sdk --manifest-path plugins/midnight-thumbnail-video/Cargo.toml --no-default-features --features system,playback-worker --lib --locked --offline` | 38 passed |
| `cargo clippy --config $sdk --manifest-path plugins/midnight-thumbnail-video/Cargo.toml --no-default-features --features system,playback-worker --all-targets --target x86_64-pc-windows-msvc --target x86_64-unknown-linux-gnu --locked --offline -- -D warnings` | clean (the pinned lockfile resolves libc 0.2.191 unchanged) |
| `python -B -m unittest discover -s scripts/tests -p test_plugin_dependency_pins.py` | 1 OK; red on the base manifest: 1 failure (`libc = "0.2"`) |
| `python -B -m unittest discover -s scripts/tests -p <file>` for `test_dependency_inventory.py`, `test_thumbnail_notices.py`, `test_package_release.py`, `test_runtime_packages.py` | 12 OK; 22 OK (5 skipped); 26 OK (7 skipped); 10 OK |
