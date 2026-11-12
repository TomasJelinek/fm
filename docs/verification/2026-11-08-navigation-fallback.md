# Native navigation fallback

Native pane navigation now recovers from missing, non-directory, inaccessible and
offline locations by trying each strict parent. After the requested native root
fails, the backend can try the operating system root: `/` on Linux, or the drive
containing the Windows system directory. An already-failed final root is not
retried. Invalid/unsupported provider data, cancellation and resource exhaustion
remain terminal failures.

The application enables this policy with
the native navigation fallback service (the current composition API is
`engine.configure_navigation_fallback()?.install(local.clone())`). The native root service returns
only an address; every subsequent listing still uses the installed filesystem
provider. There is no built-in filesystem-provider bypass. Archive requests use
their separate typed protocol and are not reinterpreted as native paths.

Each failed listing produces a generation-tagged `ListingFallback` event. The
presenter accepts it only while that exact native request is pending and submits
the next address under a fresh generation. Thus earlier batches, completions,
storage information and fallback responses cannot replace later navigation or a
different tab. The fallback chain has a budget derived from the original ancestor
count plus one system-root attempt, even if a defective service repeats targets.
Queue rejection or total failure retains the last committed directory and reports
an error. Only a successful listing commits its actual address and history.

Native root queries and listings run on the existing bounded backend workers.
Cancellation remains cooperative around native calls, including blocked calls
that finish after supersession. `PathBuf` and raw OS strings remain authoritative;
the Windows root query uses the existing dynamically-sized UTF-16 system-directory
helper. No `MAX_PATH`, lossy identity conversion or new dependency was introduced.

Regression coverage added:

- Real native listing recovery through two missing Unicode descendants to the
  first accessible ancestor, using an exclusively owned temporary fixture.
- Cancelled uninterruptible failure, terminal denied root, provider-data failures,
  relative-address exclusion and preservation of long native parent paths.
- Windows unavailable-drive/UNC-root recovery and unpaired UTF-16 identity;
  Linux undecodable native bytes.
- Presenter generation rejection, partial-listing disposal, late storage/results,
  recovered tab/history/focus, bounded retries and queue-rejection reporting.
- Windows service result is an absolute root containing the queried OS directory.

Verification at implementation handoff: direct `rustfmt` passed for all changed
Rust files. Cargo was deliberately left to the coordinating agent to avoid
concurrent workspace builds; no pre-implementation red run was performed. The
focused commands for the coordinated run are:

```text
cargo test -p midnight-engine --test navigation_fallback
cargo test -p midnight-engine --lib navigation_fallback
cargo test -p midnight-ui --lib presenter::tests::navigation_fallback
cargo test -p midnight-platform-windows navigation_fallback
```

Use the workspace-local `.tools/cargo` and `.tools/rustup` toolchain environment,
with generated artifacts under `_build/`. Linux-specific tests require Linux;
Windows execution does not qualify Linux behavior.

Coordinated results: Windows passed 4 engine policy tests, both native integration
tests, all 6 presenter recovery tests and the OS-drive root service test. Linux
passed 3 engine policy tests (including undecodable path bytes), both native
integration tests and all 6 presenter tests. Windows metadata/backend regression
coverage also passed 25 tests. See `backend-consolidated.log`,
`linux-navigation.log` and the UI logs under `_build/qa/metadata-review/`.
