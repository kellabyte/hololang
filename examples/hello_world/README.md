# Hello World

[main.holo](main.holo) is a standalone Holo program with no package dependencies.
Its entry point passes a fixed text literal to `core.print`, writes one line to
standard output, and returns exit status zero.

From the repository root, run `make hello`. It builds the CLI and runs the
example with the native LLVM tools described in the [CLI guide](../../cli/README.md).
The expected output from the program is `Hello, world!` followed by a newline.
