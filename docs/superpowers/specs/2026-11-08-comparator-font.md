# Text comparator font

The text comparator uses fixed-pitch fonts independently of the application's
interface and the text/source viewers. Its toolbar offers installed monospaced
families and **System monospace**. On narrow windows the picker moves to its own
compact row; image comparison does not show text font controls.

Default: Consolas when available. An unavailable stored family falls back to
Consolas, then the system monospace family. An explicit System monospace choice
always uses the system's generic family. Unicode glyph fallback remains enabled.
Existing font size follows the application's base size; this request changes the
font family, not zoom or font size controls.

The existing Parley/Fontique and Swash renderer discovers fonts on a backend
worker, checking fixed-pitch metadata. A bounded completed family catalog is
cached for the process. Cancellation does not publish a partial catalog.
The picker never performs filesystem enumeration or text layout on the UI thread.
User choices redraw existing comparison rows without rereading or recomputing the
diff, and retain the viewport. Unknown/proportional choices are rejected by the
worker. Family names are passed literally, not interpreted as CSS family lists.

The existing viewer preference patch/persistence path saves only an explicit
choice in `viewer.comparison_font`. Schema 22 adds this field; earlier schemas
retain their values and receive the Consolas default. A missing installed font
does not overwrite the saved choice. Image comparison and viewer font choices
are unaffected. No new third-party dependency or plugin ABI change is required.

Validation covers codec round trips/migration/invalid fields, fallback choice,
equal-width rendering, explicit picker changes, rapid consecutive choices,
viewport retention and the 320 × 200 minimum window.
