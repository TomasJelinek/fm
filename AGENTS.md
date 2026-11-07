# Project rules

- Third-party libraries must be usable for the intended development, application
  use and distribution **without mandatory fees, subscriptions or royalties**.
  Prefer maintained open-source libraries and compatible permissive licenses.
  Trial-only and paid-only dependencies are not acceptable.
- This applies to direct/transitive dependencies of the app, public API/SDK,
  independent plugins, build tools and tests. Free of charge does not waive
  attribution, source-distribution or other license obligations.
- On dependency changes, check the actual pinned license and enabled features,
  update the [inventory](docs/THIRD_PARTY.md) and distribution notices, and follow
  the [dependency policy](docs/DEPENDENCY_POLICY.md). Do not silently change the
  project's own license or select a paid alternative of a multi-licensed library.
- Follow [CODE_STYLE.md](CODE_STYLE.md) and [SECURITY.md](SECURITY.md). Keep platform
  implementations separate and all non-UI work in backend services. Build, test
  and generated output belongs under `_build/`; toolchain caches use `.tools/`.
