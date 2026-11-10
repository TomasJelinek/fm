# Independent native macOS filesystem

This separate Cargo workspace implements the existing public filesystem ABI/SDK
as `libmidnight_filesystem_native_macos.dylib`. It has no dependency on application
or private platform crates. The only third-party dependency is pinned libc
0.2.191 with `std`, under its inspected MIT alternative; retain THIRD-PARTY.md in
every package. The project's own license remains undecided.

The source provides paged directory browsing, no-follow metadata, captured regular
file reads, mounted roots, capacity and optional volume details. Native Unix bytes
remain addresses and names; display text may be lossy for invalid Unicode. Darwin
filesystem normalization is not repeated in the provider. Component-relative
lookup preserves dot/dot-dot and trailing slash semantics without an application
PATH_MAX limit. Explicit directory navigation follows native directory links;
metadata listing does not follow final links or open file contents.

Reads retain the original descriptor and compare device/inode/generation, length,
modification/change/birth times before and after each SDK read. A changed or
replaced source fails. Ordinary links, FIFOs, devices and unsupported pseudo-files
are refused. Capture checks regular-file identity before and after a no-follow,
nonblocking open. A concurrent namespace substitution can still reach that native
open, including a device's open handler; the result is never read unless it is
the same regular inode. This is a live validation contract, not an atomic snapshot
or a guarantee against privileged tampering. No Linux `/proc` reopen assumption
is used. Data is never read from an unvalidated special entry.

Volume enumeration uses bounded `getfsstat(MNT_NOWAIT)` snapshots and retries a
saturated array instead of publishing it as a complete list. Persistent churn or
exhausted budgets fails explicitly. Kernel fields that could be truncated are
unavailable rather than shortened navigation identities. Rich information keeps
physical total/free and caller-available capacity separate. The native volume
label is explicitly unavailable; a mount basename is not presented as its label.
Known network filesystem types are Network. All other storage is Unknown until
positive native physical SSD evidence is implemented; APFS alone is not SSD proof.

Build and native qualification, from the repository root on a Mac:

```sh
export CARGO_HOME="$PWD/.tools/cargo"
export RUSTUP_HOME="$PWD/.tools/rustup"
export CARGO_TARGET_DIR="$PWD/_build/goal/native-macos"
export MIDNIGHT_NATIVE_FIXTURE_ROOT="$PWD/_build/goal/native-macos/fixtures"
export TMPDIR="$PWD/_build/goal/native-macos/tmp"
mkdir -p "$TMPDIR"
cargo fmt --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml -- --check
cargo build --locked --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml
cargo test --locked --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml
cargo clippy --locked --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml --all-targets -- -D warnings
cargo build --locked --release --manifest-path plugins/midnight-filesystem-native-macos/Cargo.toml
```

Use an APFS fixture directory owned by the test user. Fixtures stay beneath the
explicit `_build` location and never touch system Trash. The dynamic test loads
the dylib alongside the Cargo test executables and checks callback provenance.
The native suite covers raw kernel names, selected binary bytes, independent
read/EOF, replacement and edits, FIFO/device/link refusal, directory-link
navigation, cancellation, paths longer than 2200 bytes and volume queries.
Non-UTF-8 creation is exercised where the fixture filesystem permits it; the
portable raw-byte policy test does not depend on filesystem acceptance.

`plugin.toml` describes an Apple Silicon build. For Intel, package
`plugin.x86_64.toml` as `plugin.toml` beside the matching dylib. Do not combine
incompatible architectures or label an unqualified deployment as supported.
Host Darwin loading/trust, worker containment, application adapters, build/release
integration and supported macOS versions remain separate work.

Primary API references consulted for this implementation:

- [Apple getfsstat manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/getfsstat.2.html)
- [Apple statfs/fstatfs manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/statfs.2.html)
- [Apple open manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/open.2.html)
- [Apple XNU open flags](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/fcntl.h)

The public declarations come from pinned libc, not copied Apple implementation
code. Apple system libraries are platform prerequisites; the MIT Rust binding
license does not relicense macOS. Verification results and outstanding native
qualification are recorded in VERIFICATION.md.
