# Dependency policy

Dependencies must be usable for development, application use and distribution
without mandatory fees, subscriptions or royalties. This covers direct and
transitive libraries, public APIs/SDKs, independent plugins, build tools and tests.
Prefer maintained open-source projects with compatible permissive licenses.
Trial-only or paid-only components are not acceptable.

Free of charge does not remove attribution, notice, source-distribution or other
license conditions. Do not change this project's own license or select a paid
alternative of a multi-licensed dependency without an explicit project decision.

For a dependency or feature change:

1. Review the exact locked version, its packaged license/notice files, upstream
   source revision and enabled features. Inspect bundled native libraries and
   build downloads too; a Rust wrapper's license is not the whole dependency.
2. Check its maintenance and RustSec status. Record unresolved advisories and
   the affected target/features; an informational warning is not automatically
   a vulnerability, and a clean scan is not proof of safety.
3. Record the free license alternative being used where alternatives differ.
   Preserve every obligation of conjunctive (`AND`) expressions.
4. Update [the inventory](THIRD_PARTY.md), its machine-readable feature table,
   and the distribution notice requirements. Keep Cargo.lock committed and use
   locked builds. Do not infer distribution contents from all-target metadata.
5. Test affected platforms and package the actual required license and notice
   texts with distributed binaries. Preserve copyright holders' notices; links
   to crates.io alone do not replace notices required in a distribution.

The current desktop uses Slint's free desktop license alternative, described in
[the inventory](THIRD_PARTY.md#slint). Its platform, attribution and API
restrictions must also be checked when designing future plugins or products.
No new third-party package is introduced by the main-branch audit.

Build and audit output belongs under `_build/`; downloaded toolchains,
registries and advisory databases belong under `.tools/`. Never add generated
binaries or caches to Git. A release must include a reviewed notice bundle;
the inventory is an audit input, not that bundle.
