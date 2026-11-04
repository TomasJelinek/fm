# Code style

Use the public [Rust Style Guide](https://doc.rust-lang.org/style-guide/) and
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/). `rustfmt` is
the formatting authority; strict Clippy checks run in CI.

- **Line width:** `rustfmt max_width = 100`, four-space indentation, no tabs.
  Rustfmt may retain an indivisible literal/URL beyond that formatting target.
- **Encoding:** UTF-8 source and LF line endings on every development platform.
- **File size:** handwritten source files must be **at most 100 KB = 100,000
  bytes**, measured on disk as UTF-8. Generated code, lockfiles, third-party
  dependencies, media and build artifacts are excluded. A few hundred lines per
  responsibility is a guideline, not a second hard file-size limit.
- **Naming:** `snake_case` modules/functions/variables, `UpperCamelCase` types and
  traits, `SCREAMING_SNAKE_CASE` constants; follow standard Rust acronym spelling.
- **Modules/platforms:** one clear responsibility per module. Windows, Linux and
  future macOS implementations live in separate platform crates/files. A small
  facade may select/re-export an implementation with `cfg`; implementation bodies
  for multiple OSes do not share a file. Shared code must be platform-neutral.
  Platform-specific tests are separated too.
- **Public APIs:** expose the smallest useful surface, prefer private or
  `pub(crate)` internals, document public contracts/errors and use typed IDs,
  paths, enums and capabilities. Each platform implements the common service
  traits; engine/UI code depends on contracts rather than OS APIs.
- **Errors/safety:** return `Result` for expected failures. No `unwrap`/`expect`
  on user input or filesystem/network results in application code. Unsafe code is
  forbidden in core/engine/app and denied in handwritten UI; isolate audited FFI
  with a `SAFETY` explanation and generated bindings with a narrow allowance.
- **Async boundaries:** filesystem/network/CPU-heavy work and large-data cleanup
  belong to backend workers. UI callbacks perform short input/presentation work.
  Use bounded channels, cooperative cancellation and explicit ownership.
- **Text/paths:** preserve Unicode and raw provider identities, never use ANSI
  conversions, truncate operation paths or impose `MAX_PATH`. Display strings
  are not filesystem identities.
- **Tests:** test observable behavior and real boundaries; cover errors, races,
  cancellation, Unicode and long paths. Keep large and OS-specific test groups
  in separate files. Do not copy implementation logic into expected results.

Checks: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
Source size: `pwsh scripts/check-source-size.ps1` on Windows or
`bash scripts/check-source-size.sh` on Linux; no extra interpreter is required.
