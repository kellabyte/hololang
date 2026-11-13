# Holo core standard library

The core standard library is written in Holo and ships as the local
[`holo` package](holo.toml).

| Namespace | Purpose |
| --- | --- |
| [`holo.bytes`](bytes.holo) | Byte helpers, including joining two low octets in little-endian order. |
| [`holo.net`](net.holo) | Typed results for socket receive, send, and readiness operations. |
| [`holo.net.tcp`](net/tcp/README.md) | Typed results for closing a TCP stream. |

Declare `holo` as a local dependency in your package manifest, then import the
namespace you need with a directive such as `using holo.bytes`.
[Package guide](../../docs/packages.md)

Check all namespaces from the repository root.

```sh
cargo run --offline -p holo -- check --strict-names --package lib/holo/holo.toml
```
