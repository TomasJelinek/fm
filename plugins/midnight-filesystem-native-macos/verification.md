# Native macOS package: source qualification

Date: 2026-11-08. Host: Windows workspace. Compiler: pinned Rust 1.99.1.
Scope: only `plugins/midnight-filesystem-native-macos/**`. Shared API/SDK, existing
plugins, application/platform code, root manifests/locks and build scripts were
not edited by this package task.

Both Darwin standard-library targets were installed through rustup into the
workspace `.tools/rustup` cache. Cargo used `.tools/cargo`, with every target output
under `_build/goal/native-macos`. Package dependency resolution generated its own
Cargo.lock offline, containing libc 0.2.191 and the project-owned API/SDK only.

Executed checks:

| Check | Result |
| --- | --- |
| `cargo test --locked --offline --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml` on Windows | Pass: 5 portable policy tests; native-only integration targets contain 0 runnable tests on Windows |
| Same portable tests against initial policy stubs | Expected failure: all 5 tests failed for absent behavior before implementation |
| `cargo fmt --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml -- --check` | Pass |
| `cargo check --locked --offline --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml --target x86_64-apple-darwin --all-targets` | Pass, native source and all tests type-checked |
| Same check with `--target aarch64-apple-darwin` | Pass |
| `cargo clippy --locked --offline --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml --all-targets -- -D warnings` | Pass on Windows host |
| Same strict Clippy command with `--target x86_64-apple-darwin` | Pass |
| Same strict Clippy command with `--target aarch64-apple-darwin` | Pass |

The eight Darwin-only test functions cover the public ABI, a real dynamically
loaded dylib, raw kernel names, exact selected content and EOF, changes/replacement,
FIFO/device/link refusal, explicit directory-link browsing, cancellation,
component-relative paths beyond 2200 bytes, volume details and deterministic
substitution between stat and content open. These test sources type-check for
both architectures. They have **not run on macOS**.

No Apple linking, macOS process execution, dylib loading, signing/packaging,
deployment-target compatibility or host application integration was established.
The native test suite must run on actual Mac hardware/runners, with the exact
OS/architecture/filesystem recorded before native success is claimed. Qualify
both APFS name/normalization behavior and a filesystem accepting non-UTF-8 names,
native ACL/search-only ancestor behavior, symlink/mount changes, cancellation
during blocking filesystem calls and network-mounted volume states.

Known declared limits: local media are Unknown without positive SSD proof;
native volume labels are unavailable; `statfs` native text at its fixed-field
boundary is refused as potentially truncated. Ordinary descriptor traversal
currently opens directory ancestors read-only and can therefore be more
restrictive than execute-only POSIX traversal. A name substituted during Darwin
capture may reach native open; type/identity validation precedes every content
read. Retained files and blocking calls own their resources until actual return.

Native package notices record the inspected libc MIT branch and full copyright
notice. Root aggregate inventory and application packaging are parent-owned
follow-ups; the package is not represented as enabled macOS application support.
