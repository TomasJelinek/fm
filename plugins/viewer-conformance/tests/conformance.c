#include <stdio.h>
#include <string.h>
#include "fixture.h"

#define CHECK(value) do { if (!(value)) { \
    fprintf(stderr, "Conformance failure at line %d: %s\n", __LINE__, #value); exit(1); \
} } while (0)

static MidnightPluginApi plugin;
static MidnightWork work[128];
static uint32_t work_head, work_count, leases, notifications;
static bool reject_work, pending_read;
static MidnightRequest read_request;
static const uint8_t content[] = {0, 127, 128, 255};

static MidnightByteSlice bytes(const uint8_t *data, uint32_t len) {
    MidnightByteSlice slice = {data, len, 0};
    return slice;
}

static MidnightStatus schedule(uint64_t context, const MidnightWork *job) {
    CHECK(context == 1 && job->run && job->drop && job->request_id != 0);
    if (reject_work || work_count == 128) return MidnightQUEUE_FULL;
    work[(work_head + work_count++) % 128] = *job;
    return MidnightOK;
}
static uint32_t cancelled(uint64_t context, uint64_t id) { (void)context; (void)id; return 0; }
static void notify(uint64_t context) { CHECK(context == 1); ++notifications; }
static void diagnostic(uint64_t context, uint32_t level, MidnightByteSlice message) {
    (void)context; (void)level; (void)message;
}
static MidnightStatus retain(uint64_t context, uint64_t id) {
    CHECK(context == 1 && id == 7); ++leases; return MidnightOK;
}
static MidnightStatus release(uint64_t context, uint64_t id) {
    CHECK(context == 1 && id == 7 && leases > 0); --leases; return MidnightOK;
}
static MidnightStatus service(uint64_t context, const MidnightRequest *request) {
    static const uint8_t expected[] = {0xa4, 0, 7, 1, 9, 2, 0x1b, 0, 0, 0, 4, 0, 0, 0, 0, 3, 4};
    CHECK(context == 1 && !pending_read && request->kind == MidnightSERVICE_READ_AT);
    CHECK(request->payload.len == sizeof(expected));
    CHECK(memcmp(request->payload.data, expected, sizeof(expected)) == 0);
    read_request = *request;
    read_request.payload = bytes(NULL, 0); /* Never retain the borrowed payload pointer. */
    pending_read = true;
    return MidnightOK;
}

static uint64_t create(void) {
    CHECK(work_count == 0 && leases == 0 && !pending_read);
    MidnightHostApi host = {0};
    host.header = (MidnightHeader){sizeof(host), 1, 0, MidnightKIND_HOST, 0, 0};
    host.context = 1;
    host.schedule_work = schedule; host.cancelled = cancelled; host.submit_service = service;
    host.notify_ready = notify; host.retain_buffer = retain; host.release_buffer = release;
    host.diagnostic = diagnostic;
    uint64_t instance = 0;
    CHECK(plugin.create(&host, bytes(NULL, 0), &instance) == MidnightOK && instance != 0);
    return instance;
}

static MidnightStatus submit(uint64_t instance, uint8_t scenario) {
    uint8_t payload[] = {0xa4, 0, scenario, 1, 0xf4, 2, 4, 3, 1};
    MidnightRequest request = {0};
    request.id = 19; request.session = 3; request.generation = 8;
    request.kind = MidnightREQUEST_VIEWPORT;
    request.payload = bytes(payload, sizeof(payload));
    return plugin.submit(instance, &request);
}

static bool run_one(void) {
    if (!work_count) return false;
    MidnightWork job = work[work_head];
    work_head = (work_head + 1) % 128;
    --work_count;
    job.run(job.context);
    job.drop(job.context);
    return true;
}

static void run_all(void) {
    uint32_t calls = 0;
    while (run_one()) CHECK(++calls < 4096);
}

static bool drain(uint64_t instance, MidnightEvent *event) {
    uint32_t count = 99;
    CHECK(plugin.drain(instance, 1, event, &count) == MidnightOK);
    CHECK(count <= 1);
    return count == 1;
}

static void release_event(uint64_t instance, MidnightEvent event) {
    if (event.content.id) CHECK(plugin.release_buffer(instance, event.content.id) == MidnightOK);
    CHECK(event.payload.id != 0);
    CHECK(plugin.release_buffer(instance, event.payload.id) == MidnightOK);
    CHECK(plugin.release_buffer(instance, event.payload.id) == MidnightUNKNOWN_HANDLE);
}

static void terminal(uint64_t instance, uint32_t kind) {
    MidnightEvent event;
    CHECK(drain(instance, &event) && event.kind == kind && event.request_id == 19);
    CHECK(event.session == 3 && event.generation == 8);
    release_event(instance, event);
    CHECK(!drain(instance, &event));
}

