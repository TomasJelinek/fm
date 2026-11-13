# Comparator settings: UI design and wireframes

[Authoritative design](2026-11-13-comparator-associations-design.md) · [Implementation plan](../plans/2026-11-13-comparator-associations.md).

## Contract alignment

The linked design is authoritative for routing, limits, policy snapshots and native launch. These wireframes are planning illustrations, not current UI or measured layout evidence.

- Defaults are empty overrides/Automatic; the fixed final fallback row is read-only. Rule count, argv/expression/template budgets and omitted-cwd behavior come from the design.
- Built-in coverage uses current-session effective enabled non-document decoder metadata; an extension listing is neither installed capability proof nor a promise for mixed pairs or decodability.
- Argument preview means symbolic argument help only. It captures/reads no input, launches nothing and introduces no regex-preview service or per-keystroke compiler job.
- Use cheap presentation revisions and local selected-field mutation. The backend owns compiled accepted policy; UI must not deep-clone/hash the complete configuration on every paint.
- Backend/Core owns validation/token meaning and rule/field save feedback through existing settings acceptance. Bind to the accepted contract rather than adding another template parser or validation service.

Design/source audit only, against Root's supplied main HEAD
`4f27e16c6ad233b1294d01d446ae560d8b9a3a9a`. No product files, the four
unqualified performance_probe examples, or frozen candidates were changed. No
compiler, tests, native application, dependency installation or Git mutation ran.
This document proposes presentation and UI ownership; Architect/Root own the
shared contract and implementation authorization.

Use one **Comparators** page after Editors. Its first section visibly explains
the built-in assignments; its second section edits ordered overrides. The selected
rule's fields stay inline below the list, following Editors. This keeps the
program and arguments visible in the normal Options transaction. A separate rule
dialog adds navigation, while an editable extension matrix would confuse fixed
decoder coverage with wildcard/regex priority rules; neither is recommended.

**What the page shows**

1. **Built-in types — when no override matches**, read-only. Show enabled image
   extensions, wrapping every mask, mapped to **Automatic → Built-in image**.
   Show **Other regular files → Automatic → Built-in text**, with the short note
   “Text must be UTF-8 or UTF-16 with a BOM.” This is routing information, not a
   promise that every file with an extension is decodable. A missing/disabled
   image decoder shows its actual availability instead of fabricated support.
   Exclude document-only providers such as PDF. This section never makes settings
   dirty and has no add/remove/priority actions.
2. **Rules**, an ordered list with only **+ − ↑ ↓** in its heading. Every row
   visibly shows enabled/off state, pattern and assigned comparator/program.
   The first enabled rule matching **both basenames** wins. With no overrides,
   show “No overrides. The built-in assignments above apply.” The page therefore
   remains informative on its first opening.
3. **Selected rule**: Enable checkbox; Wildcards/Regular expression; filename
   pattern; Case sensitive; **Compare using** choice: Automatic, Built-in text,
   Built-in image, External program. Keep the selected complete pattern in a
   character-wrapped summary as well as its editable field. List elision/tooltips
   alone do not provide full pattern access on touch or a small window.
4. **External program** reveals an editable executable path and Browse, literal
   argument rows, optional working directory and Browse, then an inert argument
   preview. Other target choices hide these fields while retaining their edits
   within the current draft. There is no system-association comparator choice,
   comparator-plugin catalog, Run/Test button or extra Save button.

The image masks are supplied by the backend's effective enabled decoder catalog,
using the same policy as comparison launch. The source image manifest currently
advertises png/jpg/jpeg/gif/webp/bmp/ico/tif/tiff/svg/avif; this is a design example,
not a hardcoded UI list or proof those packages are installed. Current-session
availability must not be confused with Plugins changes awaiting restart. Discovery
uses existing image-provider metadata; this design adds no plugin ABI.

**Desktop wireframe — illustrative enabled decoder data and custom rules**

