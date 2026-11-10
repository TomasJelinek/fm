# Third-party notices

This independent Network Neighborhood package builds only against the public
project-owned filesystem API/SDK and the pinned libraries below. The project's
own license is unchanged. No copied Salamander implementation or resources are
included. These selected open-source licenses permit development, application use
and distribution without mandatory fees, subscriptions or royalties; their notice
and redistribution obligations still apply.

| Target | Library | Enabled features | Selected license |
| --- | --- | --- | --- |
| Windows | windows-sys 0.61.2 | no defaults; Win32_Foundation, Win32_NetworkManagement_WNet (with parent Win32/Win32_NetworkManagement) | MIT |
| Windows | windows-link 0.2.1 | no features | MIT |
| Linux | libloading 0.8.9 | no optional features | ISC |
| Linux | cfg-if 1.0.5 | no optional features | MIT |

Exact registry source manifests and retained license files were inspected.
Cargo.lock records all checksums. The selected target graph excludes libloading
on Windows and windows-sys/windows-link on Linux. MIT is selected from the
MIT OR Apache-2.0 alternatives. Full selected crate notices follow below.

## Optional system runtime on Linux

The adapter lazily loads the system's libgio-2.0.so.0, libgobject-2.0.so.0 and
libglib-2.0.so.0. GLib/GIO is maintained by the GLib project and its contributors:
https://gitlab.gnome.org/GNOME/glib . The inspected Ubuntu 24.04 runtime was
libglib2.0-0t64 2.80.0-6ubuntu3.9. Its package copyright identifies the main
libraries as LGPL-2.1-or-later, with additional per-file notices; the authoritative
installed notices are /usr/share/doc/libglib2.0-0t64/copyright.

Network discovery and SMB/native handoff may use separately installed GNOME GVfs,
its selected backends and FUSE bridge: https://gitlab.gnome.org/GNOME/gvfs . The
inspected upstream source tag was 1.54.4; COPYING contains the GNU Library General
Public License version 2, and the inspected network/SMB/client source headers
permit version 2 or later. Do not describe the entire GVfs distribution as having
one uniform later license; follow its actual package's complete copyright and
source notices. Source: https://gitlab.gnome.org/GNOME/gvfs/-/tree/1.54.4 .

No GLib, GVfs, Samba, FUSE, or system backend binaries are bundled here. These
runtime components are not Cargo dependencies. Their distribution's packages
supply their corresponding notices and source obligations. Bundling any native
runtime later requires a separate pinned dependency/feature and source-distribution
audit, including backend dependencies; the current crate notice inventory does
not qualify such a bundle. System GVfs/backends/FUSE were absent in the Linux
fixture environment, so live SMB/FUSE behavior remains unqualified.

## libloading 0.8.9 — ISC
Copyright © 2015, Simonas Kazlauskas

Permission to use, copy, modify, and/or distribute this software for any purpose with or without
fee is hereby granted, provided that the above copyright notice and this permission notice appear
in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.

## cfg-if 1.0.5 — MIT

Copyright (c) 2014 Alex Crichton

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

## windows-sys 0.61.2 and windows-link 0.2.1 — MIT

    MIT License

    Copyright (c) Microsoft Corporation.

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE
