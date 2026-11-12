#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include "fixture.h"

int main(int argc, char **argv) {
    if (argc != 2) { fputs("Expected absolute plugin SO path\n", stderr); return 2; }
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!library) { fprintf(stderr, "dlopen failed: %s\n", dlerror()); return 2; }
    void *address = dlsym(library, "midnight_plugin_entry_v1");
    if (!address) { dlclose(library); return 2; }
    MidnightEntry entry;
    _Static_assert(sizeof(entry) == sizeof(address), "POSIX function pointer representation");
    memcpy(&entry, &address, sizeof(entry));
    int result = conformance(entry);
    dlclose(library);
    return result;
}
