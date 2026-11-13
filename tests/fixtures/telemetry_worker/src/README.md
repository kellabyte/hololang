# Telemetry package sources

[`main.holo`](main.holo) owns input admission, the reusable slot, parser state,
and outcome reporting. It selects [`telemetry.frame`](frame/README.md) with
`using` for header decoding and validation. [`wire.holo`](wire.holo) uses `holo.bytes`
to decode copied little-endian octets. The manifest selects this directory
as the source root. The [worker guide](../README.md) documents the native
host contract and commands for checking and building the package.
