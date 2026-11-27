"""Windows Installer databases through the operating system's own msi.dll (Windows only).

The release MSI is written and read with the Windows Installer API that every supported
Windows version ships, so building it needs no third-party installer toolkit. Only the
calls that `package-msi.py` uses are bound here.
"""
import ctypes
from ctypes import wintypes
import os

MSIDBOPEN_READONLY = 0
MSIDBOPEN_TRANSACT = 1
MSIDBOPEN_CREATE = 3
MSIMODIFY_INSERT = 1
MSICOLINFO_TYPES = 1
ERROR_SUCCESS = 0
ERROR_MORE_DATA = 234
ERROR_NO_MORE_ITEMS = 259
MSI_NULL_INTEGER = -0x80000000

VT_I2 = 2
VT_I4 = 3
VT_LPSTR = 30
VT_FILETIME = 64

# Summary information property IDs.
PID_CODEPAGE = 1
PID_TITLE = 2
PID_SUBJECT = 3
PID_AUTHOR = 4
PID_KEYWORDS = 5
PID_COMMENTS = 6
PID_TEMPLATE = 7
PID_REVNUMBER = 9
PID_PAGECOUNT = 14
PID_WORDCOUNT = 15
PID_APPNAME = 18
PID_SECURITY = 19
SUMMARY_TYPES = {PID_CODEPAGE: VT_I2, PID_TITLE: VT_LPSTR, PID_SUBJECT: VT_LPSTR,
                 PID_AUTHOR: VT_LPSTR, PID_KEYWORDS: VT_LPSTR, PID_COMMENTS: VT_LPSTR,
                 PID_TEMPLATE: VT_LPSTR, PID_REVNUMBER: VT_LPSTR, PID_PAGECOUNT: VT_I4,
                 PID_WORDCOUNT: VT_I4, PID_APPNAME: VT_LPSTR, PID_SECURITY: VT_I4}

MSIHANDLE = wintypes.ULONG
_msi = None


class MsiError(RuntimeError):
    pass


class Stream:
    """A record field whose value is the content of a local file (Icon, _Streams)."""

    def __init__(self, path):
        self.path = os.fspath(path)


def system_folder():
    """The Windows system folder (System32), from the API rather than the environment."""
    if os.name != "nt":
        raise MsiError("Building or reading an MSI needs Windows (its Windows Installer API)")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    buffer = ctypes.create_unicode_buffer(32768)
    length = kernel32.GetSystemDirectoryW(buffer, len(buffer))
    if not 0 < length < len(buffer):
        raise MsiError("The Windows system folder could not be determined")
    return buffer.value


def _system_library(name):
    # Load from the system folder by full path, never through the search path.
    return ctypes.WinDLL(os.path.join(system_folder(), name))


