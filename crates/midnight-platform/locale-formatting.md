# System locale formatting

Locale services capture number preferences on a backend worker. The UI receives an
immutable `NumberLocale` and formats exact byte counts directly from decimal integer
digits, including `u64::MAX`, nonbreaking spaces, and repeating or finite grouping.
Readable sizes also use integer arithmetic and the captured decimal separator.
The initial fallback is the C locale until the worker returns its snapshot.

Windows reads the current user's NLS overrides with `GetLocaleInfoEx`, verifies
number grouping against `GetNumberFormatEx`, and formats metadata using
`GetDateFormatEx` and `GetTimeFormatEx`. Captured timezone rules convert the original
timestamp before formatting. Date and time remain separate strings throughout the
metadata and panel-details APIs, so spaces and AM/PM text are retained.

Linux chooses `LC_ALL`, then the relevant `LC_NUMERIC` or `LC_TIME`, then `LANG`,
then C. Each worker owns private `newlocale` objects; the adapter never changes the
process locale with `setlocale` or installs a thread locale with `uselocale`.
`nl_langinfo_l` provides grouping and separators, and `strftime_l` provides `%x`
and `%X`. Text from a non-UTF-8 locale is converted with `iconv` without replacement
characters. Invalid or unavailable locales return a discovery error; metadata
whose native date cannot be formatted retains its original sortable timestamp.

Locale choices are captured on first backend use. Restart after changing system
locale preferences. UI formatting performs no native locale or timezone lookup;
when localized metadata is unavailable, it displays a labelled UTC fallback or
an unknown marker for timestamps outside the supported display calendar.

Tests inject Czech-style nonbreaking-space grouping and Indian grouping, exercise
the full unsigned integer range, and compare Windows and Linux timestamps with
their native formatters. UI tests cover row presentation, separate panel fields,
clipboard values, and partial folder-size markers.

Native API references:

- [Windows number formatting](https://learn.microsoft.com/en-us/windows/win32/api/winnls/nf-winnls-getnumberformatex)
- [Windows date formatting](https://learn.microsoft.com/en-us/windows/win32/api/datetimeapi/nf-datetimeapi-getdateformatex)
- [Windows time formatting](https://learn.microsoft.com/en-us/windows/win32/api/datetimeapi/nf-datetimeapi-gettimeformatex)
- [POSIX locale objects](https://man7.org/linux/man-pages/man3/newlocale.3.html)
- [Locale information](https://man7.org/linux/man-pages/man3/nl_langinfo.3.html)
- [Native date/time formatting](https://man7.org/linux/man-pages/man3/strftime.3.html)
