# Find persistence and result-panel actions

Verified on Linux on 2026-11-24 using the existing offline Docker build image and
the pinned Rust 1.99.1 toolchain. No dependencies or enabled dependency features
changed. Implemented behavior is documented in [Find](../developer/FIND.md).

## Targeted results

| Target and selection | Passed | Ignored |
| --- | ---: | ---: |
| Core library | 180 | 0 |
| Common platform settings codec | 94 | 0 |
| Engine Find, feed projection, feed construction and folder sizes | 16 | 0 |
| Engine Find integration | 89 | 4 |
| Core Find integration | 5 | 0 |
| UI Find and captured file-action presenters | 179 | 2 |
| UI Find windows, fed-panel keys, panel choices and Make File List | 75 | 0 |
| Total | 638 | 6 |

The traversal regression streams 1,000,001 entries with a late match. Other
regressions cover preference compatibility and strict validation, accepted-mask
history, late settings without replacing typed queries, geometry policies,
captured paths and selection, queue refusal, cancellation and stale worker
receipts, folder-size actor wakeup, worker retirement, resizable columns and
Ctrl+M/Ctrl+F8 routing. Software-rendered desktop/input tests ran serially;
headless processes used at most four test threads in total.

Compile command:

```sh
cargo test -p midnight-core -p midnight-platform-common -p midnight-engine \
  -p midnight-ui --lib --test integration --locked --offline -j 2 \
  --no-run --message-format=json
```

The compiled artifacts were selected with `scripts/run-targeted-tests.py`.
Headless selections were the core library, common `settings::`, engine
`feed_projection::`, `find_feed_sizes::`, `filters::` and `find::`, engine
integration `find_`, and core integration `find::`. UI selections were library
`find`, `presenter::file_list::`, `presenter::checksums`,
`presenter::file_compare`, `presenter::tests::user_commands`, and integration
`find_`, `feed_panel_keys::`, `panel_choices::`, `file_list_window::`.
The final build reused the unchanged, already-passing headless artifacts; the
final UI artifacts were run after the last implementation change.

## Additional checks and scope

```sh
cargo clippy -p midnight-core -p midnight-platform-common -p midnight-engine \
  -p midnight-ui -p midnight-explorer --all-targets --locked --offline -j 2 \
  -- -D warnings
cargo fmt --all -- --check
bash scripts/check-source-size.sh
git diff --check
```

All completed successfully. Clippy includes the application entry point and test
targets. The viewer build script emitted existing Slint layout warnings in
unchanged viewer sources. All 3,454 checked source files met the 100,000-byte limit.

This is targeted validation, not a completed full-workspace run. An earlier
workspace attempt was stopped for disk headroom. Native Windows execution,
interactive monitor/DPI placement and the six ignored tests were not run.
