# Disk operations

**Options → General → Disk operations → Disk I/O concurrency** sets how many
participating background disk operations may run at the same time, for example
folder-size calculations and copies.

| Choice | Behavior |
| --- | --- |
| Automatic (4), the default | Up to four at a time. |
| Manual: 1 to Manual: 32 | That many at most; a job may use fewer, or keep a stricter limit of its own. |

The new value takes effect when **OK** saves it successfully. **Cancel** and a
failed save leave the active limit unchanged.

## What the limit covers

- **Folder sizes, checksums, file comparisons, previews, the image and video viewers
  and other bulk reads.** Their reads run in parallel only on storage proven to be a
  local SSD. On a rotational
  disk, on network or unknown storage, and on a local disk whose media type cannot be
  proven (a virtual machine's disk, a USB disk), each read runs alone: no other
  participating disk work runs beside it.
- **Copy and Move.** Up to that many transfers do their disk work at the same time
  when they use different files and folders; each transfer copies one file at a time.
  A transfer that is paused, or waiting for your decision or for another transfer,
  holds no slot. A running transfer lets waiting reads go first at its next step, so
  they wait for at most one step of it, never for the whole transfer.
- **Not covered:** Find, folder-tree discovery and the plugins' own I/O keep separate
  budgets. The limit is not a total thread count and does not size the CPU pools
  that decode or hash.

## Changing the limit

- Lowering it lets disk calls already running finish; new calls wait until they fit
  the new limit.
- Raising it does not add workers to a folder-size calculation already running; the
  next calculation uses the higher limit.
- More concurrency can use more memory.

See the [developer contract](developer/DISK_OPERATIONS.md) for scheduling and
persistence details, or return to the [documentation map](README.md).