def _library():
    global _msi
    if _msi is not None:
        return _msi
    if os.name != "nt":
        raise MsiError("Building or reading an MSI needs Windows (its Windows Installer API)")
    msi = _system_library("msi.dll")
    signatures = {
        "MsiOpenDatabaseW": (wintypes.LPCWSTR, ctypes.c_void_p, ctypes.POINTER(MSIHANDLE)),
        "MsiDatabaseOpenViewW": (MSIHANDLE, wintypes.LPCWSTR, ctypes.POINTER(MSIHANDLE)),
        "MsiDatabaseCommit": (MSIHANDLE,),
        "MsiViewExecute": (MSIHANDLE, MSIHANDLE),
        "MsiViewFetch": (MSIHANDLE, ctypes.POINTER(MSIHANDLE)),
        "MsiViewModify": (MSIHANDLE, ctypes.c_int, MSIHANDLE),
        "MsiViewGetColumnInfo": (MSIHANDLE, ctypes.c_int, ctypes.POINTER(MSIHANDLE)),
        "MsiViewClose": (MSIHANDLE,),
        "MsiCloseHandle": (MSIHANDLE,),
        "MsiRecordSetStringW": (MSIHANDLE, wintypes.UINT, wintypes.LPCWSTR),
        "MsiRecordSetInteger": (MSIHANDLE, wintypes.UINT, ctypes.c_int),
        "MsiRecordSetStreamW": (MSIHANDLE, wintypes.UINT, wintypes.LPCWSTR),
        "MsiRecordGetStringW": (MSIHANDLE, wintypes.UINT, wintypes.LPWSTR,
                                ctypes.POINTER(wintypes.DWORD)),
        "MsiRecordReadStream": (MSIHANDLE, wintypes.UINT, ctypes.c_char_p,
                                ctypes.POINTER(wintypes.DWORD)),
        "MsiGetSummaryInformationW": (MSIHANDLE, wintypes.LPCWSTR, wintypes.UINT,
                                      ctypes.POINTER(MSIHANDLE)),
        "MsiSummaryInfoSetPropertyW": (MSIHANDLE, wintypes.UINT, wintypes.UINT, ctypes.c_int,
                                       ctypes.c_void_p, wintypes.LPCWSTR),
        "MsiSummaryInfoGetPropertyW": (MSIHANDLE, wintypes.UINT, ctypes.POINTER(wintypes.UINT),
                                       ctypes.POINTER(ctypes.c_int), ctypes.c_void_p,
                                       wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)),
        "MsiSummaryInfoPersist": (MSIHANDLE,),
        "MsiFormatRecordW": (MSIHANDLE, MSIHANDLE, wintypes.LPWSTR,
                             ctypes.POINTER(wintypes.DWORD)),
    }
    for name, arguments in signatures.items():
        function = getattr(msi, name)
        function.argtypes = arguments
        function.restype = wintypes.UINT
    for name, arguments, result in (
            ("MsiCreateRecord", (wintypes.UINT,), MSIHANDLE),
            ("MsiGetLastErrorRecord", (), MSIHANDLE),
            ("MsiRecordGetFieldCount", (MSIHANDLE,), wintypes.UINT),
            ("MsiRecordGetInteger", (MSIHANDLE, wintypes.UINT), ctypes.c_int),
            ("MsiRecordIsNull", (MSIHANDLE, wintypes.UINT), wintypes.BOOL)):
        function = getattr(msi, name)
        function.argtypes = arguments
        function.restype = result
    _msi = msi
    return msi


def rehearse_actions(path, actions, properties=()):
    """Run `actions` of the package at `path` in a client session that installs nothing.

    The session ignores this machine's installation state and shows no UI; it checks
    that the package's own actions (searches, costing, property settings) run, the way
    they do before the first dialog of a real installation. Raises MsiError naming the
    first action that fails, with Windows Installer's error text.
    """
    msi = _library()
    msi.MsiSetInternalUI.argtypes = (wintypes.UINT, ctypes.c_void_p)
    msi.MsiSetInternalUI.restype = wintypes.UINT
    msi.MsiOpenPackageExW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(MSIHANDLE))
    msi.MsiOpenPackageExW.restype = wintypes.UINT
    msi.MsiDoActionW.argtypes = (MSIHANDLE, wintypes.LPCWSTR)
    msi.MsiDoActionW.restype = wintypes.UINT
    msi.MsiSetPropertyW.argtypes = (MSIHANDLE, wintypes.LPCWSTR, wintypes.LPCWSTR)
    msi.MsiSetPropertyW.restype = wintypes.UINT
    INSTALLUILEVEL_NONE = 2
    MSIOPENPACKAGEFLAGS_IGNOREMACHINESTATE = 1
    previous = msi.MsiSetInternalUI(INSTALLUILEVEL_NONE, None)
    try:
        handle = MSIHANDLE()
        _check(msi.MsiOpenPackageExW(os.fspath(path), MSIOPENPACKAGEFLAGS_IGNOREMACHINESTATE,
                                     ctypes.byref(handle)), "open the package")
        with _Handle(handle.value) as session:
            for name, value in properties:
                _check(msi.MsiSetPropertyW(session.value, name, value), f"set {name}")
            for action in actions:
                result = msi.MsiDoActionW(session.value, action)
                if result != 0:
                    raise MsiError(f"Action {action} failed ({result}): {_last_error_text()}")
    finally:
        msi.MsiSetInternalUI(previous, None)


def _last_error_text():
    msi = _library()
    record = msi.MsiGetLastErrorRecord()
    if not record:
        return ""
    try:
        size = wintypes.DWORD(0)
        msi.MsiFormatRecordW(0, record, None, ctypes.byref(size))
        buffer = ctypes.create_unicode_buffer(size.value + 1)
        size = wintypes.DWORD(size.value + 1)
        msi.MsiFormatRecordW(0, record, buffer, ctypes.byref(size))
        return buffer.value
    finally:
        msi.MsiCloseHandle(record)


