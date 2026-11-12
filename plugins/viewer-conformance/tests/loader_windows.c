#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <string.h>
#include "fixture.h"

int wmain(int argc, wchar_t **argv) {
    if (argc != 2) { fputs("Expected absolute plugin DLL path\n", stderr); return 2; }
    HMODULE library = LoadLibraryExW(argv[1], NULL,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!library) { fprintf(stderr, "LoadLibraryExW failed: %lu\n", GetLastError()); return 2; }
    FARPROC address = GetProcAddress(library, "midnight_plugin_entry_v1");
    if (!address) { FreeLibrary(library); return 2; }
    MidnightEntry entry;
    _Static_assert(sizeof(entry) == sizeof(address), "function pointer representation");
    memcpy(&entry, &address, sizeof(entry));
    int result = conformance(entry);
    FreeLibrary(library);
    return result;
}
