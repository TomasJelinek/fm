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
star, bookmark-add, copy, move, help, settings, attributes and update (a circular
arrow around a download arrow, added 23 November 2026). Each has an original .svg
light variant and a -dark.svg variant.

Action glyphs share one style in every window and on every platform: labelled
buttons (dialogs, Find, Jobs, the viewer and the comparator included), toolbar
buttons, function keys, menu rows and tab close buttons draw the bundled glyph
as a monochrome mask in the theme's dimmed foreground (`secondary-text`,
`muted-text` while unavailable; destructive and removal actions use the error
color of their caption). Close uses the exit glyph, never a red cross. Host
artwork is not used for an action that has a bundled glyph, because it differs
per platform and icon theme and is often colored; a labelled button without a
glyph tints its host artwork the same way. Dialog header icons keep the colored
variants and native artwork; the application logo, shell icons, custom-command
icons and file, archive and compression artwork keep their own colors.

The artwork adds no dependency, feature or third-party notice.
