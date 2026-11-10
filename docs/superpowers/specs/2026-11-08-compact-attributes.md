# Compact attributes editor

The attributes chooser now uses one three-state checkbox per attribute instead
of three separate buttons. A dash and `Unchanged` preserve each selected item's
existing value; a checkmark and `Set` request the flag; an empty box and `Clear`
request its removal. Compression uses `Compress` and `Uncompress` captions.
The state text is always visible, also to assistive technology, so preserving a
mixed selection cannot be mistaken for clearing it.

Click or Space cycles states. Shift+Space reverses the cycle; Delete/Backspace
returns to Unchanged. Unsupported native capabilities are disabled. Windows flags
use the common capability mask of the captured selection. Unix permissions retain
their separate octal editor. No filesystem operation runs on the UI thread.

The Windows flags always use one vertical column, including in wide windows.
One shared `Include subfolders` checkbox sits outside the scrolling topic content,
above the footer. It keeps its position and value when switching Attributes,
Times and Case, and applies to the whole draft. Its tooltip explains the shared
scope; the review screen shows the prepared scope instead of an editable checkbox.
The default dialog target is 640 × 440 logical pixels;
large fonts expand its preferred size while the existing frame/scrolling retains
the 320 × 200 application minimum. Times and Case remain separate topics.

Review changes still validates the exact captured draft on the backend. Only the
matching completed revision can enable Apply. Refresh remains read-only, Cancel
does not apply edits, and no controls imply that metadata has already changed.

Salamander references: `src/dialogs.cpp` (`CChangeAttrDialog::Transfer`) and
`src/lang/lang.rc` (`IDD_ATTRIBUTES`, `BS_AUTO3STATE`). This replaces the expanded
attribute-button design in the 6 November metadata refinement; no source/assets
are copied and no new dependencies are required.
