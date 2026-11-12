# Copy buffer probe

Decision: retain the current 256 KiB production transfer buffer. A small local
probe does not show a reliable benefit from increasing it to 1 or 4 MiB.

Windows 11, D: NTFS with 4 KiB clusters, SK hynix PC801 1 TB NVMe. The standalone
.NET 10 probe used a 64 MiB random source, buffered synchronous I/O, five rotated
rounds, explicit Flush(true), and independently reopened destination readback.
The full mode computed source SHA-256 while copying and destination SHA-256 on
readback. Sample mode captured and compared the first/middle/last 64 KiB, with
an additional untimed full-file SHA-256 audit. All audits passed.

| Buffer | Full SHA-256 median | Three-sample median |
| --- | ---: | ---: |
| 256 KiB | 168.77 ms | 96.69 ms |
| 1 MiB | 165.38 ms | 94.97 ms |
| 4 MiB | 168.85 ms | 98.92 ms |

The roughly 2% difference is smaller than run-to-run variation. This is a warm
filesystem-cache microbenchmark, not the Rust application's complete identity,
publication, overwrite or queue pipeline. It cannot establish cold-disk, network,
HDD or very-large-file performance. Source plus destination occupied at most
128 MiB and were cleaned afterward; artifacts are in `_build/qa/copy-buffer/`.

Windows caches buffered I/O independently of the application's buffer, and
explicit flushing is a separate operation; see Microsoft's
[buffered I/O documentation](https://learn.microsoft.com/en-us/windows/win32/fileio/flushing-system-buffered-i-o-data-to-disk).
Rust's [BufReader documentation](https://doc.rust-lang.org/std/io/struct.BufReader.html)
also distinguishes many small reads from already-large reads. Increasing buffer
size alone is not evidence of increased device throughput.

A production change requires end-to-end comparisons with the exact Rust pipeline,
including flush and selected verification, separate source/destination devices,
large and small files, cancellation latency and memory per concurrent job.
