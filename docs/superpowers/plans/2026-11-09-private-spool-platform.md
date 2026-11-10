# Private Spool Platform Implementation Plan

**Status: resumed by explicit user instruction on 9 November 2026.** The earlier
cancelled attempt left no implementation. The resumed implementation and native
verification are tracked in `_build/archive-streaming-spool/ledger.md`.

> **For agentic workers, only after explicit resumption:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide backend-only bounded append/random-read anonymous storage with native crash cleanup.

**Architecture:** A small shared facade owns length, input validation, sealing and sticky I/O failure state. Separate OS modules reuse the existing private-store root verification/pins, then create an exclusive temporary file with native crash cleanup. The host owns integrity validation and publication. There are no artificial per-file or aggregate disk byte quotas.

**Tech Stack:** Existing Rust std, rustix 1.1.6 and windows-sys 0.61.2; no dependency or feature changes.

**Spec:** `docs/superpowers/specs/2026-11-09-archive-streaming-design.md`, implementation step 2.

## Global Constraints

- No worker-controlled path or exported native handle; root path is host configuration.
- Every append/read is at most 256 KiB; lengths and offsets are checked u64 values.
- No retained content Vec, total-size policy, worker protocol or UI changes.
- Retain existing ancestor and private-owner/permission checks. Never follow a replaced ancestor.
- Windows uses exclusive create, explicit private DACL and delete-on-close; Linux uses O_TMPFILE|O_EXCL in the verified directory and fails explicitly on unsupported filesystems.
- Output under `_build/archive-streaming-spool`, toolchain caches under `.tools`, at most two build jobs.
- Coordinate existing store/lib edits with root; no staging, commits or unrelated edits.

## Review Focus

- Partial native writes poison the object; no partially written object becomes readable.
- Root replacement or symlinks/reparse points cannot redirect spool creation.
- Holding a spool after dropping its factory retains the required directory pins.
- Force-killing a process leaves no content files; ordinary close does likewise.
- Unsupported filesystems and real I/O failures fail explicitly without a memory fallback.

### Task 1: Shared facade and native storage

**Files:** new `crates/midnight-worker-platform/src/spool.rs`, `spool_windows.rs`, `spool_linux.rs`; minimal crate export and private-store helper visibility/accessors; native and shared tests.

**Interfaces:** `PrivateSpoolDirectory::open(&Path) -> io::Result<Self>`, `create(&self) -> io::Result<Spool>`; `Spool::length() -> u64`, `append(&mut self, expected_offset: u64, bytes: &[u8]) -> io::Result<u64>`, `read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize>`, `finish(&mut self) -> io::Result<()>`. Finish flushes and seals; reads require sealing and later writes are rejected. Drop closes the owned file. Finish is an I/O check, not archive validation, publication or a durable cache promise.

- [x] Write observable append/read bounds, independent-object and cleanup tests; run and record missing-primitive failure.
- [x] Implement facade and native storage, reusing audited root checks; observe tests pass.
- [x] Add native symlink/reparse, ancestor replacement, exact crash cleanup and actual OS I/O failure tests; run and fix failures.
- [x] Run existing private-store tests and all new tests natively on Windows and Linux, strict Clippy and formatting.
- [ ] Request independent review and report interface, evidence and supported-filesystem limitations to coordinator.

## Execution record

The resumed implementation passes17 Windows tests and18 native Linux tests,
including existing process containment/private-store tests, with strict Clippy
and formatting checks. The fixture using `/dev/full` proves Linux ENOSPC;
controlled fault injection covers partial writes followed by disk exhaustion and
flush failure without filling user storage. Windows tests exercise actual native
access failures, non-inheritance, reparse refusal, pins and delete-on-close.
Review logs and source hashes are under `_build/archive-streaming-spool`.

### Earlier cancelled attempt

Stopped at the user's request. An initial test scaffold reached the expected
missing-API compile failure and was then removed. No lib exports, private-store
helpers, native implementations or dependency files were modified. The design
is retained only as a deferred note; ARCH-01 remains unimplemented.

Ruling: use the coordinator's existing workspace and reserve only the named files, rather than creating a divergent worktree; concurrent agents own disjoint work. Cost if wrong: integration conflicts, addressed by explicit ownership coordination.

Ruling: Linux O_TMPFILE eliminates the crash window between named create and unlink. Unsupported filesystems fail closed; the host may select a suitable private local root. Approved by coordinator. Cost: spooling unavailable on filesystems without anonymous-file support.

Ruling: keep the ledger here and command logs under `_build/archive-spool-platform`; project output-location rules and coordinator's no-Git instruction supersede generic skill scratch/commit steps.
