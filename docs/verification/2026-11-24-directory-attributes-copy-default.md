# Directory attribute details and Copy defaults — 24 November 2026

Compare directories now retains top-level badges and their reason even without
Compare subfolders. Differing folders whose children were not examined no longer
imply matching descendants. Copy/Move starts with Preserve times and attributes
checked; an explicit saved false choice and a per-job opt-out remain effective.

## Reproduction and results

The nonrecursive attribute regression failed before the fix because `marks` was
absent. The unexamined-child regression failed with an inferred `Same` instead of
no outcome. All six copy metadata settings tests failed before the default and
serialization change, then passed as part of the settings group below.

A one-off read-only native probe in the validation export used
`LocalFileSystem::entry_metadata` and the comparison engine on the requested two
folders, with attributes enabled and size, time, content and recursion disabled.
It reported `selected [0, 0], marks 14, unknown 0, identical true`. All 14 top-level
file pairs matched. A separate metadata inspection found 11 pairs with permissions
0644 and three with 0664; each pair also had equal size, modification time,
ownership and extended attributes. Folder names differed, but folders were excluded.
No content comparison or copy of these inputs was performed.

## Targeted validation

Linux, offline locked Cargo builds. A source export under `_build/` isolated this
change from concurrent manifest edits in the shared checkout. UI linking used the
installed Fontconfig 2.17.1 runtime with local pkg-config metadata under `_build/`:
the ordinary build lacked `fontconfig.pc`; the attempted dynamic-loading override
was incompatible with the pinned Fontique imports. No dependencies were changed.

| Target and filter | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-core --lib compare::` | 6 | 0 | 0 |
| `midnight-engine --lib comparison::` (includes the one-off native probe) | 51 | 0 | 1 |
| `midnight-platform-common --lib settings::` | 100 | 0 | 0 |
| `midnight-ui --lib copy_dialog::` | 36 | 0 | 0 |
| `midnight-ui --lib comparison` | 32 | 0 | 0 |
| `midnight-ui --lib preferences_load::` | 10 | 0 | 0 |
| `midnight-ui --test integration copy_dialog::` | 14 | 0 | 0 |
| `midnight-ui --test integration options_transaction::preserve_option_defaults_on_and_only_ok_persists_it` | 1 | 0 | 0 |
| `midnight-ui --test integration file_actions_ui::a_clipboard_move_keeps_links_unless_the_user_chooses_to_copy_their_files` | 1 | 0 | 0 |

Build commands used `cargo test -p <package> --lib --locked --offline`; compatible
UI/settings artifacts were compiled together with `--lib --test integration
--no-run`, then their executables ran with the filters above. Headless settings
and engine runs used four test threads; UI runs were serial. The ignored engine
test is the optional native tree timing benchmark. The final core inference change
was checked by the core and engine groups, including the ambiguous-folder case.

`cargo fmt -p midnight-core -p midnight-engine -p midnight-platform-common
-p midnight-ui -p midnight-acceptance -- --check`, `git diff --check` and
`bash scripts/check-source-size.sh` passed (3,499 source files within the limit).
An independent read-only review found the unexamined-child issue above and
confirmed its correction. Full workspace and desktop acceptance suites were not run.
