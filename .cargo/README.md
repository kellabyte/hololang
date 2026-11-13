# Cargo configuration

`config.toml` sets the workspace's default Cargo target directory to
`build/cargo/`. Direct Cargo commands and Make can share the same build cache.
Use `cargo metadata --no-deps --format-version 1` to see Cargo's effective
`target_directory` when using an override.
