#ifndef MIDNIGHT_FILESYSTEM_H
#define MIDNIGHT_FILESYSTEM_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define ME_FS_ABI_MAJOR 1
#define ME_FS_ABI_MINOR 1
#define ME_FS_KIND 2
#define ME_FS_CAP_BROWSE UINT64_C(1)
#define ME_FS_CAP_READ UINT64_C(2)
#define ME_FS_CAP_STAT UINT64_C(4)
#define ME_FS_CAP_VOLUMES UINT64_C(8)
#define ME_FS_CAP_STORAGE UINT64_C(16)
#define ME_FS_CAP_NATIVE_ADDRESS UINT64_C(32)
#define ME_FS_CAP_VOLUME_DETAILS UINT64_C(64)

typedef uint32_t me_fs_status;
enum { ME_FS_OK=0, ME_FS_INVALID=1, ME_FS_INCOMPATIBLE=2, ME_FS_BUSY=3,
       ME_FS_UNSUPPORTED=4, ME_FS_RESOURCE_LIMIT=5, ME_FS_UNKNOWN_HANDLE=6,
       ME_FS_SHUTTING_DOWN=7, ME_FS_PANICKED=8 };
enum { ME_FS_RESOLVE=1, ME_FS_STAT=2, ME_FS_LIST_BEGIN=3, ME_FS_LIST_NEXT=4,
       ME_FS_CLOSE_LIST=5, ME_FS_PARENT=6, ME_FS_OPEN_READ=7,
       ME_FS_VALIDATE_READ=8, ME_FS_READ_AT=9, ME_FS_CLOSE_READ=10,
       ME_FS_CLOSE_MOUNT=11, ME_FS_VOLUMES=12, ME_FS_STORAGE_KIND=13,
       ME_FS_SPACE_INFO=14, ME_FS_VOLUME_LABEL=15, ME_FS_VOLUME_INFORMATION=16 };
typedef struct { const uint8_t *data; uint64_t len; } me_fs_slice;
typedef struct { uint64_t id; me_fs_slice bytes; } me_fs_buffer;
typedef struct {
    uint32_t struct_size;
    uint16_t abi_major, abi_minor;
    uint32_t kind;
    uint16_t minimum_host_minor, reserved;
} me_fs_header;
typedef struct {
    uint64_t context;
    uint32_t (*cancelled)(uint64_t context);
} me_fs_cancellation;
typedef struct {
    uint32_t struct_size, operation;
    me_fs_slice payload;
    me_fs_cancellation cancel;
} me_fs_call;
typedef struct {
    uint32_t struct_size, reserved;
    uint64_t request_bytes, reply_bytes, retained_buffer_bytes;
    uint32_t max_buffers, max_calls;
} me_fs_config;
typedef struct {
    me_fs_header header;
    uint64_t capabilities;
    me_fs_slice id, version;
    me_fs_status (*create)(const me_fs_config *, uint64_t *instance);
    me_fs_status (*call)(uint64_t instance, const me_fs_call *, me_fs_buffer *);
    me_fs_status (*release_buffer)(uint64_t instance, uint64_t buffer);
    me_fs_status (*destroy)(uint64_t instance);
} me_fs_api;
typedef me_fs_status (*me_fs_entry)(me_fs_api *, uint32_t caller_capacity);
me_fs_status midnight_filesystem_entry_v1(me_fs_api *, uint32_t caller_capacity);

#ifdef __cplusplus
}
#define ME_FS_ASSERT static_assert
#else
#define ME_FS_ASSERT _Static_assert
#endif
ME_FS_ASSERT(sizeof(void *)==8, "filesystem ABI v1 currently supports 64-bit targets");
ME_FS_ASSERT(sizeof(me_fs_slice)==16, "slice layout");
ME_FS_ASSERT(sizeof(me_fs_buffer)==24, "buffer layout");
ME_FS_ASSERT(sizeof(me_fs_header)==16, "header layout");
ME_FS_ASSERT(sizeof(me_fs_cancellation)==16, "cancellation layout");
ME_FS_ASSERT(sizeof(me_fs_call)==40, "call layout");
ME_FS_ASSERT(sizeof(me_fs_config)==40, "config layout");
ME_FS_ASSERT(sizeof(me_fs_api)==88, "descriptor layout");
ME_FS_ASSERT(offsetof(me_fs_api,create)==56, "descriptor callback offset");
#undef ME_FS_ASSERT
#endif