```text
Options                                                              ×
General       Comparators
Appearance    Built-in types — when no override matches
Keyboard      *.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.ico;
Editors       *.tif;*.tiff;*.svg;*.avif          → Automatic → Image
Comparators   Other regular files              → Automatic → Text
Plugins       Rules                                      +  −  ↑  ↓
...           ✓ *.c;*.cpp;*.h;*.hpp            → External: DiffTool
              ✓ *.log                          → Built-in text
              — *.png                          → Built-in image
              Other file types                 → Automatic (fixed)
              [✓] Enable this rule
              [Wildcards v] [*.c;*.cpp;*.h;*.hpp                 ]
              [ ] Case sensitive
              Compare using [External program v]
              Executable [D:\Tools\DiffTool\diff.exe     ] [Browse…]
              Arguments                                  +  −  ↑  ↓
              1  {left}
              2  {right}
              [selected argument value                         ]
              [Insert left file] [Insert right file]
              Working directory [Executable folder (default)] [Browse…]
              Argument preview — does not run
              Program: D:\Tools\DiffTool\diff.exe
              Argument 1: {left}  → left file path
              Argument 2: {right} → right file path
              Working directory: executable folder
Reset to defaults                                      [OK] [Cancel]
```

DiffTool is an illustrative user-supplied executable, not a dependency or product
recommendation. Preview displays separate program/argv/cwd values, not a shell
command that invites quoting guesses. Use symbolic sample file values; opening
Options must not capture/read the user's files. Core owns whole-item token meaning; UI displays symbolic values without
another parser or backend regex-preview worker.
Long values wrap and scroll without truncating stored values.

**Compact wireframe — 320×200, body at its initial scroll position**

```text
┌────────────────────────────────────┐
│ [Comparators                  v] × │
│ Built-in types                     │
│ *.png;*.jpg;*.jpeg;*.gif;*.webp;     │
│ *.bmp;*.ico;*.tif;*.tiff;*.svg;     │ ↕ body scrolls
│ *.avif → Automatic → Image         │
│ Other files → Automatic → Text     │
│────────────────────────────────────│
│ [↺]                   [OK] [Cancel]│
└────────────────────────────────────┘
```

The rest of the same page scrolls below this initial view. At short height, merge
the topic picker and title into one header row, retaining the Options accessible
dialog name and close control; this leaves room for a full-height field even with
touch/font enlargement. Keep OK/Cancel/Reset in the existing fixed DialogFooter.
Inline validation belongs in the scrollable body and is revealed on error, rather
than adding fixed chrome that consumes the remaining body. Narrow executable and
cwd Browse controls move below their fields. A heading caption can sit above its
four-button action row when needed; touch targets never shrink to fit. Exact
geometry must be qualified later, not inferred from the existing Cancel-only test.

**Editing and transaction behavior**

- Reuse Editors' retained all-rule draft: selection, reorder and topic changes
  preserve incomplete edits. Add selects a new rule; remove selects its nearest
  surviving neighbor; moving a row retains its identity and draft. Backend bounds
  control action availability. A new external configuration starts with the two
  complete arguments `{left}` and `{right}`; no paths are appended implicitly.
- Reuse ArgumentEditor literal-row editing and ListHeader actions. Supply only
  comparator insertion choices, **Insert left file** and **Insert right file**;
  neither the editor `{file}` nor the Custom Commands `${…}` menu belongs here.
  Every row is one argument. Show **No arguments** for zero rows and **"" (empty
  string)** for an intentional empty argument. A pending blank add is visibly a
  new-entry editor; Enter commits one empty argument and consumes that key.
- Symbolic argument help reflects the latest draft only. It never launches a
  program, invokes a shell, browses/captures inputs or extracts an archive.
  Browsing only chooses program/cwd paths. Retain original raw native paths
  separately from display text; stale picker/validation replies cannot overwrite
  a changed rule, selected target or cancelled Options transaction.
- Normal **OK** validates all saved rules, including disabled rules, from any
  topic, then performs the existing single settings save. An invalid result
  reveals Comparators, the exact rule and offending field; preserve the draft.
  A persistence failure leaves Options open for retry/Cancel. Closing follows the
  existing successful-save acknowledgement, not merely validation completion.
- **Cancel/Escape/×** roll back the entire Options draft. **Reset to defaults**
  stages the Architect-approved comparator default in the same transaction;
  proposed default is no overrides plus Automatic. Cancel restores previous
  rules; OK persists the reset. No page Save or Discard button is introduced.
- Keyboard: list Up/Down/Home/End, Insert/Delete, Alt+Up/Down; Tab reaches every
  field/action and reveals it in the body. Editing keys affect their field.
  Theme colors/metrics and WrappingCheckBox distinguish selected row and keyboard
  focus, support touch and font changes, and preserve caret/selection on resize.

