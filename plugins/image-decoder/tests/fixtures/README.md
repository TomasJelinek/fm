# AVIF test fixture provenance

Unmodified fixtures from libavif 1.3.0, immutable commit
`1aadfad932c98c069a1204261b1856f81f3bc199`, under its BSD-2-Clause license.
The exact upstream test-data README explicitly licenses each selected file under
the same terms as libavif. The complete upstream license is retained in
[libavif-LICENSE](libavif-LICENSE), including its unrelated bundled-code notices.

Source: https://github.com/AOMediaCodec/libavif/tree/1aadfad932c98c069a1204261b1856f81f3bc199/tests/data

`white_1x1.avif` is an opaque white pixel encoded at upstream default (lossy) quality. `draw_points_idat.avif` is the upstream
quality 100 AVIF encoding of the generated `draw_points.png`, with color and alpha
items stored in `idat`. The PNG supplies independent reference colors/alpha.
No photograph or third-party font artwork is used in these fixtures.

| File | Bytes | SHA-256 |
| --- | --- | --- |
| white_1x1.avif | 305 | `ea4e43d1f07e4c00de16c13afa32376111bb306e51f08212cf4c1b6064df3667` |
| draw_points_idat.avif | 498 | `ce2fd627efae49391ea82584e9beae05959b867ba429e688a2b95a015b38d3db` |
| draw_points.png | 304 | `48d684549df1239efcbda1418684e19d827bdd3d83d5beea1f90fce3822f9572` |
