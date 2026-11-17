#ifndef MIDNIGHT_FILESYSTEM_H
#define MIDNIGHT_FILESYSTEM_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define ME_FS_ABI_MAJOR 1
#define ME_FS_ABI_MINOR 3
#define ME_FS_KIND 2
#define ME_FS_CAP_BROWSE UINT64_C(1)
#define ME_FS_CAP_READ UINT64_C(2)
#define ME_FS_CAP_STAT UINT64_C(4)
#define ME_FS_CAP_VOLUMES UINT64_C(8)
#define ME_FS_CAP_STORAGE UINT64_C(16)
#define ME_FS_CAP_NATIVE_ADDRESS UINT64_C(32)
#define ME_FS_CAP_VOLUME_DETAILS UINT64_C(64)
#define ME_FS_CAP_ACTIVATE UINT64_C(128)
#define ME_FS_CAP_REFRESH UINT64_C(256)
#define ME_FS_CAP_RESOLVE_TEXT UINT64_C(512)
#define ME_FS_CAP_PREPARED_RENAME UINT64_C(1024)

typedef uint32_t me_fs_status;
enum { ME_FS_OK=0, ME_FS_INVALID=1, ME_FS_INCOMPATIBLE=2, ME_FS_BUSY=3,
       ME_FS_UNSUPPORTED=4, ME_FS_RESOURCE_LIMIT=5, ME_FS_UNKNOWN_HANDLE=6,
       ME_FS_SHUTTING_DOWN=7, ME_FS_PANICKED=8 };
enum { ME_FS_RESOLVE=1, ME_FS_STAT=2, ME_FS_LIST_BEGIN=3, ME_FS_LIST_NEXT=4,
       ME_FS_CLOSE_LIST=5, ME_FS_PARENT=6, ME_FS_OPEN_READ=7,
       ME_FS_VALIDATE_READ=8, ME_FS_READ_AT=9, ME_FS_CLOSE_READ=10,
       ME_FS_CLOSE_MOUNT=11, ME_FS_VOLUMES=12, ME_FS_STORAGE_KIND=13,
       ME_FS_SPACE_INFO=14, ME_FS_VOLUME_LABEL=15, ME_FS_VOLUME_INFORMATION=16,
       ME_FS_ACTIVATE=17, ME_FS_REFRESH=18, ME_FS_RESOLVE_TEXT=19,
       ME_FS_PREPARE_RENAME=20, ME_FS_RENAME=21, ME_FS_GET_RENAME_STATE=22,
       ME_FS_CLOSE_RENAME=23 };

/* FSBR payload values; NATIVE_PLUGIN_PROTOCOL.md gives each payload's field order. */
/* u16 response tags */
enum { ME_FS_REPLY_UNIT=1, ME_FS_REPLY_RESOLVED=2, ME_FS_REPLY_NODE=3,
       ME_FS_REPLY_PAGE=4, ME_FS_REPLY_PARENT=5, ME_FS_REPLY_OPENED=6,
       ME_FS_REPLY_CONTENT=7, ME_FS_REPLY_VOLUMES=8, ME_FS_REPLY_STORAGE_KIND=9,
       ME_FS_REPLY_SPACE_INFO=10, ME_FS_REPLY_FAILED=11,
       ME_FS_REPLY_VOLUME_LABEL=12, ME_FS_REPLY_VOLUME_INFORMATION=13,
       ME_FS_REPLY_ACTIVATED=14, ME_FS_REPLY_RENAME_PREPARED=15,
       ME_FS_REPLY_RENAME_STATE=16 };
/* u16 failure codes */
enum { ME_FS_ERROR_INVALID=1, ME_FS_ERROR_UNSUPPORTED=2, ME_FS_ERROR_NOT_FOUND=3,
       ME_FS_ERROR_PERMISSION_DENIED=4, ME_FS_ERROR_SOURCE_CHANGED=5,
       ME_FS_ERROR_CANCELLED=6, ME_FS_ERROR_RESOURCE_LIMIT=7, ME_FS_ERROR_IO=8,
       ME_FS_ERROR_STALE_REFERENCE=9, ME_FS_ERROR_BUSY=10,
       ME_FS_ERROR_UNAVAILABLE=11, ME_FS_ERROR_OFFLINE=12,
       ME_FS_ERROR_AUTHENTICATION_REQUIRED=13, ME_FS_ERROR_MOUNT_REQUIRED=14,
       ME_FS_ERROR_TIMED_OUT=15 };
