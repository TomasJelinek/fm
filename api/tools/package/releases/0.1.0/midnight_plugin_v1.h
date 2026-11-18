#ifndef MIDNIGHT_PLUGIN_V1_H
#define MIDNIGHT_PLUGIN_V1_H

/* Generated from src/abi.rs by midnight-api-headergen. Do not edit. */

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

#define MidnightABI_MAJOR 1

#define MidnightABI_MINOR 0

#define MidnightKIND_HOST 0

#define MidnightKIND_VIEWER 1

#define MidnightKIND_FILESYSTEM 2

#define MidnightREQUEST_OPEN 1

#define MidnightREQUEST_VIEWPORT 2

#define MidnightREQUEST_SEARCH 3

#define MidnightREQUEST_CLOSE 4

#define MidnightREQUEST_SERVICE_REPLY 5

/**
 * Optional text/source coordinate lookup. Older plugins may return UNSUPPORTED.
 */
#define MidnightREQUEST_LOCATE 6

#define MidnightEVENT_OPENED 1

#define MidnightEVENT_TEXT_PAGE 2

#define MidnightEVENT_BINARY_PAGE 3

#define MidnightEVENT_DOCUMENT_PAGE 4

#define MidnightEVENT_SEARCH_MATCH 5

#define MidnightEVENT_PROGRESS 6

/**
 * Sent only in response to an accepted REQUEST_LOCATE.
 */
#define MidnightEVENT_TEXT_POSITION 7

#define MidnightEVENT_COMPLETED 100

#define MidnightEVENT_CANCELLED 101

#define MidnightEVENT_FAILED 102

#define MidnightEVENT_QUIESCENT 103

#define MidnightSERVICE_READ_AT 1001

#define MidnightSERVICE_CLOSE_READER 1002

#define MidnightWORK_CPU 1

#define MidnightWORK_IO 2

typedef uint32_t MidnightStatus;

/**
 * This prefix may be read only when the supplied capacity covers all 16 bytes.
 */
typedef struct MidnightHeader {
  uint32_t struct_size;
  uint16_t abi_major;
  uint16_t abi_minor;
  uint32_t kind;
  uint16_t minimum_host_minor;
  uint16_t reserved;
} MidnightHeader;

typedef struct MidnightByteSlice {
  const uint8_t *data;
  uint32_t len;
  uint32_t reserved;
} MidnightByteSlice;

/**
 * The producing instance owns this allocation until `release_buffer(id)` succeeds.
 */
typedef struct MidnightBuffer {
  struct MidnightByteSlice bytes;
  uint64_t id;
} MidnightBuffer;

typedef struct MidnightRequest {
  uint64_t id;
  uint64_t session;
  uint64_t generation;
  uint32_t kind;
  uint32_t flags;
  struct MidnightByteSlice payload;
  /**
   * Optional immutable host buffer. Retain through HostApi before async use.
   */
  struct MidnightBuffer content;
} MidnightRequest;

typedef struct MidnightEvent {
  uint64_t request_id;
  uint64_t session;
  uint64_t generation;
  uint32_t kind;
  uint32_t flags;
  struct MidnightBuffer payload;
  /**
   * Optional raw bytes; released independently of the CBOR payload buffer.
   */
  struct MidnightBuffer content;
} MidnightEvent;

/**
 * Rejected scheduling retains job ownership in the plugin. Accepted jobs receive
 * exactly one drop, whether run is invoked or cancelled before starting.
 */
typedef struct MidnightWork {
  void *context;
  void (*run)(void*);
  void (*drop)(void*);
  uint64_t request_id;
  uint32_t class_;
  uint32_t affinity;
} MidnightWork;

typedef struct MidnightHostApi {
  struct MidnightHeader header;
  uint64_t context;
  MidnightStatus (*schedule_work)(uint64_t, const struct MidnightWork*);
  uint32_t (*cancelled)(uint64_t, uint64_t);
  MidnightStatus (*submit_service)(uint64_t, const struct MidnightRequest*);
  void (*notify_ready)(uint64_t);
  MidnightStatus (*retain_buffer)(uint64_t, uint64_t);
  MidnightStatus (*release_buffer)(uint64_t, uint64_t);
  void (*diagnostic)(uint64_t, uint32_t, struct MidnightByteSlice);
} MidnightHostApi;

typedef struct MidnightPluginApi {
  struct MidnightHeader header;
  /**
   * Immutable UTF-8 metadata valid for the lifetime of the loaded library.
   */
  struct MidnightByteSlice id;
  struct MidnightByteSlice version;
  MidnightStatus (*create)(const struct MidnightHostApi*, struct MidnightByteSlice, uint64_t*);
  MidnightStatus (*submit)(uint64_t, const struct MidnightRequest*);
  MidnightStatus (*cancel)(uint64_t, uint64_t);
  MidnightStatus (*drain)(uint64_t, uint32_t, struct MidnightEvent*, uint32_t*);
  MidnightStatus (*release_buffer)(uint64_t, uint64_t);
  MidnightStatus (*begin_shutdown)(uint64_t);
  MidnightStatus (*destroy)(uint64_t);
} MidnightPluginApi;

/**
 * Exported as `midnight_plugin_entry_v1`. Never writes past capacity.
 */
typedef MidnightStatus (*MidnightEntry)(struct MidnightPluginApi*, uint32_t);

#define MidnightOK 0

#define MidnightINVALID 1

#define MidnightINCOMPATIBLE 2

#define MidnightQUEUE_FULL 3

#define MidnightUNSUPPORTED 4

#define MidnightSHUTTING_DOWN 5

#define MidnightNOT_QUIESCENT 6

#define MidnightPANICKED 7

#define MidnightUNKNOWN_HANDLE 8

#endif  /* MIDNIGHT_PLUGIN_V1_H */
