# XML source highlighting — 12 November 2026

Extend the independent Source viewer from its current 18 lexical profiles to 19.
Add XML-family defaults: xml, xsd, xsl, xslt, xaml, csproj, fsproj, vbproj, vcxproj,
props, targets and resx. HTML/SVG routing and saved user overrides stay intact.

Use a dedicated small XML lexical-state module called by the existing highlighter.
Recognize element/attribute names, quoted values, comments, CDATA, processing
instructions, character/entity-reference spelling and inert declarations. Keep
complete-line checkpoint semantics, Unicode source boundaries, cancellation and
existing viewport/read/span/checkpoint limits. Do not parse or expand XML entities,
load DTDs, resolve URLs, build a document tree or scan whole files.

Tests cover real styled output, multiline state, XML quoting, Unicode names,
resource/cancellation rollback and SDK viewport/huge-line bounded reads. Build
the separate plugin workspace with released public SDK only; record Windows/Linux
results and rebuilt DLL/manifest locations separately from deployment.

[W3C XML 1.0 names and markup](https://www.w3.org/TR/xml/#sec-common-syn)
provides the lexical reference; this feature is highlighting, not XML validation.