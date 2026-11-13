# Compose dashboard errors

The dashboard loads a session, queries a count, and saves it. Three named
children in `DashboardError` retain which service failed. Each `try` binds
the successful value or returns a failure immediately. The caller matches
child cases such as `Session.PermissionDenied` and reads the retry delay from
`Sql.Timeout(delay)`. `on error` handles the remaining SQL failure.

```sh
cargo run --offline -p holo -- check --locked --package tests/fixtures/error_unions/holo.toml
cargo run --offline -p holo -- run --locked --release --package tests/fixtures/error_unions/holo.toml
```

The program prints `42`, `1`, `500`, `3`, and `4` on separate lines. The
services are deterministic stand-ins and the UI functions return observable
codes. The [source directory](src/README.md) explains how the files fit together.
