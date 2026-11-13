# Use Cargo for compiler builds and Holo for native example builds.
# Cargo's artifacts stay under build/cargo, away from maintained source.

CARGO_TARGET_DIR := build/cargo

.PHONY: build hello test clean

build:
	cargo build --locked --release -p holo --target-dir "$(CARGO_TARGET_DIR)"

hello: build
	"$(CARGO_TARGET_DIR)/release/holo" run examples/hello_world/main.holo

test:
	cargo test --locked --workspace --target-dir "$(CARGO_TARGET_DIR)"

# Clean the repository's generated output, even if Cargo uses an override.
clean:
	rm -rf ./build
