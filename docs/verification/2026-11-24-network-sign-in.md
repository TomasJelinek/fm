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

## Remembered sign-ins, 25 November 2026

The dialog asked again for every share or server of a device after a sign-in. Each
activation mounted through GVfs with `G_PASSWORD_SAVE_NEVER`, and only the one retry
after the dialog carried credentials. The provider now remembers a successful sign-in
for its SMB server for the rest of the session ([contract](../developer/network-filesystem.md#platform-behavior-and-tests)).

| Check | Result |
| --- | --- |
| `catalog::sign_ins` with a fixture adapter | 4 passed. A sign-in answers later activations on the same server only; a rejected sign-in is not remembered and a rejected memory is forgotten; a newer sign-in survives the rejection of an older one; at most 32 servers. |
| `linux::tests::a_sign_in_is_scoped_to_its_smb_server_and_port` | Passed. The scope ignores ASCII case; another port or server is another scope; containers and the root have none. |
| GIO password challenge tests on a desktop with GIO | Passed: the supplied fields, refusal of a second response, and the no-save setting. |
| Provider integration tests (`activation`, `catalog`, `concurrency`, `paging`) | 22 passed |

The GIO tests were built in the build image and run on a desktop that has GIO. The
five GIO directory-listing tests did not run there, because their fixture folder is
the build image's path. They do not touch sign-in. Live SMB sign-in against a real
server was not run.
