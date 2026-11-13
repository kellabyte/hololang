<p align="center">
  <img src="logo.png" alt="Holo Lang" width="520">
</p>

Holo is an **experimental** systems language for readable, high performance native
code. Its Goose inspired memory model aims to move fewer bytes and make memory
costs predictable enough to compete with tuned native implementations.

**Today**, the Rust compiler checks lifetimes and plans storage before lowering
to LLVM. The experimental subset includes local packages and typed `Result`
values for recoverable errors. `try` propagates failures, and `on` handles them.

The language and standard library are still taking shape and highly experimental.

## Goose inspired memory

Goose assigns growable values to **data stacks** separate from native call
frames. A value at the top grows by advancing a pointer. At scope exit, lifetime
checks let the program restore the saved pointer and reuse the whole tail.
[Goose data stacks](https://aardappel.github.io/goose/docs/reference.html#data-stacks)

- **Predictable cleanup.** A reset does fixed work per stack, with no GC pauses,
  reference count updates, or per object frees for scoped data. This removes
  sources of tail latency spikes. The effect on p99 depends on the workload.
- **Better cache locality.** Inline fields keep related bytes together, so a
  scan can touch fewer cache lines and chase fewer pointers. Smaller headers
  can fit more payload in cache.
- **Stable growth.** In place appends keep existing references valid and avoid
  copying old elements. Constructors can write directly into the final slot.

Holo implements checked in place append, bulk reset, and packed records in
explicit bounded regions. Goose can also store links as narrow offsets to save
space. Relative links and automatic data stack assignment remain design work
in Holo. [Holo memory rules](docs/spec.md#m1-memory-rules) · [Goose relative links](https://aardappel.github.io/goose/#links-narrower-than-pointers)

## Structs and memory layouts
```diff
--- PROPOSAL ---
```
A struct defines the contract for a value’s fields and their types. The proposed `layout` declaration lets you combine memory storage strategies for those fields. This design sketch puts `server` and `cpu` together in rows and delta encodes the timestamp column. [Layout proposal](docs/spec.md#9-layouts-encodings-and-collections)

```rust
struct Sample {
    server u32
    time   u64
    cpu    f32
}

layout Telemetry for Sample {
    layout server, cpu as rows
    layout time as columns using Delta
}

let samples = Array<Sample, Telemetry>([
    Sample { server: 7, time: 1000, cpu: 0.25 },
    Sample { server: 7, time: 1003, cpu: 0.40 }
])

for readonly sample in samples {
    core.print(sample.time)
}
```

`Array<Sample, Telemetry>` pairs the record type with its storage layout. Delta
encoding can store `1000` followed by the difference `3`. Field access yields the
original `u64` timestamps, so the loop prints `1000` and `1003`.

Inline records and inspectable packed layouts work today. Named layouts, codecs,
and the array construction shown here are proposed.

## Examples

| Program | What it demonstrates | Scope |
|---|---|---|
| [Hello World](examples/hello_world/README.md) | Prints `Hello, world!` with `make hello`. | Standalone program with no package dependencies. |
| [TCP echo](examples/tcp_echo/README.md) | Receives a byte, sends it back, and closes the connection. The streaming variant echoes binary data until the client finishes sending. | IPv4 loopback demos with bounded buffers. TLS and remote listening are outside the demo. |
| [Redis server](examples/redis_server/README.md) | Stores binary keys and values in memory and serves plain RESP2 `GET` and `SET`. Try it with `redis-cli -2`. | 16 entries, keys up to 32 bytes, and values up to 128 bytes. The demo serves clients one at a time and omits persistence, expiration, authentication, and RESP3. |

The [examples guide](examples/README.md) links to build and run instructions.

## Getting started

On 64-bit macOS or Linux (Arm64 or x86-64), install Rust 1.88 or later and GNU
Make. Compiling a Holo program also requires matching LLVM `opt` and `clang`
tools and a system linker.
[CLI setup](cli/README.md)

From the repository root, build the compiler and try Hello World.

Build the Holo compiler.
```sh
make
```

Compile and run the hello world Holo program with the Holo compiler.
```sh
./build/cargo/release/holo build examples/hello_world/main.holo -o hello
./hello
Hello, world!
```

Run `make check` to verify Hello World and the Redis demo from a fresh
source snapshot.

### Build output

Cargo writes compiler artifacts under `build/cargo/`. Use `build/examples/`
for native examples and `build/failures/` for test replay inputs.

## Finding your way

```text
compiler/     Rust compiler library and LLVM backend
cli/          The holo command line executable
lib/holo/     Holo standard-library modules
runtime/      Native LLVM prelude and optional C measurement support
examples/     Runnable Holo source programs
tests/        Integration tests by component, plus reference interpreter checks
docs/         Specification, packages, compiler architecture, plan, and research
```

[Language design](docs/spec.md) · [Compiler](docs/compiler.md) · [Errors](docs/error-handling.md) · [Examples](examples/README.md)

## Collaborating

Holo is a personal learning project exploring my ideas about the future of software.
I’m still figuring out what I want it to become. For now, I’ll make the design
decisions so I have room to explore my ideas, try things, and change my mind.
I welcome questions, suggestions, and contributions, and I hope more people will
help shape Holo as it grows.

I may take time to think over an idea or pull request, revisit it when the project
is ready, or decline it because I want to explore a different direction. I’ll get
some decisions wrong, and it may take me a while to see the value in a good idea.

I use AI as part of exploring and building Holo. If you’d like to help, feel free
to start a conversation or open a pull request directly. If talking through a 
larger change would help before you invest time in it, I’d be glad to do that.
Either way, I appreciate the care you bring to the project.

I use AI every day in my day job, and Holo gives me room to be more ambitious and find out where I do well and where I fall short when I take on problems that challenge my current skills in new ways.

If you’re up for experimenting together and seeing where it leads, I’d love
the company.
