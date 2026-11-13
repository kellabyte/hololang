#!/usr/bin/env python3
"""Check the hosted worker against a whole-input model and its flat LLVM control.

Run from the repository root with a built CLI and a new output directory. Each
native case retains the wire, command, stdout, stderr and expected counters.
Socket writers fragment their input and close the write side to end the worker.
"""

import argparse
import json
import socket
import subprocess
import time
from pathlib import Path


# The output field order is also the native worker's plain-text interface.
FIELDS = (
    "outcome", "bytes_seen", "chunks_seen", "hits", "misses", "errors",
    "checksum", "bad_magic", "bad_kind", "oversized", "truncated",
)
LAYOUTS = ("wide", "derived", "compact")
PACKAGE = Path("tests/fixtures/telemetry_worker")
MANIFEST = PACKAGE / "holo.toml"
SOURCE = PACKAGE / "src"
# Match package binding order so the standalone control retains declaration IDs.
FILES = (
    SOURCE / "main.holo",
    SOURCE / "frame/decode.holo", SOURCE / "frame/validate.holo",
    SOURCE / "wire.holo",
    Path("lib/holo/bytes.holo"),
    Path("lib/holo/bytes/read/cursor.holo"), Path("lib/holo/bytes/read/integer.holo"),
)
TIMEOUT = 10


def frame(kind, payload, magic=165):
    """Build a fixture record from one-octet tags and a representable payload."""
    return bytes((magic, kind)) + len(payload).to_bytes(2, "little") + payload


def expected(wire):
    """Return whole-wire counter expectations, independent of read boundaries."""
    counts = dict.fromkeys(FIELDS, 0)
    counts["bytes_seen"] = len(wire)
    offset = 0
    # Each iteration consumes one complete frame or marks the final suffix.
    while len(wire) - offset >= 4:
        magic, kind = wire[offset:offset + 2]
        length = int.from_bytes(wire[offset + 2:offset + 4], "little")
        stop = offset + 4 + length
        # Incomplete frames cannot affect any frame or checksum totals.
        if stop > len(wire):
            break
        # The oracle checks complete frames in the documented precedence order.
        if length > 4096:
            counts["oversized"] += 1
        elif magic != 165:
            counts["bad_magic"] += 1
        elif kind > 2:
            counts["bad_kind"] += 1
        else:
            counts[FIELDS[3 + kind]] += 1
            counts["checksum"] += sum(wire[offset + 4:stop])
        offset = stop
    counts["truncated"] = int(offset != len(wire))
    return counts


def capture(directory, command, result):
    """Save one subprocess invocation and its exact output before inspection."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "command.json").write_text(json.dumps(command) + "\n")
    (directory / "stdout").write_bytes(result.stdout)
    (directory / "stderr").write_bytes(result.stderr)
    (directory / "exit-status").write_text(f"{result.returncode}\n")
    result.check_returncode()
    return result.stdout


def check_counters(directory, wire, stdout, file_input):
    """Check exact totals and the transport's permitted positive-read count."""
    actual = dict(zip(FIELDS, map(int, stdout.splitlines()), strict=True))
    want = expected(wire)
    # The bounded regular file is read in full slots except for its final tail.
    # Stream segmentation is an OS choice. Each chunk spans 1–4,096 bytes.
    minimum = (len(wire) + 4095) // 4096
    if file_input:
        want["chunks_seen"] = minimum
    else:
        assert minimum <= actual["chunks_seen"] <= len(wire), actual
        want["chunks_seen"] = actual["chunks_seen"]
    (directory / "expected.json").write_text(json.dumps(want, indent=2) + "\n")
    assert actual == want, (directory, actual, want)


def run_file(executable, directory, wire):
    """Run one file session with a fresh immutable fixture and bounded time."""
    directory.mkdir()
    fixture = directory / "input.bin"
    fixture.write_bytes(wire)
    command = [str(executable), "file", str(fixture)]
    result = subprocess.run(command, capture_output=True, timeout=TIMEOUT)
    check_counters(directory, wire, capture(directory, command, result), True)


