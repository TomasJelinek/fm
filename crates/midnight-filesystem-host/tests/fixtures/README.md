# Archive worker input

`hello.gz` is a 25-byte GZIP stream containing the five ASCII bytes `hello`, with
a zero modification timestamp. It is self-authored test data with no third-party
content. Keep this source fixture immutable: the native worker test must not race
endpoint security metadata updates on newly created files. A separate captured-copy
test deliberately changes its input and verifies revision rejection.
