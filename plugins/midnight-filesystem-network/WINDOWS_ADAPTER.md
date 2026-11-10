# Windows network adapter

`Windows` implements the shared `Adapter` contract using Unicode MPR calls. It
shares no application code and creates no worker pool. The caller's admission
slot and synchronous cursor ownership remain live until each native call returns.
The shared policy owns cache limits, TTL, paging and complete ancestor addresses.

The root opens `RESOURCE_GLOBALNET` with no parent and lists the installed native
provider containers. It does not eagerly discover every provider's neighborhood.
`RESOURCE_CONTEXT` can wait for unavailable browser services before returning an
unnamed Entire Network wrapper; directly listing that wrapper's providers keeps
the root usable without increasing the request deadline or fabricating devices.
A selected container/server opens one
`RESOURCE_GLOBALNET` enumeration with its exact retained provider/resource
identity. It never recursively opens returned containers. No connection,
credential, mount, Shell, NetServerEnum or NetShareEnum API is called. MPR may
use the caller's existing OS network session; this adapter supplies no credentials
and requests no authentication dialog. See Microsoft's
[WNetOpenEnumW contract](https://learn.microsoft.com/en-us/windows/win32/api/winnetwk/nf-winnetwk-wnetopenenumw).

The versioned adapter key preserves all four NETRESOURCE scalar fields and all
four optional UTF-16 strings, including null versus empty and unpaired surrogates.
Captions use lossy text only for display. `RESOURCE_CONTEXT` leaves usage undefined;
when opening an explicitly selected server/container, the temporary native input
adds the container bit required by WNet. Its stored key remains unchanged.

Manual parsing performs no I/O. A bare server or UNC server stays within the
provider. Explicit share activation returns the unchanged UTF-16 UNC path to the
host's native handoff. Native paths require server and share components; device
namespaces, dot components, credentials in the server name and unsupported URLs
are rejected. Longer paths are preserved subject to the identity resource budget;
there is no MAX_PATH truncation. Non-UNC provider shares report Unsupported.
Ordinary discovery skips print and hidden/system shares; explicitly entered
hidden UNC shares remain available for user activation.

## Native boundary and limits

- Identity admission is 64 KiB. An enumeration buffer starts at 16 KiB and grows
  up to 1 MiB on `ERROR_MORE_DATA`; it is aligned for NETRESOURCEW and fully
  initialized. The prior page is cleared before reuse.
- Successful pages contain at most 128 records. Record headers, every optional
  string pointer, alignment, extent and terminator are checked before copying.
  Supplied string pointers are converted to checked offsets in the owned buffer;
  they are never directly dereferenced. Repeated aliases are charged by actual
  retained vector capacity, so pointer reuse cannot evade the 1 MiB decoded-page
  budget. Converted pending records have a separate 1 MiB budget.
- Only successful calls define records/count. Failed MORE_DATA output and
  NO_MORE_ITEMS buffers are ignored. Growth, malformed counts and progress are
  bounded: at most 128 native page calls and 4096 examined resources per cursor.
  Exceeding a budget is an explicit failure, never silent truncation. These limits
  complement the shared cache/reply policy.
- Cancellation/deadline checks surround native calls and each page/record. MPR
  has no general cancellation facility; a blocked call retains ownership until
  real return. Error or EOF closes a successfully acquired cursor once. Failed
  opens never release an unowned output handle.
- A completed native failure retains its Windows/provider diagnostic even when
  the call outlived the request deadline. Cancellation still takes precedence;
  late successful results and buffer-growth retries remain deadline bounded.
  A failing provider does not make the other root containers unavailable.

The buffer contract follows
[WNetEnumResourceW](https://learn.microsoft.com/en-us/windows/win32/api/winnetwk/nf-winnetwk-wnetenumresourcew)
and [NETRESOURCEW](https://learn.microsoft.com/en-us/windows/win32/api/winnetwk/ns-winnetwk-netresourcew).
Known MPR system errors distinguish missing resources, offline service, unavailable
discovery, denied access, required authentication and timeout. Extended error codes
belong to the provider's namespace, so numeric coincidences are not interpreted as
Windows credential/mount authority. Only their number is retained in an Io
diagnostic; provider-supplied diagnostic text is discarded. See
[WNetGetLastErrorW](https://learn.microsoft.com/en-us/windows/win32/api/winnetwk/nf-winnetwk-wnetgetlasterrorw).

On the development machine, the old context open took about 19.7 seconds and
returned only an unnamed root; direct provider enumeration completed in about
25 milliseconds. Explicit Microsoft Windows Network enumeration still reported
provider error 6118 after about 19.7 seconds. This is an OS discovery limitation,
not evidence that all available devices were found. Other installed providers and
explicitly entered UNC shares remain independent of that browser failure.

All ordinary tests use injected MPR responses and owned buffers. They do not
discover the live corporate network. The adapter-only harness passed 22 Windows
tests, strict all-target Clippy with `-D warnings`, and a release build. Shared
policy/ABI integration, host activation validation and installed-package tests are
separate gates; this report makes no deployment or live discovery claim.

The 2026-11-09 root-timeout correction passes all 58 independent plugin tests
(37 unit and 21 integration), strict all-target Clippy and formatting checks.
Three causal regressions failed before the correction. An opt-in native smoke
through the public ABI returned the four installed provider containers in
4.735 ms; it did not expand those containers or claim successful device discovery.
Evidence is under `_build/qa/network-root-timeout/`. The debug DLL staged for the
application has SHA-256
`2A22B9908D73AEE731D9BE723EB98E0DAC24FD0C1D9822B2146ACE62450C24EA`.
