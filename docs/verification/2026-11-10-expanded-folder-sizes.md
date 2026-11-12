# Ctrl+Shift+S includes expanded folder rows

The user's follow-up supersedes the original top-level-only unselected size scope.
With no marks, the backend now captures every selectable ordinary directory in the
acknowledged projection, including expanded descendants outside the scroll viewport.
Nonempty selection still wins, and files-only marks never cause a fallback scan.
Hidden/collapsed rows, filter-context ancestors and directory links are excluded.
Ctrl+A retains its separate top-level-only selection policy.

`TreeSizeScope::SelectionOrVisible` makes this policy explicit. Exact publication,
selection identity, provider retention, resource budgets, progress throttling and
cancellation ownership are unchanged. Scans stay on backend workers; progress
neither reprojects nor resorts rows. This is an internal Rust contract change,
not a plugin ABI, dependency or preference-format change.

Two new backend regressions first failed with 2 instead of 3 expanded targets,
and 0 instead of 1 target beneath a filtered context ancestor. After the change,
the Windows engine library passed 644 tests with one intentional benchmark ignored.
The existing selection-allocation, files-only and focused-Space cases passed too.
Logs and scoped before-images are under `_build/qa/expanded-sizes/`.

Native/archive bridge tests now require independent parent, large child, small
child and empty-child totals while retaining the same tree projection and all
three view modes. The existing scanners and storage-aware scheduling are reused.
Overlapping roots can still perform repeated enumeration; removing those roots
without descendant result propagation would lose their values or smaller-folder
fairness. Safe per-request reuse is a separate optimization, not silently assumed
by this scope change.

Salamander's `CFilesWindow::CalculateDirSizes` in `src/fileswn6.cpp` was consulted:
it dispatches native and archive calculation separately. Inline expanded rows are
Midnight Explorer's added display scope; no source code was copied.

## Final verification

- Windows: engine 644, UI library 735, directory-size bridges 7, native tree
  Copy/Move bridges 6 and native tree watch bridges 2 passed.
- Linux: engine 635, UI library 730 and the same 7/6/2 bridges passed. Each engine
  suite intentionally ignores one CPU benchmark; no test failed.
- Windows core/UI all-target Clippy passed with `--no-deps -- -D warnings`.
  Existing engine unused-field warnings remain; this is not a claim of a clean
  strict workspace lint run.
- `cargo fmt --all -- --check` passed. The source-size check passed for 1,659
  handwritten files, each at most 100,000 bytes.
- The Windows debug application built successfully. Both
  `D:\experiments\dev\FM\_build\debug\me.exe` and the existing private installation
  `C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe` have SHA-256
  `24620126053B5909EB9191559BC05A6F96F262DD0433AFAD3188583DB20A2B91`.
  The private plugin runtime was registered for that exact hash.
- The actual build-path executable passed `--smoke-test`, exited with code 0
  and empty stderr, and listed three volumes plus 30/132 panel entries.

The bridge tests drive the real installed Slint callbacks and backend workers
with deterministic native/archive fixtures. They verify the requested scope and
retained projection, not physical GPU/gesture performance or every installed
archive decoder. No release rebuild or plugin binary replacement was needed.
