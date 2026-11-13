# Holo source fixtures

The shared compiler, CLI, and native integration tests load these source inputs
directly. The fixtures preserve concrete programs and package boundaries so the
tests can check compilation, output, and error behavior.

| Input | Coverage |
|---|---|
| `region.holo`, `recovery.holo` | Bounded regions, retained references, reset and recovery |
| `borrowed_buffer.holo`, `byte_input.holo` | Borrowed input, alias-visible state, byte initializers |
| `typed_append.holo`, `typed_frame.holo` | Typed rejection, rollback and retry during record construction |
| `typed_refill_frame.holo`, `typed_stream_reader.holo` | Typed frame handling with native file or stream input. The incremental reader imports `holo.buffer` from `lib/holo`. |
| [Error union package](error_unions/README.md) | Locked multi-file resolution, nested error cases, retry payloads |
| [Gzip header inspector](gzip_header/README.md) | Formatter regression input and an independent native verifier |
| [Telemetry worker](telemetry_worker/README.md) | Import resolution, locked package and independent native verifier |

Run the shared suite from the repository root with matched LLVM tools installed.

```sh
cargo test --offline --locked --workspace
```

See the [validation suite](../README.md) for test ownership and the fixture
guides for standalone reproduction commands.
