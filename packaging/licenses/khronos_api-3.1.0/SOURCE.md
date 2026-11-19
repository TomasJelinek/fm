# khronos_api 3.1.0 license supplement

The exact published crate declares `Apache-2.0` and omits root license
files. The selected free branch is **Apache-2.0**. The full upstream texts and
copyright notices listed below are retained without modification from the
recorded source commit. Upstream alternative notices are also retained; this
does not change the declaration in generated inventories or the application/SDK license.

- Published source: <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate>
- Cargo.lock / crate archive SHA-256: `e2db585e1d738fc771bf08a151420d3ed193d9d895a36df7f6f8a9456b911ddc`
- Repository: <https://github.com/brendanzab/gl-rs>
- Recorded `.cargo_vcs_info.json` commit: `f150967b1c44ae888e6676f93f639ebc82771bdc`
- Recorded package directory: `(not recorded)`

The crate also embeds Khronos/OpenGL/EGL/WebGL and ANGLE definitions. Their
complete original leading copyright/license comments are copied verbatim from
the exact published source files below. The gl-rs recorded tree pins ANGLE at
`7403dd2cd3764fe96660fe09892e764e9ae1dbca`; its complete BSD notice is retained.
Both included ANGLE XML files were byte-compared with that upstream commit.
EGL/WebGL permission notices remain in addition to the crate Apache declaration.
The recorded WebGL submodule root has no standalone license file; its included
IDL notices are retained, without substituting a current-branch license.

| Retained file | Exact source | Bytes | SHA-256 |
| --- | --- | --- | --- |
| LICENSE | <https://raw.githubusercontent.com/brendanzab/gl-rs/f150967b1c44ae888e6676f93f639ebc82771bdc/LICENSE> | 11358 | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |
| bundled-notices/ANGLE-LICENSE | <https://raw.githubusercontent.com/google/angle/7403dd2cd3764fe96660fe09892e764e9ae1dbca/LICENSE> | 1643 | `bf4da21bd20bcfb5b60b7ecc67fa864a79be049e21d6178076887f178dd6c71a` |
| bundled-notices/api/xml/gl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api/xml/gl.xml> | 996 | `ae502497a3655e23be6e5d5865884055e9ee6e2d6ce4b1ad49b872fd2ec0d6ad` |
| bundled-notices/api/xml/glx.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api/xml/glx.xml> | 966 | `b1e6618ec009690b5bd5f24efb58c31a8989978adc134ca9abc8253202ec6fe3` |
| bundled-notices/api/xml/wgl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api/xml/wgl.xml> | 1081 | `a3f6b56ea94d349df63fbbf2cde874937b8c6c4620f44d7354514e2087283c0b` |
| bundled-notices/api_angle/scripts/egl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_angle/scripts/egl.xml> | 1165 | `8a7572d955f871de863843c527759ce04c883cadd88c250989859e50ace5eb01` |
| bundled-notices/api_angle/scripts/egl_angle_ext.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_angle/scripts/egl_angle_ext.xml> | 280 | `e3de431001525745a7dc68663cd51ae90317fb9343827ebe74e44ecf79f81299` |
| bundled-notices/api_angle/scripts/gl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_angle/scripts/gl.xml> | 996 | `ae502497a3655e23be6e5d5865884055e9ee6e2d6ce4b1ad49b872fd2ec0d6ad` |
| bundled-notices/api_angle/scripts/gl_angle_ext.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_angle/scripts/gl_angle_ext.xml> | 279 | `fa7f47ecbad1475e51481171a212bd580a9d1ba71b4b076390f3d351f8f027ce` |
| bundled-notices/api_angle/scripts/wgl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_angle/scripts/wgl.xml> | 1081 | `a3f6b56ea94d349df63fbbf2cde874937b8c6c4620f44d7354514e2087283c0b` |
| bundled-notices/api_egl/api/egl.xml.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_egl/api/egl.xml> | 1165 | `8a7572d955f871de863843c527759ce04c883cadd88c250989859e50ace5eb01` |
| bundled-notices/api_webgl/specs/latest/1.0/webgl.idl.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_webgl/specs/latest/1.0/webgl.idl> | 1315 | `0976403647ef6c51ad03594182c7b72588a167fd56feb1fc530f55534a597b20` |
| bundled-notices/api_webgl/specs/latest/2.0/webgl2.idl.notice | <https://static.crates.io/crates/khronos_api/khronos_api-3.1.0.crate#api_webgl/specs/latest/2.0/webgl2.idl> | 1315 | `0976403647ef6c51ad03594182c7b72588a167fd56feb1fc530f55534a597b20` |

Use `packaging/licenses` as the existing release collector's `--license-overrides`
root. Keep all these version-specific texts, source links and hashes in the
distributed `licenses/<name-version>/` directory. No new package, runtime
feature, mandatory fee or paid license branch is introduced by this supplement.
