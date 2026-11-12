# User Menu and external-editor form refactor

The forms were checked against Salamander's actual User Menu sources before
implementation: `src/lang/lang.rc:247`, `src/dialogs4.cpp:1980–2341`, field menus in
`src/execute.cpp:223/321/407/1965`, and ordering in `src/edtlbwnd.cpp:369` plus
`dialogs4.cpp:2580`. No reference code or assets were copied.

Custom Commands now presents Menu Contents above the selected item's Command,
argument rows, Initial directory and toolbar appearance. Program, directory and
image Browse actions use the backend native-picker service. Advanced terminal
script/context controls remain separate. Editor settings shares the literal
argument-row control. Both retain actual drafts across page switches and use the
common Save footer; saving, editing and browsing never execute a command.
The existing drag controller, stable IDs, clipping, cancellation, autoscroll and
Up/Down actions remain intact. Native picked program/image paths retain their
original bytes/UTF-16 independently of display text.

## Causal checks

- `forms-widget-red.log`: both former forms expose ordinary JSON argument editing.
- `editor-cache-red.log`: loading saved custom editor preferences leaves the
  initial `0:0` global cache unchanged. The corrected key includes load state and
  editor configuration identity; the same assertion and actual row-edit/save
  workflow pass.
- The editor's long argument form uses the same unconditional scroll-layout
  wrapper required by pinned Slint's conditional-child handling. A real wheel
  event reaches the complete 64th argument above the scrollbar overlay.
- Hovered argument removal passes without a production tooltip change. The final
  Windows direct-link check drops the test's ItemRc before removal so the harness
  cannot keep the removed delegate alive accidentally.

## Fresh functional scope

Windows: **510 passed** in `windows-forms-final2.log`: UI library 488, existing
settings/drag 12, new forms 5, Editor 1, parent focus 1, color picker 1, path elision 1,
dialog resize 1. The late test-only ItemRc lifetime strengthening was rerun against
the exact compiled production dependencies: **5 forms passed** in
`forms-final-direct.log`.

Linux: **506 passed** in `linux-forms-final.log`: UI library 484 and the same 22
integration cases. This run predates only that test-handle lifetime strengthening;
production source is unchanged. Screenshots cover 1340×820, 720×500 and 320×200 in
`_build/goal/commands-refactor/ui` and `ui-linux`. The existing 12 settings cases
also exercise every earlier theme/density and genuine pointer drag/scroll paths.

The native Windows acceptance in `native-menu-acceptance.log` creates a command
through software-rendered widgets, adds literal/empty/Unicode argument rows,
saves once, explicitly clicks its toolbar action, then runs the captured request
through the real preparation/native process service on a backend thread. The
owned argv helper receives exact UTF-16 argument boundaries and cwd and writes
only beneath `_build`. No shell script or real preferences are used. Its first
fixture compared a mixed-separator cwd spelling against Windows' native spelling;
that fixture was corrected to construct native path components before comparison.

## Remaining qualification

The final UI/application all-target strict check is **not yet green**. The first
run found an unrelated viewer-engine `collapsible_if`; its owner fixed it. The
retry reached UI/application checking but failed an unrelated test borrow in
`metadata_editor.rs:125`; its owner scoped the popup Ref afterward. A fresh strict
rerun remains required. Linux strict has not been run for this form checkpoint.
No full-workspace or final release/native Options-window acceptance is claimed.

Working directory remains the existing template-string schema: an unrepresentable
native directory pick is refused explicitly, and dollar signs in picked literal
directories are escaped. Native picker platform/compositor qualification remains
documented by its backend owner. No new dependency, schema, normal profile or
deployed executable was changed by this form work.

Raw logs, replay scripts and the implementation plan are under
`_build/goal/commands-refactor/`.