**Proposed file ownership for the later UI task — no reservation or edits yet**

| Ownership | Paths relative to D:/experiments/dev/FM |
| --- | --- |
| New, exclusive UI | `crates/midnight-ui/ui/comparator-options.slint`; `crates/midnight-ui/src/comparator_options_presentation.rs`; `crates/midnight-ui/src/presenter/comparator_draft.rs`; `crates/midnight-ui/src/presenter/comparator_draft_tests.rs`; `crates/midnight-ui/tests/comparator_options.rs` |
| Coordinated shared Slint hunks | `crates/midnight-ui/ui/options-dialog.slint`; `options-navigation.slint`; `argument-editor.slint`; `main.slint` |
| Coordinated shared Rust hunks | `crates/midnight-ui/src/lib.rs`; `src/presenter.rs`; `src/presenter/preferences.rs`; `src/presenter/preferences/options.rs`; `src/presenter/settings_picker.rs` |
| Direct test extensions | `crates/midnight-ui/tests/argument_lists.rs`; `options_transaction.rs`; `options_reflow.rs`; `crates/midnight-ui/src/presenter/settings_picker_tests.rs`; `src/presenter/preferences/options_tests.rs` |

Reuse ListHeader and literal argument mutation/sync without feature edits to
editor-options.slint or argument_rows.rs. ArgumentEditor needs optional supplied
insertion choices/helper text, preserving existing consumers. Keep topic IDs
0,1,3–9 stable (2 is retired); propose new comparator ID 10 inserted visually
after Editors using explicit index↔ID mapping. Settings picker context follows
that ID. Backend owns preferences/schema, matching, validation/template meaning,
capabilities and process launch; Docs owns updates to FILE_COMPARISON.md. No
comparator-window or directory-comparison implementation is assigned here.

**Direct acceptance cases for later implementation**

1. First open with zero overrides still displays real enabled image assignments
   and text fallback; missing/disabled/PDF-only capability states are truthful.
2. Ordered enabled overrides, including a .c/.cpp pair, round-trip through OK;
   selecting/reordering/changing topics retains unfinished fields. Invalid regex
   or target arguments reveal the exact row; failed save and Cancel retain the
   existing transaction guarantees. Reset then Cancel restores all prior rules.
3. Literal Unicode/space/quote/empty arguments and complete left/right insertion
   round-trip without a hidden appended argument; Enter in the argument field
   cannot accidentally accept Options.
4. Editing, picking, capability display and preview emit no comparator launch.
   Stale preview/picker replies after edit, type switch, remove or Cancel are
   rejected; unchanged raw Windows/Unix program and cwd paths survive.
5. At 320×200 and desktop size, keyboard/touch/font-32/theme cases keep a usable
   body and reachable footer. Every full pattern/path/preview value is readable by
   scrolling, with selected row, focus and caret retained after resize.

These are planned tests, not executed evidence. Existing editor_options,
argument_lists, options_transaction, options_reflow and picker unit tests provide
the analogous boundaries; full suites remain Build-coordinated after integration.

**Shared handoff decisions**

Use the settled both-basename/first-enabled rule policy, Automatic/Text/Image/
External targets and whole-native-argument `{left}`/`{right}` substitution. Bind UI to
the accepted settings validation/save feedback, exact rule/field identity and
backend effective-capability snapshot; symbolic argument help is not a new
validation/preview service. Defaults are empty overrides/Automatic, omitted cwd
uses the executable parent, and bounds come from the design. The compact header
merge and stable topic ID 10 are planned UI details, not current implementation.

Source evidence: editor-options.slint:38–240, list-header.slint:6–34,
argument-editor.slint:48–184, options-navigation.slint:4–8,
options-dialog.slint:96–201/331–355, presenter/editor_draft.rs:149–318 and
presenter/preferences/options.rs:93–239. Important existing gap: editor draft
acceptance does not compile regex; settings encoding does, so comparator validation
must report field/rule errors before the final save. Decoder routing evidence:
midnight-compare-engine/src/worker.rs:136–183 and
midnight-image-viewer-host/src/providers.rs:15–43; see
docs/developer/FILE_COMPARISON.md for the existing captured A/B choice boundary.
