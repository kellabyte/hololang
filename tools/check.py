#!/usr/bin/env python3
"""Build a fresh Holo checkout snapshot and verify the 0.1 preview examples."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Iterator


SOURCE_EXPORT_MARKER = ".holo-source-export"
SOURCE_EXPORT_ID = b"Holo source export v1\n"


def checkout_sources(root: Path) -> Iterator[Path]:
    """Yield the regular tracked and unignored paths in a Git checkout."""
    checkout = subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], cwd=root,
    )
    if Path(os.fsdecode(checkout).removesuffix("\n")).resolve() != root:
        raise RuntimeError("run the preview check from a Holo Git checkout")
    paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=root,
    )
    for name in dict.fromkeys(paths.split(b"\0")):
        if not name:
            continue
        relative = Path(os.fsdecode(name))
        source = root / relative
        # A checkout can delete a tracked file or replace it with a directory.
        if source.resolve() != source:
            raise RuntimeError(f"preview source path is symbolic: {relative}")
        if not source.exists() or source.is_dir():
            continue
        if not source.is_file():
            raise RuntimeError(f"preview source path is not a regular file: {relative}")
        yield relative


def copy_sources(root: Path, snapshot: Path) -> None:
    """Copy the Git checkout into a new tree, excluding ignored build output."""
    for relative in checkout_sources(root):
        destination = snapshot / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / relative, destination)


def verify(root: Path, offline: bool, redis_cli: str) -> None:
    """Build and test the native examples against independent output checks."""
    client = shutil.which(redis_cli)
    if client is None:
        raise RuntimeError(f"Redis client executable not found: {redis_cli}")
    client = os.path.abspath(client)

    with tempfile.TemporaryDirectory(prefix="holo-preview-") as directory:
        marker = root / SOURCE_EXPORT_MARKER
        if marker.is_file() and marker.read_bytes() == SOURCE_EXPORT_ID:
            # The extracted archive is already a source snapshot.
            snapshot = root
        else:
            snapshot = Path(directory) / "source"
            snapshot.mkdir()
            copy_sources(root, snapshot)
        cargo_target = Path(directory) / "cargo"
        holo = cargo_target / "release" / "holo"
        hello = Path(directory) / "hello"
        redis_server = Path(directory) / "redis-server"

        print("Building the locked Holo release from a fresh source snapshot", flush=True)
        cargo = ["cargo", "build", "--locked", "--release", "-p", "holo",
                 "--target-dir", cargo_target]
        if offline:
            cargo.append("--offline")
        subprocess.run(cargo, cwd=snapshot, check=True)

        subprocess.run(
            [holo, "build", "--release", "examples/hello_world/main.holo", "-o", hello],
            cwd=snapshot, check=True,
        )
        greeting = subprocess.run([hello], cwd=snapshot, capture_output=True, check=True, timeout=10)
        if greeting.stdout != b"Hello, world!\n" or greeting.stderr:
            raise RuntimeError(
                f"unexpected Hello World output: stdout={greeting.stdout!r}, stderr={greeting.stderr!r}"
            )
        print(greeting.stdout.decode().rstrip(), flush=True)

        subprocess.run(
            [holo, "build", "--locked", "--release", "--package",
             "examples/redis_server/holo.toml", "-o", redis_server],
            cwd=snapshot, check=True,
        )
        subprocess.run(
            [sys.executable, "-B", "examples/redis_server/verify_parser.py", "--holo", holo,
             "--output", Path(directory) / "parser"],
            cwd=snapshot, check=True,
        )
        subprocess.run(
            [sys.executable, "-B", "examples/redis_server/verify_server.py", redis_server,
             "--listener", "--redis-cli", client],
            cwd=snapshot, check=True,
        )
        print("Holo 0.1 preview check passed", flush=True)


def main() -> None:
    """Read verification options and report failures as a nonzero exit."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="use Cargo's local dependency cache")
    parser.add_argument("--redis-cli", default="redis-cli", help="Redis client executable")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        verify(root, args.offline, args.redis_cli)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"preview check: {error}\n")


if __name__ == "__main__":
    main()
