# Telemetry package sources

[`main.holo`](main.holo) owns input admission, the reusable slot, parser state,
and outcome reporting. It selects [`telemetry.frame`](frame/README.md) with
`using` for header validation and uses `holo.bytes.read` for a checked cursor.
[`wire.holo`](wire.holo) combines the low octet retained after a short read
with the high octet from the next prefix. The manifest selects this directory
as the source root. The [worker guide](../README.md) documents the native host
contract and commands for checking and building the package.
