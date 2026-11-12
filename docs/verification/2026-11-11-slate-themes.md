# Slate theme verification, 11 November 2026

The screenshot-inspired Slate family adds three original palettes: Slate Blue,
Slate Teal and Slate Violet. All share neutral charcoal surfaces, raised gray
controls and near-white text. Their accents, active panel headers and selection
surfaces differ. IDs 23–25 append to the catalog; existing saved choices and the
Blue default remain unchanged. No dependency or plugin API changes are involved.

## Targeted evidence

The following integration tests passed on Windows x64 using the debug/test
configuration, the pinned offline SDK and the existing `_build` target:

| Package / integration test | Passed | Test execution time |
| --- | ---: | ---: |
| midnight-core / themes | 6 | <0.01 s |
| midnight-platform-common / filename_colors | 8 | 0.14 s |
| midnight-platform-common / theme_preferences | 4 | 0.62 s |
| midnight-ui / filename_colors | 3 | 1.51 s |
| midnight-ui / themes | 1 | 16.43 s |
| midnight-ui / presenter theme_tests | 3 | 0.12 s |

The Options test selects all 26 themes through the real picker in both desktop
and touch density. It checks semantic text, borders, focus/selection indicators,
error colors, edit selection and the actual top toolbar background. The new
Slate palettes require at least 7:1 for the tested row text roles across all five
row backgrounds; ordinary control text retains the 4.5:1 minimum.

The filename-color matrix covers all eleven default categories, 26 themes, four
selection/focus states and both active/inactive panels: 2,288 rendered category
cases. Each category's foreground is also checked against both alternating row
backgrounds at a minimum of 4.5:1. Pixel checks confirm the intended filename ink
is actually drawn. Existing custom foreground/background, view-mode, search and
selection-cue checks passed too. This is scoped palette evidence, not a claim of
whole-application accessibility conformance.

All 25 targeted tests passed. The presenter checks cover saved-choice mapping,
early previews surviving delayed preference loading, and unknown-index fallback.
The persistence tests exercise all 26 storage names and preserve unrelated
preferences. Default/category rules remain user-configurable and are not reset
when selecting a Slate theme.

## Visual review and artifacts

Software-rendered snapshots of all three normal Slate variants and the inactive
selected/focused Slate Teal state were inspected. They retain the neutral surface
hierarchy and distinguish the filename categories and selection markers.

Logs and original BMP snapshots are under `_build/qa/slate-themes-20261111/`.
Lossless PNG copies for review are `render/slate-blue.png`, `render/slate-teal.png`
and `render/slate-violet.png`. These are actual rendered test fixtures, not mockups.
Build artifacts and screenshots are not committed.

The focused integration run uses `--test themes --test theme_preferences
--test filename_colors` with the app/UI/viewer packages selected together to retain
the existing feature unification. Tests execute in seconds; the initial generated
UI recompile and linking took 12 min 11 s. Full workspace suites were not rerun for
this palette-only change.

## Build and startup

The fresh `me.exe` debug build passed in 35.22 s after the targeted test builds.
Its SHA-256 is
`069e08775c013097f62bbf5e1e0814975dd7c2ae677b601bda628433ee232385`.
A matching protected runtime containing viewers, filesystems and thumbnailers was
deployed and registered using the existing deployment script. The installed EXE
hash matches the build output.

The new `_build/debug/me.exe` passed `--ui-smoke-test` with both panes pointing at
the isolated QA directory, produced `app.png`, exited successfully and wrote no
stderr. User preferences were not modified; the startup smoke uses the default
Blue theme while the dedicated rendering tests exercise the new Slate variants.
`cargo fmt --all --check` and `git diff --check` passed. Changed handwritten source
files are below the 100,000-byte limit and use LF line endings. An independent
read-only code review found no issues in the theme patch.
