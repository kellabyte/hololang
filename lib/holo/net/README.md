# Networking modules

The parent [`holo.net`](../net.holo) module supplies shared typed socket
outcomes. This directory holds protocol-specific child namespaces.

`net.recv_result`, `net.send_result`, and `net.wait_result` decode the
receive, send, and readiness status domains. `net.next_session_result`
decodes `core.next_session()`. Success admits a client, `SessionError.End`
marks the end of a file or inherited stream, and `Interrupted` permits
another accept attempt. `Native(code)` preserves the positive native errno.

| Module | Purpose |
|---|---|
| [`holo.net.tcp`](tcp/README.md) | Terminal close results for the checked hosted TCP stream. |

Check the entire standard-library package from the repository root with
`holo check --strict-names --package lib/holo/holo.toml`.