def _check(result, action):
    if result != ERROR_SUCCESS:
        detail = _last_error_text()
        raise MsiError(f"{action} failed with Windows Installer error {result}"
                       + (f": {detail}" if detail else ""))


class _Handle:
    def __init__(self, value):
        self.value = value

    def close(self):
        if self.value:
            _library().MsiCloseHandle(self.value)
            self.value = 0

    def __enter__(self):
        return self

    def __exit__(self, *exception):
        self.close()


def _record(values):
    msi = _library()
    record = _Handle(msi.MsiCreateRecord(len(values)))
    if not record.value:
        raise MsiError("MsiCreateRecord failed")
    try:
        for index, value in enumerate(values, 1):
            if value is None:
                continue
            if isinstance(value, Stream):
                _check(msi.MsiRecordSetStreamW(record.value, index, value.path), "Reading a stream file")
            elif isinstance(value, bool) or not isinstance(value, (int, str)):
                raise MsiError(f"Unsupported field value: {value!r}")
            elif isinstance(value, int):
                _check(msi.MsiRecordSetInteger(record.value, index, value), "Setting an integer field")
            else:
                _check(msi.MsiRecordSetStringW(record.value, index, value), "Setting a string field")
    except BaseException:
        record.close()
        raise
    return record


def _string(record, index):
    msi = _library()
    size = wintypes.DWORD(0)
    empty = ctypes.create_unicode_buffer(1)
    result = msi.MsiRecordGetStringW(record, index, empty, ctypes.byref(size))
    if result not in (ERROR_SUCCESS, ERROR_MORE_DATA):
        _check(result, "Reading a string field")
    buffer = ctypes.create_unicode_buffer(size.value + 1)
    size = wintypes.DWORD(size.value + 1)
    _check(msi.MsiRecordGetStringW(record, index, buffer, ctypes.byref(size)), "Reading a string field")
    return buffer.value


