#!/usr/bin/env python3
"""Write a reproducible Holo 0.1 source candidate and its SHA-256 checksum."""

import argparse
import gzip
import hashlib
import io
from pathlib import Path
import stat
import subprocess
import tarfile
import tempfile

from check import SOURCE_EXPORT_ID, SOURCE_EXPORT_MARKER, checkout_sources


ARCHIVE_NAME = "holo-0.1.0-rc-source"


def export(root: Path, output: Path) -> None:
    """Pack current source bytes in a stable order with normalized metadata."""
    paths = sorted(set(checkout_sources(root)) - {Path(SOURCE_EXPORT_MARKER)})
    required = {Path("Cargo.lock"), Path("LICENSE"), Path("docs/releases/unreleased.md")}
    if not required.issubset(paths):
        raise RuntimeError("the source candidate requires Cargo.lock, LICENSE, and the release notes")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{ARCHIVE_NAME}.tar.gz"
    checksum = output / f"{archive.name}.sha256"
    if archive.exists() or checksum.exists():
        raise FileExistsError(f"choose a new output directory for {archive.name}")

    # Write into temporary siblings and rename only after the archive is complete.
    with tempfile.TemporaryDirectory(prefix=".holo-export-", dir=output) as staging:
        staged_archive = Path(staging) / archive.name
        with staged_archive.open("wb") as raw, gzip.GzipFile(
            fileobj=raw, filename="", mode="wb", mtime=0
        ) as compressed, tarfile.open(fileobj=compressed, mode="w") as bundle:
            marker = tarfile.TarInfo(f"{ARCHIVE_NAME}/{SOURCE_EXPORT_MARKER}")
            marker.size = len(SOURCE_EXPORT_ID)
            marker.mode = 0o644
            bundle.addfile(marker, io.BytesIO(SOURCE_EXPORT_ID))

            for relative in paths:
                source = root / relative
                metadata = source.stat()
                if not stat.S_ISREG(metadata.st_mode):
                    raise RuntimeError(f"source candidate path is not a regular file: {relative}")
                member = tarfile.TarInfo(f"{ARCHIVE_NAME}/{relative.as_posix()}")
                member.size = metadata.st_size
                member.mode = 0o755 if metadata.st_mode & stat.S_IXUSR else 0o644
                with source.open("rb") as payload:
                    bundle.addfile(member, payload)

        digest = hashlib.sha256(staged_archive.read_bytes()).hexdigest()
        staged_checksum = Path(staging) / checksum.name
        staged_checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
        staged_archive.rename(archive)
        staged_checksum.rename(checksum)
    print(archive)
    print(checksum)


def main() -> None:
    """Read the output path and fail without replacing an existing candidate."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="output directory, default: build/release")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = (args.output or root / "build" / "release").resolve()
    try:
        if output.is_relative_to(root) and not output.is_relative_to(root / "build"):
            raise RuntimeError("choose an output directory under build/ or outside the checkout")
        export(root, output)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"source export: {error}\n")


if __name__ == "__main__":
    main()
