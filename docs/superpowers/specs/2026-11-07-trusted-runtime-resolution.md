# Trusted plugin runtime for shared Windows builds

The user's launch path is `_build/release/me.exe`. A desktop shortcut to another
copy does not fix that launch path. Reproduced with the exact release and isolated
smoke settings: startup exits 1 because `thumbnailers/` inherits write access for
other accounts. The inherited ACL is real, not a false positive.

Keep the current plugin trust policy. Do not modify permissions on the checkout,
its ancestors or the drive. Do not copy executable plugins automatically at app
startup. An explicitly deployed private installation may supply the plugins for
an identical host executable in the shared checkout.

## Resolution contract

All installed plugin entry points use one backend-only resolver. A directly
trusted executable retains adjacent discovery, including deliberately absent
optional packages. For a shared executable, compute its SHA-256 and consult a
protected per-user registration keyed by that exact digest. Cache the resolved
root for the process; existing discovery and load-time checks still validate the
actual packages and executable code.

Windows registrations live in `%USERPROFILE%/.midnight-explorer/plugin-runtimes/`.
The authority directory is separate from AppData preferences/cache, whose existing
permissions may allow other identities to write. The profile ancestors and new or
existing registration directories must satisfy the same trust policy.
The filename is `<lowercase sha256>.toml`. Strict schema:

```toml
schema_version = 1
sha256 = "64 lowercase hexadecimal digits"
executable_utf16 = [67, 58, 92]
```

The UTF-16 array is an absolute native path, never a display string. The example
only illustrates the encoding. Reject NULs, malformed schema, redirects/reparse
points, unsafe owners/ACLs and target executable digest mismatches. Check original
ancestors before canonicalization. Read registration and executable hashes through
handles denying concurrent write/delete. Bound a registration to 1 MiB and host
hashing to 512 MiB; these are resource budgets, not general path-length limits.

Only the deployment helper publishes registrations, after copy/readback hashes
and private ACL validation succeed. Registration uses an explicit switch so test
deployments never edit the real profile; tests supply their own protected registry
directory. Publish by an exclusive temporary file and atomic replacement within
the protected directory. Normal Windows build helpers deploy and register the
completed bundle. Adding/removing plugins in a shared checkout requires another
deployment; no untrusted package is imported by the running app.

If a shared standalone executable has no plugin directories and no registration,
optional functionality remains absent without inventing a trust exception for
later-added packages. Missing/invalid registrations cannot authorize code loading.
Built-in text comparison remains available when optional image providers cannot
be resolved. Linux keeps its existing executable-relative discovery; platform
implementations remain separate.

## Verification

Test matching builds, stale hashes, unsafe and redirected registrations/targets,
malformed/oversized metadata, native Unicode paths and trusted portable precedence.
Run the actual release from the reported shared path and prove startup no longer
reports the trust error after registration. Exercise real text/binary, highlighted
source, Markdown, image and PDF viewers and their controls in an isolated native
UI harness. All QA settings stay in memory; compare the user's preferences hash
before/after. Preserve independent plugin projects and their own tests.
