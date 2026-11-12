# Action icon artwork

The command icons refined on 8 November 2026 are original SVG artwork within
Midnight Explorer. No external icon package, image, font, library or license was
added. Existing command metaphors were simplified and given one accent color
with a neutral outline; light and dark variants preserve contrast on application
surfaces. Navigation arrows and close controls retain neutral theme coloring.

The design references were official guidance, not copied assets:

- [Fluent 2 iconography](https://fluent2.microsoft.design/iconography): recognizable
  metaphors, restrained detail, clear small-size rendering and careful color use.
- [Fluent 2 tags](https://fluent2.microsoft.design/components/web/react/core/tag/usage):
  removable values and complete wrapping labels informed the archive extension
  editor. Its controls and artwork are application-owned implementations.
- [Material 3 icons](https://m3.material.io/styles/icons/overview) and
  [chips](https://m3.material.io/components/chips/overview) were consulted as
  references; their pages required JavaScript and supplied no copied material.

Artwork lives in crates/midnight-ui/ui/icons/. The colored command names are
checksums, compare-files, compare-directories, filter, find, folder-sizes, view,
star, bookmark-add, copy, move, help, settings and attributes. Each has an
original .svg light variant and a -dark.svg variant. The shared renderer in
action-icons.slint chooses the variant and preserves its accent; unavailable
command fallbacks use the muted theme color. Native artwork retains its RGB
values; disabled standalone native action images reduce opacity. File, archive
and compression artwork is unchanged.

No dependency, feature, third-party notice or project license changed.
