# Midnight Explorer release builds

The workspace release profile enables Rust's highest standard optimization level
(`opt-level = 3`), whole-program **fat LTO**, a single code-generation unit and no
incremental compilation. Release artifacts omit debug information and strip
symbols. Development-only assertions are disabled. These settings apply to the
whole dependency graph; build-time host tools retain Cargo's normal build overrides.

Integer overflow checks remain enabled, and `panic = "unwind"` preserves the
backend's worker-panic reporting. File integrity, bounds checks, native identity
validation and explicit confirmation are required behavior in every profile.
There are no unsafe floating-point rewrites or unchecked data-access shortcuts.

The default executable targets the supported platform's portable CPU baseline.
Do not set `target-cpu=native` globally: that would make a redistributed binary
depend on the builder's processor. Explicit machine-specific builds can use that
flag when deployment hardware is known, but must be labeled and tested separately.

PGO is a future measured optimization, not an enabled flag without training data.
It requires an instrumented build, representative browsing/scrolling/search/
operation workloads, matching LLVM profile merging and a rebuild with the profile.
Store generated profiles under `_build/pgo`; never commit them. Compare frame-time
tails, operation throughput and memory, rather than assuming that every additional
optimization always improves performance.

Run `build.bat release` or `./build.sh release` (Cargo uses `--release`), then smoke-test that actual
executable. Native tests that exercise integrity and queue/cancellation semantics
must also run under the release profile. Builds take longer and may use more RAM
because of whole-program optimization. Development builds remain available for
fast iteration and debugging.

Primary references: [Cargo profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
and [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html).
