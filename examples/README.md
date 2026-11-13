# Holo examples

The programs build with the experimental Holo compiler and a matched LLVM
`opt` and `clang` installation. See [CLI setup](../cli/README.md) for toolchain
details.

| Example | What it does | Scope |
|---|---|---|
| [Hello World](hello_world/README.md) | Prints `Hello, world!`. Run `make hello` from the repository root. | A standalone program using a text literal. |
| [TCP echo](tcp_echo/README.md) | Echoes one byte and closes the connection. The streaming worker echoes binary data in a reusable buffer and handles short writes. | IPv4 loopback, with finite client counts and bounded buffers. The demo omits TLS and remote listening. |
| [Redis server](redis_server/README.md) | Implements plain RESP2 `GET` and `SET` over TCP. An in-memory table survives client reconnects while the server is running. | 16 entries, 32-byte keys, and 128-byte values. One client at a time. Additional Redis commands, RESP3, expiration, persistence, and authentication are outside the demo. |

The example guides include local commands to try the programs. Compiler test inputs
live with the [test suite](../tests/README.md).
