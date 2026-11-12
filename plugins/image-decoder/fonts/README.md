# Bundled SVG fonts

Unmodified font files from the immutable cosmic-text 0.19.0 crates.io archive,
`fonts/` directory. The upstream Noto font project is https://github.com/notofonts.
These fonts retain SIL Open Font License 1.1; see [OFL.txt](OFL.txt) and
[the exact embedded copyright notices](COPYRIGHT.md).
The worker embeds these bytes and never reads system or document-selected fonts.
Latin/Greek/Cyrillic, Arabic and Hebrew fallback is available; this is not full
Unicode or exact system-font coverage. Other requested families use Noto Sans.

| File | SHA-256 |
| --- | --- |
| NotoSans-Regular.ttf | `2ec33f84606cbaa0a1a944488e14f97faf2f6a25ecdd8354f5358f06da13c7d9` |
| NotoSansArabic.ttf | `ee489b994b3e62def9874c918145e32b133b625abaf98cec60502bdb40102c56` |
| NotoSansHebrew.ttf | `3d4fef85b449ade4d165de982969374fa30b2a5fe7bc679f5a3f5bfc047fb703` |
