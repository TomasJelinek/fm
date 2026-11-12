# Literal three-dot path elision

Shortened displayed paths use the literal ASCII marker `...`. Upper panel
breadcrumbs keep their beginning and end; other path labels reserve a separate
marker at the right edge. The shared `PathText` component measures the complete
text at the current font and clips only its presentation. It performs no path
parsing, filesystem work or modification of the source string.

The audited displays are upper breadcrumbs, Find progress and result paths,
Find search-folder choices, comparison headings, checksum result paths,
saved-folder management, history, location and Hot Paths pickers. Bounded custom
command previews use `... [display truncated]`. Ordinary captions retain their
existing overflow behavior. Full native paths, clipboard/navigation identities,
path editors and the scrolling command-line path retain their original values.

The `midnight-ui` integration target `path_elision` exercises rendered text and
geometry, rather than matching source-code strings. It covers breadcrumb root
navigation/copy and full editing, Find and comparison path accessibility,
Unicode/combining-character paths at narrow widths and two font sizes, omission
markers disappearing when unnecessary, saved-path selection and Find root
selection/removal while preserving the complete model values. The custom-command
tooltip unit test checks the bounded Unicode preview marker and omitted private
arguments/script contents.

Initial rendered regressions failed for the old breadcrumb, Find and comparator
markers; the tooltip regression failed for the old preview marker. Logs are under
`_build/qa/archive-alignment/`. The additional Find-root picker regression was
added before its replacement but its intermediate build was stopped to batch
shared UI generation; it has no claimed pre-fix runtime failure. Final validation
passed all **6 path tests** within the combined **42-test UI gate**, with no
failures. The log is `_build/qa/path-elision/final.log`. The other gate targets
cover archive options, commander controls, Find windows, the shared toolbar and
the minimum window size. The path target can be repeated with
`scripts/cargo.ps1 test --offline -p midnight-ui --test path_elision`.
The focused custom-command tooltip unit test also passed (**1 passed, 0 failed**);
its log is `_build/qa/path-elision/tooltip-green.log`.

Visual inspection of the captures under `_build/qa/path-elision/` confirmed the
complete marker in 64px and 120px fixture windows at the larger font, no marker
for the short Unicode path, middle breadcrumb elision, Find progress and result
columns, both comparison headings, and opened saved-path/search-root menus.
The clipped source text does not paint over the reserved dots. Captures use the
software renderer on the Windows test host; they do not by themselves qualify
every native backend, DPI setting or installed font. Native Find/comparator
probes are tracked separately from this presentation-only check.
