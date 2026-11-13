# Third-party dependency policy

Confirmed product requirement, 6 November 2026: every third-party library must
allow the intended development, use and distribution without mandatory license
fees, subscriptions or royalties. Prefer maintained open-source libraries.
This covers direct and transitive dependencies in the application, public API/SDK,
independent plugins, tests and build/release tooling.

Prefer compatible MIT, Apache-2.0, BSD, ISC and similarly permissive licenses.
Open-source copyleft is not automatically excluded: its conditions must fit the
way the component is built, modified and distributed. A free download, trial,
source-visible repository or optional paid license alone does not establish this.
For `OR` expressions choose a compatible free branch; `AND` combines obligations.
Check license files and notices in the pinned source, not just a crate's label.

## Dependency changes

1. Establish the needed capability, maintained upstream and smallest useful feature
   set. Reuse a suitable existing dependency before adding another.
2. Inspect the exact version's license, bundled code/data, native libraries and
   enabled optional components, including codec licenses where relevant.
3. Keep independent plugins' dependency locks and tests in their own projects.
   Sharing this Git repository does not permit dependency on private host crates.
4. Regenerate the [direct and complete inventories](THIRD_PARTY.md), and preserve
   required copyright/license/source notices in each distributable artifact.
5. Check maintenance and RustSec advisories for the affected target/features.
   Record unresolved findings; a clean scan is not proof of safety.
6. Record the selected license branch and any concrete distribution obligations
   when a permissive default does not describe the actual component.

The inventory reports upstream declarations, not an automated compatibility verdict.
Missing metadata requires source inspection; the generator refuses an `UNKNOWN`
license. Do not add paid-only or trial-only dependencies. Do not assign or change
the license of Midnight Explorer or its public API as a side effect of this policy.

## Current decisions

**Slint 1.18.1:** the application uses the **Slint Royalty-free License 2.0** for
desktop applications. It requires no paid Slint subscription. Retain Slint notices
and the attribution required by the exact license; the application has About Slint
attribution and public release download pages carry the Slint badge. The license
also limits standalone redistribution, embedded use and exposing Slint APIs to
third-party applications. The custom royalty-free license is not described here
as an OSI open-source license. The upstream GPL-3.0 alternative is open source;
the paid Software License alternative is not selected. See the
[pinned license](https://github.com/slint-ui/slint/blob/v1.18.1/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md)
and [upstream licensing terms](https://slint.dev/terms-and-conditions).

**cbindgen 0.29.4:** MPL-2.0, used by the public API header-generation tool. It is
not linked into the file manager runtime. Preserve covered-source obligations
when modifying/distributing this tool; MPL's file-level scope does not automatically
apply its license to all application files. See the
[Mozilla MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/).

**Native runtimes:** Windows/system graphics libraries remain governed by their
vendors; their use does not make the operating system open source. Linux libraries
retain their distribution's licenses. Rust bindings do not relicense native
libraries. See the separate native/runtime section of the [inventory](THIRD_PARTY.md).

**Required media workers:** standard builds include the independent image/video
thumbnailers. `image` 0.25.10 is MIT OR Apache-2.0. The pinned FFmpeg 9.0.2 shared
runtime uses LGPL-2.1-or-later with GPL, nonfree, version3, networking and external
autodetection disabled. Only the approved VP8/VP9/Theora/FFV1/rawvideo decoders
and Matroska/WebM, AVI and Ogg demuxers are enabled; making the workers required
does not expand that codec set. Matching source, build configuration and notices
must accompany the replaceable shared libraries. `ffmpeg-next` 9.0.0 and
`ffmpeg-sys-next` 9.0.1 use WTFPL; preserve their pinned declarations and the
missing-upstream-license provenance recorded in the inventory. No paid decoder
or license branch is selected. See the [video worker guide](../plugins/midnight-thumbnail-video/README.md).
