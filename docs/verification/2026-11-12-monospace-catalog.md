# Basic-text coverage in the monospace font catalog

## Failure and cause

GitHub run [43122031929](https://github.com/TomasJelinek/fm/actions/runs/43122031929),
head `35227750df8bce15c6419310c9307ab9386604a3`, failed Ubuntu job `130866843187`
in `installed_monospace_catalog_is_cached_and_lays_out_equal_width_columns`.
Noto Color Emoji produced widths **35.5625 versus 126.5625** for eight i and W
characters. The original log is retained at
`_build/qa/viewer-settings-20261111/ci-3522775-ubuntu-job130866843187.log`.

The catalog admitted any first family face whose Swash metrics reported fixed
pitch. Noto Color Emoji reports Fontconfig spacing 100, but its cmap lacks Latin
letters and most ASCII punctuation. It does contain ASCII digits. Selecting that
face for basic text invokes renderer fallback, which can be proportional.

## Correction

`monospace_families` now requires fixed-pitch metrics and nonzero glyph mappings
for all 95 printable ASCII characters in the same inspected face. This is the
code/comparison font catalog contract, not a font-name blacklist. The general
`installed_families` catalog still contains all valid installed family names,
including Noto Color Emoji. Rendering fallback and saved font names are unchanged.
The original equal-column assertion and tolerance remain unchanged.

The eligibility work adds at most 95 cmap lookups per candidate face. Existing
candidate bounds, cancellation checks and completed-result sharing remain intact.
No dependencies or installed system/user fonts were changed.

## Reproduction and verification

An official Ubuntu `fonts-noto-color-emoji` package, version
`2.047-0ubuntu0.24.04.1`, was downloaded from the Noble updates archive and extracted
only into `_build/qa/monospace-20261112/fixture`. A child-process `FONTCONFIG_FILE`
includes ordinary system fonts, this fixture and an isolated cache directory.
`fc-match` confirms that the requested family resolves to the fixture. The package
is a local test input and is not bundled with the application.

- Package SHA-256:
  `b102b62ce6a7313315223cff5e052bdf8fc0ad162c9e961ce0dc2202bc139ce2`.
- Font SHA-256:
  `93cdc4ee9aa40e2afceecc63da0ca05ec7aab4bec991ece51a6b52389f48a477`.
- RED: the unchanged width test reproduced **35.5625 versus 126.5625**, and the new
  no-fallback coverage test failed on Noto Color Emoji's missing `!` mapping.
- GREEN with the fixture: **4/4** monospace integration tests passed, including
  general catalog membership, completed-cache identity and cancellation checks.
- GREEN with ordinary installed Linux fonts: **4/4** tests passed.
- Targeted strict Linux Clippy passed with `-D warnings`; Rust formatting and
  diff-whitespace checks passed. The coordinated renderer lib/raster/source
  continuity follow-up also passed **14/14** tests.
- Independent review found no actionable issue in the bounded coverage check or
  preserved fallback/cancellation behavior.

Commands use the existing locked/offline Linux toolchain and native `_build/linux`
target: `cargo test -p midnight-viewer-render --test monospace`, with the local SDK
configuration supplied by the QA scripts. Exact commands, package metadata and
logs are in `_build/qa/monospace-20261112/{qualify.sh,ordinary.sh,fixture.log,red.log,
green-fixture.log,green-ordinary-lint.log}`. The final Windows monospace integration also passed 4/4. The affected-package
strict Clippy gate and combined debug build passed; see
[the integration record](2026-11-12-tree-delete-integration.md).

These local results do not establish a green remote pipeline. The earlier GitHub
head did not contain this correction; remote status must be checked after pushing
the integrated batch.
