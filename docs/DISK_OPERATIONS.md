# Disk operations

General > Disk operations sets the maximum parallel background disk work for
participating bulk operations, such as folder-size calculations.

| Mode | Behavior |
| --- | --- |
| Automatic (default) | Preserve the existing limit of four. |
| Manual, 1–32 | Set an upper bound; individual jobs may use fewer workers or keep stricter limits. |

Changes take effect only after OK saves successfully. Cancel and failed saves
leave the active setting unchanged.

Parallel reads through this setting are allowed only for confirmed local SSD
storage. HDD, network and unknown storage keep their exclusive policy. Copy and
Move jobs share this limit: up to that many transfers do their disk work at the
same time when they touch different files and folders, each copying one file at a
time. A transfer that is paused, or waiting for your decision or for another
transfer, uses no slot, and a running transfer briefly lets waiting folder-size,
checksum, thumbnail and viewer reads go first. Find, tree discovery and
plugin-private I/O keep separate budgets.

Lowering the limit lets disk calls already running finish, while new calls wait
for capacity under the new limit. Raising it does not add workers to a folder-size
calculation already in progress; new calculations can use the higher limit.

More concurrency can use more memory. The value is not a total application-thread
count and does not resize CPU decoding pools. Automatic preserves existing
behavior.

See the [developer contract](developer/DISK_OPERATIONS.md) for scheduling and
persistence details, or return to the [documentation map](README.md).
