# System-locale panel sorting

The user requires file-panel ordering to respect the system locale. Names and
extensions use native collation, and equal size/date values use the same locale
name ordering. Directory links remain grouped with directories ahead of files;
unknown sizes, dates and missing extensions remain last in either direction.
Native case and accent weights are retained. Numeric/natural sorting is outside
this change.

`LocaleService::collation_key` produces an opaque key from an exact native name
on a backend worker. Windows captures the user locale name and uses Unicode
`LCMapStringEx(LCMAP_SORTKEY)`. Linux captures `LC_ALL`, then `LC_COLLATE`, then
`LANG`, defaulting to `C`; private locale objects use `wcsxfrm_l` without changing
the process locale. If an installed Linux locale is unavailable, a private C
locale keeps browsing usable. Other native transformation failures are reported
as listing errors instead of claiming a locale-sorted result.

Entries retain name/extension keys through shared immutable caches. Re-sorting a
captured listing performs no native collation calls. Comparators perform only
key/raw-name comparisons; they never call FFI. Preparation checks cancellation
between native calls, and publication checks it before committing a snapshot.
Native output length checks and a 4 MiB budget per generated key bound allocation
without truncating names or imposing a pathname length limit.

Invalid UTF-8 on Linux and unpaired UTF-16 on Windows retain their exact native
identity. Their unavailable name keys sort after collatable names, then by raw
native name. Equal native keys use raw name and entry identity as deterministic
ties. Sort keys are session data and are never persisted or treated as identity.

Windows byte keys remain opaque, including internal zero bytes. Linux wchar
weights are encoded in native comparison order, including the terminator, so
byte comparison agrees with `wcscmp` across supported wchar representations.

Validation covers Czech h/ch/i contraction and accents using native locale
objects, direct-native case ordering, raw-name fallback, ascending/descending
ordering, metadata ties, cached re-sorts, cancellation and error publication.
Native platform tests are separate files; Linux Czech fixtures use a private
locale directory under `_build` when the host has no Czech locale installed.

API references: [Microsoft LCMapStringEx](https://learn.microsoft.com/en-us/windows/win32/api/winnls/nf-winnls-lcmapstringex)
and [GNU libc collation functions](https://sourceware.org/glibc/manual/latest/html_node/Collation-Functions.html).
Salamander's `src/sort.cpp` likewise delegates linguistic comparison to the
user-locale API; its ANSI transport and configurable case/numeric policies are
not copied into this Unicode implementation.