/* u8 node kinds, address and name encodings, storage kinds and compression */
enum { ME_FS_NODE_DIRECTORY=1, ME_FS_NODE_FILE=2, ME_FS_NODE_DIRECTORY_LINK=3,
       ME_FS_NODE_SYMLINK=4, ME_FS_NODE_OTHER=5 };
enum { ME_FS_ADDRESS_OPAQUE=0, ME_FS_ADDRESS_WINDOWS_UTF16LE=1,
       ME_FS_ADDRESS_UNIX_BYTES=2 };
enum { ME_FS_NAME_OPAQUE=0, ME_FS_NAME_UTF8=1, ME_FS_NAME_WINDOWS_UTF16LE=2,
       ME_FS_NAME_UNIX_BYTES=3 };
enum { ME_FS_STORAGE_KIND_UNKNOWN=0, ME_FS_STORAGE_KIND_LOCAL_SSD=1,
       ME_FS_STORAGE_KIND_ROTATIONAL=2, ME_FS_STORAGE_KIND_NETWORK=3 };
enum { ME_FS_COMPRESSION_UNKNOWN=0, ME_FS_COMPRESSION_UNCOMPRESSED=1,
       ME_FS_COMPRESSION_COMPRESSED=2 };
/* u16 metadata presence bits; present fields follow in bit order */
enum { ME_FS_METADATA_SIZE=1, ME_FS_METADATA_MODIFIED=2,
       ME_FS_METADATA_NATIVE_ATTRIBUTES=4, ME_FS_METADATA_UNIX_MODE=8 };
/* u8 volume properties (minor 1) */
enum { ME_FS_PROPERTY_CASE_PRESERVED_NAMES=1,
       ME_FS_PROPERTY_CASE_SENSITIVE_LOOKUP=2, ME_FS_PROPERTY_UNICODE_NAMES=3,
       ME_FS_PROPERTY_PERSISTENT_ACLS=4, ME_FS_PROPERTY_FILE_COMPRESSION=5,
       ME_FS_PROPERTY_SPARSE_FILES=6, ME_FS_PROPERTY_NAMED_STREAMS=7,
       ME_FS_PROPERTY_HARD_LINKS=8, ME_FS_PROPERTY_REPARSE_POINTS=9,
       ME_FS_PROPERTY_ENCRYPTION=10, ME_FS_PROPERTY_QUOTAS=11,
       ME_FS_PROPERTY_NO_EXECUTE=12, ME_FS_PROPERTY_NO_SET_ID=13,
       ME_FS_PROPERTY_NO_DEVICE_FILES=14 };
/* u8 activation target kinds (minor 2) */
enum { ME_FS_ACTIVATED_WITHIN_PROVIDER=1, ME_FS_ACTIVATED_NATIVE=2 };
/* u8 Rename policy, outcome, source location and recovery role (minor 3) */
enum { ME_FS_POLICY_NO_REPLACE=0, ME_FS_POLICY_RECOVERABLE_CURRENT_ENTRY=1 };
enum { ME_FS_OUTCOME_READY=0, ME_FS_OUTCOME_EXECUTING=1, ME_FS_OUTCOME_CONFLICT=2,
       ME_FS_OUTCOME_UNCHANGED=3, ME_FS_OUTCOME_RENAMED=4,
       ME_FS_OUTCOME_RECOVERY_REQUIRED=5 };
enum { ME_FS_SOURCE_ORIGINAL_NAME=0, ME_FS_SOURCE_DESTINATION=1,
       ME_FS_SOURCE_RECOVERY=2, ME_FS_SOURCE_UNKNOWN=3 };
enum { ME_FS_RECOVERY_SOURCE=0, ME_FS_RECOVERY_DISPLACED_TARGET=1,
       ME_FS_RECOVERY_EMPTY_STAGE=2 };

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
