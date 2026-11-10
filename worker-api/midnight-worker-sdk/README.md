# Restricted worker startup SDK 0.2

Call `enter` before parsing content. Failure is terminal. The host grants one
captured source through its provider protocol; startup supplies no source paths,
network service or inherited content handle.

Legacy `Limits`, `write_limits`, `read_ready` and the host's default `spawn` keep
the original MWS1/MWR1 contract and the 1–60 second aggregate CPU limit unchanged.
SDK 0.2 additionally accepts an explicit `ResourcePolicy` with
`CpuBudget::Seconds(1..=60)` or `CpuBudget::NoAdditionalLimit`.

NoAdditionalLimit adds no application CPU-time ceiling. It preserves inherited
OS limits and security policy; it is not a promise of unlimited machine resources
and never requests Windows job breakaway or raises a Linux inherited CPU limit.
Memory limits, one process, sandboxing, host idle deadlines and cancellation stay
in force. The host remains responsible for supervising progress and cancellation.

`write_policy` emits MWS2, little-endian memory bytes (`u64`), then a CPU policy tag:
0 for NoAdditionalLimit with no further field, 1 for Seconds followed by `u32`.
The matching `read_policy_ready` requires MWR2. Old helpers continue to emit and
accept only their version 1 acknowledgement. `enter` accepts either version and
acknowledges that exact version only after its OS restriction phase succeeds.
Unknown versions/tags, zero or excessive Seconds values, invalid memory limits
and truncated messages fail before content parsing.

Windows verifies the explicit job's memory/process limits and requested CPU
policy inside a capability-free AppContainer. Linux applies memory and applicable
CPU limits before restricting syscalls and closing non-broker descriptors.