static void destroy(uint64_t instance) {
    CHECK(plugin.begin_shutdown(instance) == MidnightOK);
    CHECK(plugin.destroy(instance) == MidnightOK);
    CHECK(plugin.destroy(instance) == MidnightUNKNOWN_HANDLE);
    CHECK(work_count == 0 && leases == 0 && !pending_read);
}

static MidnightStatus reply(uint64_t instance, bool wrong_generation) {
    uint8_t payload[] = {0xa5, 0, 9, 1, 0x1b, 0, 0, 0, 4, 0, 0, 0, 0, 2, 4, 3, 0xf5, 4, 0};
    CHECK(pending_read);
    MidnightRequest request = read_request;
    request.kind = MidnightREQUEST_SERVICE_REPLY;
    request.payload = bytes(payload, sizeof(payload));
    request.content = (MidnightBuffer){bytes(content, sizeof(content)), 7};
    if (wrong_generation) ++request.generation;
    MidnightStatus result = plugin.submit(instance, &request);
    if (result == MidnightOK) pending_read = false;
    return result;
}

static void admission(void) {
    uint64_t instance = create();
    reject_work = true;
    CHECK(submit(instance, 2) == MidnightQUEUE_FULL);
    MidnightEvent event;
    CHECK(!drain(instance, &event) && work_count == 0);
    reject_work = false;
    CHECK(submit(instance, 2) == MidnightOK);
    CHECK(plugin.destroy(instance) == MidnightNOT_QUIESCENT);
    run_all(); terminal(instance, MidnightEVENT_COMPLETED); destroy(instance);
}

static void cancelled_work(void) {
    uint64_t instance = create();
    CHECK(submit(instance, 0) == MidnightOK);
    CHECK(plugin.cancel(instance, 19) == MidnightOK);
    CHECK(plugin.cancel(instance, 19) == MidnightOK);
    terminal(instance, MidnightEVENT_CANCELLED);
    CHECK(plugin.begin_shutdown(instance) == MidnightOK);
    CHECK(plugin.destroy(instance) == MidnightNOT_QUIESCENT);
    run_all(); CHECK(!pending_read); destroy(instance);
}

static void retained_read(void) {
    uint64_t instance = create();
    CHECK(submit(instance, 0) == MidnightOK);
    run_all(); CHECK(pending_read && work_count == 0);
    CHECK(reply(instance, true) == MidnightINVALID && leases == 0);
    CHECK(reply(instance, false) == MidnightOK && leases == 1);
    CHECK(run_one() && leases == 1);
    MidnightEvent event;
    CHECK(drain(instance, &event) && event.kind == MidnightEVENT_BINARY_PAGE);
    CHECK(event.content.bytes.len == sizeof(content));
    CHECK(memcmp(event.content.bytes.data, content, sizeof(content)) == 0);
    release_event(instance, event);
    run_all(); CHECK(leases == 0);
    terminal(instance, MidnightEVENT_COMPLETED); destroy(instance);
}

static void late_reply(void) {
    uint64_t instance = create();
    CHECK(submit(instance, 0) == MidnightOK);
    run_all(); CHECK(pending_read);
    CHECK(plugin.begin_shutdown(instance) == MidnightOK);
    terminal(instance, MidnightEVENT_CANCELLED);
    CHECK(plugin.destroy(instance) == MidnightNOT_QUIESCENT);
    CHECK(reply(instance, false) == MidnightOK && leases == 0 && work_count == 0);
    destroy(instance);
}

static void backpressure(void) {
    uint64_t instance = create();
    CHECK(submit(instance, 1) == MidnightOK);
    run_all(); CHECK(work_count == 0);
    uint32_t progress = 0, completed = 0;
    MidnightEvent event;
    while (drain(instance, &event)) {
        if (event.kind == MidnightEVENT_PROGRESS) ++progress;
        else { CHECK(event.kind == MidnightEVENT_COMPLETED); ++completed; }
        release_event(instance, event); run_all();
        CHECK(progress <= 100 && completed <= 1);
    }
    CHECK(progress == 100 && completed == 1);
    destroy(instance);
}

static void panic_boundary(void) {
    uint64_t instance = create();
    CHECK(submit(instance, 3) == MidnightOK);
    run_all(); terminal(instance, MidnightEVENT_FAILED); destroy(instance);
}

int conformance(MidnightEntry entry) {
    MidnightPluginApi untouched;
    memset(&untouched, 0xa5, sizeof(untouched));
    MidnightPluginApi short_table = untouched;
    CHECK(entry(&short_table, sizeof(MidnightHeader)) == MidnightINVALID);
    CHECK(memcmp(&short_table, &untouched, sizeof(untouched)) == 0);
    CHECK(entry(&plugin, sizeof(plugin)) == MidnightOK);
    CHECK(plugin.header.abi_major == 1 && plugin.header.kind == MidnightKIND_VIEWER);
    admission(); cancelled_work(); retained_read(); late_reply(); backpressure(); panic_boundary();
    CHECK(notifications != 0);
    puts("PASS: 7 exported plugin ABI scenarios, no retained jobs/services/buffers");
    return 0;
}
