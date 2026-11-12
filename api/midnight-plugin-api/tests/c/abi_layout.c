#include "midnight_plugin_v1.h"
#include <stdio.h>

#define BEGIN(T) printf(#T " %zu %zu", sizeof(Midnight##T), _Alignof(Midnight##T))
#define FIELD(T, F) printf(" %zu", offsetof(Midnight##T, F))
#define END() puts("")

int main(void) {
    BEGIN(Header); FIELD(Header, struct_size); FIELD(Header, abi_major);
    FIELD(Header, abi_minor); FIELD(Header, kind); FIELD(Header, minimum_host_minor);
    FIELD(Header, reserved); END();
    BEGIN(ByteSlice); FIELD(ByteSlice, data); FIELD(ByteSlice, len); FIELD(ByteSlice, reserved); END();
    BEGIN(Buffer); FIELD(Buffer, bytes); FIELD(Buffer, id); END();
    BEGIN(Request); FIELD(Request, id); FIELD(Request, session); FIELD(Request, generation);
    FIELD(Request, kind); FIELD(Request, flags); FIELD(Request, payload); FIELD(Request, content); END();
    BEGIN(Event); FIELD(Event, request_id); FIELD(Event, session); FIELD(Event, generation);
    FIELD(Event, kind); FIELD(Event, flags); FIELD(Event, payload); FIELD(Event, content); END();
    BEGIN(Work); FIELD(Work, context); FIELD(Work, run); FIELD(Work, drop);
    FIELD(Work, request_id); FIELD(Work, class_); FIELD(Work, affinity); END();
    BEGIN(HostApi); FIELD(HostApi, header); FIELD(HostApi, context); FIELD(HostApi, schedule_work);
    FIELD(HostApi, cancelled); FIELD(HostApi, submit_service); FIELD(HostApi, notify_ready);
    FIELD(HostApi, retain_buffer); FIELD(HostApi, release_buffer); FIELD(HostApi, diagnostic); END();
    BEGIN(PluginApi); FIELD(PluginApi, header); FIELD(PluginApi, id); FIELD(PluginApi, version);
    FIELD(PluginApi, create); FIELD(PluginApi, submit); FIELD(PluginApi, cancel); FIELD(PluginApi, drain);
    FIELD(PluginApi, release_buffer); FIELD(PluginApi, begin_shutdown); FIELD(PluginApi, destroy); END();
    return 0;
}
