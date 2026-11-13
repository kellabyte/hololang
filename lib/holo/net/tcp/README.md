# `holo.net.tcp`

Use `using holo.net.tcp` in a manifest package. [`close_error.holo`](close_error.holo)
defines the public terminal `CloseError` family. [`close.holo`](close.holo)
defines the pure `tcp.close_result(status)` adapter with a payload-free success.
It preserves a positive native error code, including interruption, and rejects words outside the
close status domain. Checked `noalloc` and `noblock` apply to the adapter.

The compiler supplies the opaque hosted stream binding and checks
`core.close(stream)`, which consumes it before returning the status.
See the [hosted TCP API](../../../../docs/packages.md#owned-tcp-capabilities).
Shared send, receive, and readiness adapters live in [`holo.net`](../../net.holo).

From the repository root, check the module with
`holo check --strict-names --package lib/holo/holo.toml`.
