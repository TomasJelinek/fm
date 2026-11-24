# Linux Network sign-in, 24 November 2026

Explicit SMB activation can now request credentials in the app. The retry uses
the same provider address and leaves native folder validation to the existing
handoff. The staged Linux network plugin advertises ABI 1.5 and `CAP_SIGN_IN`.

| Boundary | Linux result |
| --- | --- |
| Network presenter | 25 passed |
| Network UI integration with the software test platform | 13 passed, including password masking, clearing and dismissal |
| Archive password UI integration | 9 passed |
| Engine Network Neighborhood | 21 passed |
| Native filesystem host library | 157 passed, 3 opt-in tests ignored |
| Filesystem API and SDK workspace | 117 passed, 1 ignored |
| Network provider library and integration tests | 59 passed, 1 opt-in test ignored |
| Release packaging and dependency pins | 63 passed, 12 platform or tool skips |

The provider checks use `GIO_USE_VFS=local`. They include a real GIO password
signal, which verifies the supplied fields, refusal of a second response, and
the no-save setting. The engine and UI fixtures verify a rejected sign-in,
generation checks, and handoff to the original target. The additional host
test confirms that a changed resolved address receives no credentials and
its mount lease is closed.

The sandbox prevented loopback socket creation and maps host root-owned
directories to another UID. Live SMB sign-in and the optional installed
host/provider metadata smoke test could not be verified in this environment.
The shared library built successfully, and a direct check of its exported
descriptor reported ABI 1.5 with sign-in support.