class Database:
    """An MSI database opened read-only or created anew (then committed explicitly)."""

    def __init__(self, path, *, create=False, update=False):
        msi = _library()
        handle = MSIHANDLE(0)
        mode = MSIDBOPEN_CREATE if create else MSIDBOPEN_TRANSACT if update else MSIDBOPEN_READONLY
        _check(msi.MsiOpenDatabaseW(os.fspath(path), mode, ctypes.byref(handle)),
               f"Opening {path}")
        self._handle = _Handle(handle.value)

    def close(self):
        self._handle.close()

    def __enter__(self):
        return self

    def __exit__(self, *exception):
        self.close()

    def _view(self, query):
        handle = MSIHANDLE(0)
        _check(_library().MsiDatabaseOpenViewW(self._handle.value, query, ctypes.byref(handle)),
               f"Preparing {query!r}")
        return _Handle(handle.value)

    def execute(self, query, values=()):
        msi = _library()
        with self._view(query) as view:
            if values:
                with _record(values) as record:
                    _check(msi.MsiViewExecute(view.value, record.value), f"Running {query!r}")
            else:
                _check(msi.MsiViewExecute(view.value, 0), f"Running {query!r}")
            msi.MsiViewClose(view.value)

    def insert(self, table, rows):
        """Inserts rows whose values follow the table's column order."""
        msi = _library()
        with self._view(f"SELECT * FROM `{table}`") as view:
            _check(msi.MsiViewExecute(view.value, 0), f"Opening table {table}")
            for row in rows:
                with _record(row) as record:
                    _check(msi.MsiViewModify(view.value, MSIMODIFY_INSERT, record.value),
                           f"Inserting {row!r} into {table}")
            msi.MsiViewClose(view.value)

    def rows(self, query):
        """Every row as a tuple: int or None for integer columns, str or None otherwise.

        Stream (binary) columns are returned as their size in bytes."""
        msi = _library()
        result = []
        with self._view(query) as view:
            _check(msi.MsiViewExecute(view.value, 0), f"Running {query!r}")
            types_handle = MSIHANDLE(0)
            _check(msi.MsiViewGetColumnInfo(view.value, MSICOLINFO_TYPES, ctypes.byref(types_handle)),
                   "Reading column types")
            with _Handle(types_handle.value) as types_record:
                count = msi.MsiRecordGetFieldCount(types_record.value)
                kinds = [_string(types_record.value, index)[:1].lower() for index in range(1, count + 1)]
            while True:
                handle = MSIHANDLE(0)
                fetched = msi.MsiViewFetch(view.value, ctypes.byref(handle))
                if fetched == ERROR_NO_MORE_ITEMS:
                    break
                _check(fetched, f"Reading {query!r}")
                with _Handle(handle.value) as record:
                    row = []
                    for index, kind in enumerate(kinds, 1):
                        if msi.MsiRecordIsNull(record.value, index):
                            row.append(None)
                        elif kind == "i":
                            row.append(msi.MsiRecordGetInteger(record.value, index))
                        elif kind in ("v", "o"):
                            row.append(len(self._read(record.value, index)))
                        else:
                            row.append(_string(record.value, index))
                    result.append(tuple(row))
            msi.MsiViewClose(view.value)
        return result

    @staticmethod
    def _read(record, index, sink=None):
        msi = _library()
        chunks = []
        buffer = ctypes.create_string_buffer(1 << 20)
        while True:
            size = wintypes.DWORD(len(buffer))
            _check(msi.MsiRecordReadStream(record, index, buffer, ctypes.byref(size)), "Reading a stream")
            if not size.value:
                break
            if sink is None:
                chunks.append(buffer.raw[:size.value])
            else:
                sink.write(buffer.raw[:size.value])
        return b"".join(chunks)

    def save_stream(self, query, destination):
        """Writes the single stream that `query` selects to a new file."""
        msi = _library()
        with self._view(query) as view:
            _check(msi.MsiViewExecute(view.value, 0), f"Running {query!r}")
            handle = MSIHANDLE(0)
            _check(msi.MsiViewFetch(view.value, ctypes.byref(handle)), f"Reading {query!r}")
            with _Handle(handle.value) as record, open(destination, "xb") as sink:
                self._read(record.value, 1, sink)
            msi.MsiViewClose(view.value)

    def set_summary(self, properties):
        msi = _library()
        handle = MSIHANDLE(0)
        _check(msi.MsiGetSummaryInformationW(self._handle.value, None, len(properties) + 1,
                                             ctypes.byref(handle)), "Opening the summary information")
        with _Handle(handle.value) as summary:
            for pid, value in properties.items():
                kind = SUMMARY_TYPES[pid]
                if kind == VT_LPSTR:
                    _check(msi.MsiSummaryInfoSetPropertyW(summary.value, pid, kind, 0, None, value),
                           f"Setting summary property {pid}")
                else:
                    _check(msi.MsiSummaryInfoSetPropertyW(summary.value, pid, kind, value, None, None),
                           f"Setting summary property {pid}")
            _check(msi.MsiSummaryInfoPersist(summary.value), "Saving the summary information")

    def summary(self):
        msi = _library()
        handle = MSIHANDLE(0)
        _check(msi.MsiGetSummaryInformationW(self._handle.value, None, 0, ctypes.byref(handle)),
               "Opening the summary information")
        values = {}
        with _Handle(handle.value) as summary:
            for pid in SUMMARY_TYPES:
                kind = wintypes.UINT(0)
                number = ctypes.c_int(0)
                size = wintypes.DWORD(0)
                empty = ctypes.create_unicode_buffer(1)
                result = msi.MsiSummaryInfoGetPropertyW(summary.value, pid, ctypes.byref(kind),
                                                        ctypes.byref(number), None, empty,
                                                        ctypes.byref(size))
                if result == ERROR_MORE_DATA:
                    buffer = ctypes.create_unicode_buffer(size.value + 1)
                    size = wintypes.DWORD(size.value + 1)
                    result = msi.MsiSummaryInfoGetPropertyW(summary.value, pid, ctypes.byref(kind),
                                                            ctypes.byref(number), None, buffer,
                                                            ctypes.byref(size))
                    empty = buffer
                _check(result, f"Reading summary property {pid}")
                if kind.value == VT_LPSTR:
                    values[pid] = empty.value
                elif kind.value in (VT_I2, VT_I4):
                    values[pid] = number.value
        return values

    def commit(self):
        _check(_library().MsiDatabaseCommit(self._handle.value), "Committing the database")