def run_stream(executable, directory, pieces):
    """Run one inherited nonblocking stream and check its half-closed input."""
    directory.mkdir()
    wire = b"".join(pieces)
    (directory / "input.bin").write_bytes(wire)
    (directory / "send-sizes.json").write_text(json.dumps(list(map(len, pieces))) + "\n")
    reader, writer = socket.socketpair()
    with reader, writer:
        reader.setblocking(False)
        writer.settimeout(TIMEOUT)
        command = [str(executable), "stream-fd", str(reader.fileno())]
        with subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            pass_fds=(reader.fileno(),),
        ) as child:
            reader.close()
            try:
                # Give the worker opportunities to see short reads and wait.
                # Kernel coalescing remains valid, so the oracle allows it.
                for piece in pieces:
                    time.sleep(0.03)
                    writer.sendall(piece)
                writer.shutdown(socket.SHUT_WR)
                stdout, stderr = child.communicate(timeout=TIMEOUT)
            except BaseException:
                # Reap exactly this worker even when a fixture or timeout fails.
                child.kill()
                child.communicate()
                raise
            result = subprocess.CompletedProcess(command, child.returncode, stdout, stderr)
    check_counters(directory, wire, capture(directory, command, result), False)


def flatten():
    """Combine the actual example bodies into an equivalent standalone source."""
    result = []
    # Use the authored bodies, including the namespace-private signature.
    # Only using directives, namespace headers, and qualification change in the control.
    for path in FILES:
        for line in path.read_text().splitlines(keepends=True):
            # Standalone source uses its own scope and the same function bodies.
            if line.startswith(("namespace ", "using ")):
                continue
            if line.startswith(("public func ", "public error ", "public struct ")):
                line = line.removeprefix("public ")
            line = line.replace("read.Cursor", "Cursor")
            # Redirect only the known cross-namespace call spellings.
            for qualified in (
                "bytes.join_low_octets_le", "wire.little_endian_u16",
                "read.start", "read.take_u8", "read.take_u16_le",
                "frame.payload_length", "frame.payload_limit", "frame.expected_magic",
            ):
                line = line.replace(qualified + "(", qualified.split(".")[1] + "(")
            result.append(line)
        result.append("\n")
    return "".join(result)


def verify(holo, output):
    """Build six native configurations and verify their single-file IR controls."""
    output.mkdir(parents=True)
    flat = output / "flattened.holo"
    flat.write_text(flatten())
    mixed = (
        frame(0, bytes((11, 2))) + frame(1, b"") + frame(2, b"abc")
        + frame(8, b"bad") + frame(0, b"wrong", 0)
    )
    # The following header starts at byte 4093. Its high octet arrives
    # in the next file read, after the low octet at byte 4095.
    split = frame(1, b"f" * 4089) + frame(2, b"tail")
    large = frame(1, b"z" * 4096)
    oversized = frame(9, b"z" * 65535, 0) + frame(0, b"ok")
    files = {
        "empty": b"",
        "mixed": mixed,
        "split-header": split,
        "maximum-body": large,
        "oversized": oversized,
        "partial-header": mixed + b"\xa5\x01\xff",
        "partial-body": mixed + frame(1, b"incomplete")[:-3],
    }
    stream = [split[:4095], split[4095:4096], split[4096:], mixed, b"\xa5"]
    # Match each layout with its own flat IR and run both native opt levels.
    for layout in LAYOUTS:
        directory = output / layout
        directory.mkdir()
        options = ["--layout", layout]
        packaged = options + ["--package", str(MANIFEST), "--locked"]
        module_command = [str(holo), "emit-llvm", *packaged]
        flat_command = [str(holo), "emit-llvm", *options, str(flat)]
        module_ir = capture(directory / "module-ir", module_command, subprocess.run(
            module_command, capture_output=True, timeout=TIMEOUT,
        ))
        flat_ir = capture(directory / "flat-ir", flat_command, subprocess.run(
            flat_command, capture_output=True, timeout=TIMEOUT,
        ))
        assert module_ir == flat_ir, f"LLVM IR differs for {layout}"
        # Each binary receives identical file and stream inputs.
        for optimize in ("o0", "o3"):
            native = directory / optimize
            native.mkdir()
            executable = (native / "worker").resolve()
            command = [str(holo), "build", *packaged, "-o", str(executable)]
            # The CLI's default is O0. Release requests native O3.
            if optimize == "o3":
                command.append("--release")
            capture(native / "build", command, subprocess.run(
                command, capture_output=True, timeout=30,
            ))
            # Each file has a fresh owned descriptor and parser state.
            for name, wire in files.items():
                run_file(executable, native / name, wire)
            run_stream(executable, native / "fragmented-stream", stream)
            run_stream(executable, native / "empty-stream", [])
        print(f"{layout}: identical LLVM IR and 18 native sessions passed")


def main():
    """Read the built CLI and new output directory from the command line."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--holo", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    verify(args.holo.resolve(), args.output)


# Allow fixture helpers to be imported without running subprocesses.
if __name__ == "__main__":
    main()
