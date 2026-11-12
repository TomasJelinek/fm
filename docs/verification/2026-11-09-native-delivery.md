# Native filesystem build, package and CI integration checks

Date: 2026-11-09. Candidate base: `9df66c54c1b1b6e7a77a6ce81ca5bf07093eec0e`.
This change connects the committed native backend to the Windows/Linux build
helpers, release packaging and CI. Application composition and viewer cutover
remain separate work. No application or UI source, dependencies, lockfiles or
third-party notices change here.

## Behavior checked

- Common build helpers build the host, native provider and viewers in that order;
  failures stop subsequent steps, including Windows private deployment.
- Native helpers support locked tests, offline builds and isolated build/output
  paths. They stage all package inputs before publishing. A missing input leaves
  the previous package unchanged; Windows refuses a locked DLL before replacing
  its metadata, and Linux installs a new inode while existing readers retain the
  old bytes. Unrelated package files are retained. Package installation is not a
  transaction spanning all files or an update of an entire running application.
- Application release packaging requires exactly one compatible native provider.
  Tests reject duplicates, missing libraries/notices, incompatible target/ABI,
  unsupported capability/encoding and escaping entries before copying filesystem
  packages. Native libraries and optional archive executables retain distinct
  entries and permission roles in the package inventory.
- CI adds independent Windows/Linux native builds with only public filesystem
  API/SDK source available, then supplies their artifacts to protected host tests.
  The existing release smoke test remains unchanged.

## Windows evidence

All local evidence is under `_build/qa/native-delivery/`:

- `delivery-tests-windows-final.log`: focused build-hook, native helper/package,
  archive/thumbnail package and release tests: 29 passed, 7 skipped, 0 failed.
  Skips cover four Linux-only helper tests, two symlink-creation privilege limits,
  and a Unix-only literal-backslash filename test.
- `windows-helper-red.log`: original helper failed the new argument-forwarding
  and missing-input preservation checks. The fixed helper passes those cases,
  locked-DLL preservation, failed-test/build admission and paths containing spaces.
- `windows-helper-actual.log`: actual `-Profile debug -Test -Offline` helper run;
  all 14 independent Windows plugin tests passed, including a dynamically loaded
  DLL, then the build installed the four expected package files.
- `windows-public-api-tests.log`, `windows-public-plugin-tests.log`: a separate
  fixture containing only `filesystem-api/`, the Windows native plugin and the
  pinned toolchain passed 38 API/SDK and 14 plugin tests. Both formatting and
  strict Clippy checks passed; `windows-public-plugin-release.log` records the
  optimized build. The public-only fixture contains no private `crates/` tree.
- `windows-package-validation.json`: the actual helper output passed the release
  packager's required-native validation. All installed files match their source
  or build output. Changed source files satisfy UTF-8/LF and the 100 KB limit.
- `windows-installed-tests.log`: both unchanged native host integration tests
  passed against that helper-built DLL in a fresh protected installation under
  `C:/Users/tjelinek/_build/`. The existing qualified backend test binary was
  reused; its hash and installed-library match are in
  `windows-installed-metadata.json`.

The helper-built Windows DLL SHA-256 is
`1928f13c7b2f220ced6e655ae5938c335ad8f0ecca7734c9085675a46c2ad4e2`.

## Linux evidence

Evidence is under `_build/qa/native-delivery/linux/`; `results.json` records
102 passing test executions and no failures:

- `python-tests.log`: 30 focused tests passed, with six expected Windows-only
  skips. The Linux inode-preservation, failed-test admission, staging-failure,
  offline/release argument and symlink/path checks all ran.
- `native-helper-actual.log`: actual offline debug helper run passed all 16
  independent plugin tests and published exactly four matching package files.
  `package-validation.json` records successful required-native release validation
  and matching helper/installed library hashes.
- `installed-host-tests.log`: both native host integration cases passed against
  that freshly built shared library in a protected fixture. The reused host test
  binary's backend sources matched the candidate; its hash is recorded alongside
  the installed library hash.
- `public-api-tests.log` and `public-plugin-tests.log`: the public-only checkout
  passed 38 API/SDK and 16 plugin tests. Formatting, strict Clippy and the release
  build passed, as did `bash -n` for both shell build helpers.
- `final-audit.log`: all 1,817 candidate source files and 54 public-only copied
  files remained unchanged. No worker processes or helper staging files remained.

The helper-built Linux SO SHA-256 is
`8774ba78c53ab548686248c62e26a242972f85ca5b72c30a14f541f884df2ccf`;
the public-only release SO is
`55fbad37b70b846421231ab919c0095feadc9b4dd3f7ac23605b8f497149faa3`.
Linux's independent JSON source snapshot SHA-256 is
`a74b0801e3272f152702bfdbc69efd6a7f561c45e64acc69ba5705d5188d3ab8`.

## Source boundary and limits

Before this evidence document, the tested candidate tree was
`89e23c0728996379564d77ca00947569dd110523`, containing 1,817 indexed source files.
`source-before-verification.sha256` records their hashes; its SHA-256 is
`a9951df3b4b1fb550174938145ef072b00ba7ff67d8c816367880251e06a516b`.
Only this evidence document was added after source qualification.

These checks cover local build, installation and packaging boundaries. They do
not claim a hosted CI run of this candidate, a complete application release
archive, production cutover, macOS support or desktop/UI qualification. The root
working tree and index were left untouched; the candidate was prepared in an
isolated source snapshot with its own Git index.
