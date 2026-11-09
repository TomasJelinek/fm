# Host content reads

`midnight-plugin-host::CapturedFile` retains the original content length and
revision. A host-owned `ContentSource` supplies bytes without exposing a native
path or Rust object to a plugin. These Rust interfaces are internal to the host;
the public plugin reader ABI is unchanged.

Both direct broker reads and retained plugin readers use the same boundary:

- Reject zero-sized requests, requests above 256 KiB and offset overflow.
- Clamp requests to the original captured length.
- Validate the source before returning empty bytes at or beyond EOF.
- Pass the original length/revision to `ContentSource::read_checked`.
- Reject responses whose byte count differs from the clamped request.

The provided `read_checked` implementation validates the live source and compares
its length/revision before and after `read_at`. Generic providers need only
implement the existing read and validation methods. A detectable mutation during
the read is an error, and its returned bytes are not published.

A trusted host adapter may override `read_checked` only if its underlying read
already enforces equivalent identity/revision validation before and after reading.
It must check the caller's original expected metadata, never replace it with newly
opened metadata. The outer bounds and returned-count checks still apply. The
native filesystem adapter uses this to avoid additional validation round trips
around the native SDK's already validated `ReadAt` operation. An evicted native
reader must match its original captured identity/revision when reopened.

`CapturedFile::revalidate` always performs live validation. Empty reads also keep
that check; they cannot be used to turn a changed source into a successful result.
Validation is the documented LiveValidated contract, not a filesystem snapshot
or a guarantee against changes that the platform's metadata cannot detect.
Capture, reading, validation and disposal remain backend work.

Regression tests are in `midnight-plugin-host/src/checked_read_tests.rs`, with
native adapter/cache tests in the native filesystem host. They cover both direct
and retained readers, original metadata forwarding, bounds/EOF/counts, and
detectable changes during a generic read.
