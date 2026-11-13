#!/usr/bin/env python3
"""Check the hosted gzip header reader against Python's fixed-prefix model.

Run from the repository root with a built Holo CLI and a new output directory.
Retain exact fixture bytes, subprocess arguments, output, and generated LLVM.
The socket fixtures exercise bounded reads with split input and a live writer.
"""

import argparse
import gzip
import io
import json
import select
import socket
import struct
import subprocess
import time
from pathlib import Path


# Four octets, one unsigned little-endian word, and two final octets.
FIXED = struct.Struct("<BBBBIBB")
SOURCE = Path("tests/fixtures/gzip_header/main.holo")
MANIFEST = SOURCE.with_name("holo.toml")
READ_NAMESPACE = Path("lib/holo/bytes/read")
LAYOUTS = ("wide", "derived", "compact")
TIMEOUT = 10


def expected_output(wire):
    """Return exact stdout from the available prefix and fixed-field order."""
    # A mismatch in an available leading field wins over missing later bytes.
    for offset, (expected, status) in enumerate(((31, 2), (139, 2), (8, 3))):
        if offset < len(wire) and wire[offset] != expected:
            return f"{status}\n".encode()
    # The reserved flag bits start at bit five of the unsigned flags octet.
    if len(wire) >= 4 and wire[3] >= 32:
        return b"4\n"
    # An otherwise valid prefix must supply all fixed fields before publication.
    if len(wire) < FIXED.size:
        return b"1\n"
    _, _, _, flags, mtime, hint, operating_system = FIXED.unpack_from(wire)
    return f"0\n{mtime}\n{flags}\n{hint}\n{operating_system}\n".encode()


def file_fixtures():
    """Create genuine gzip members and bounded malformed or synthetic prefixes."""
    payload = b"Holo checks the gzip header.\n"
    member = io.BytesIO()
    # Python writes FNAME and the payload trailer, giving a real optional field.
    with gzip.GzipFile("sample.txt", "wb", fileobj=member, mtime=0x10203040) as archive:
        archive.write(payload)
    named = member.getvalue()
    # Verify the generator produced the intended optional field and valid member.
    if named[3] & 8 == 0 or gzip.decompress(named) != payload:
        raise ValueError("unexpected named gzip fixture")
    fixtures = {
        "named-member": named,
        "unspecified-time": gzip.compress(payload, mtime=0),
        "maximum-metadata": FIXED.pack(31, 139, 8, 0, 0xffffffff, 255, 254),
        "all-defined-flags": FIXED.pack(31, 139, 8, 31, 1, 0, 3),
        "bad-first-magic": b"\x00",
        "bad-second-magic": b"\x1f\x00",
        "unsupported-method": b"\x1f\x8b\x00\xff",
        "reserved-flag-5": b"\x1f\x8b\x08\x20",
        "reserved-flag-6": b"\x1f\x8b\x08\x40",
        "reserved-flag-7": b"\x1f\x8b\x08\x80",
        "rejection-order": b"\x00\x00\x00\xff" + named[4:],
    }
    # Every proper prefix of a valid header must return the truncation status.
    for size in range(FIXED.size):
        fixtures[f"truncated-{size}"] = named[:size]
    return fixtures


def record_result(directory, result):
    """Save the child's raw streams and exit status before propagating failure."""
    (directory / "stdout").write_bytes(result.stdout)
    (directory / "stderr").write_bytes(result.stderr)
    (directory / "exit-status").write_text(f"{result.returncode}\n")
    result.check_returncode()


def run_command(directory, command, timeout=TIMEOUT):
    """Run a bounded command and retain its arguments and captured streams."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "command.json").write_text(json.dumps(command) + "\n")
    try:
        result = subprocess.run(command, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as failure:
        # subprocess.run has already stopped its child and retained partial output.
        (directory / "stdout").write_bytes(failure.stdout or b"")
        (directory / "stderr").write_bytes(failure.stderr or b"")
        (directory / "timeout").write_text(f"{timeout}\n")
        raise
    record_result(directory, result)
    return result


def prepare_case(directory, wire):
    """Retain exact fixture bytes and oracle stdout before starting the child."""
    directory.mkdir()
    (directory / "input.bin").write_bytes(wire)
    want = expected_output(wire)
    (directory / "expected").write_bytes(want)
    return want


def check_output(directory, result, want):
    """Require exact output and empty stderr, with retained bytes for diagnosis."""
    # All specified parser statuses are ordinary returns in this example.
    if result.stdout != want or result.stderr:
        raise ValueError(
            f"{directory}: expected {want!r}, got {result.stdout!r}, "
            f"stderr {result.stderr!r}"
        )


def run_file(executable, directory, wire):
    """Run the parser once with a regular file and verify its exact report."""
    want = prepare_case(directory, wire)
    command = [str(executable), "file", str(directory / "input.bin")]
    check_output(directory, run_command(directory, command), want)


def wait_for_drain(reader, directory):
    """Observe that the child consumed a queued fragment without reading it."""
    deadline = time.monotonic() + TIMEOUT
    # The writer stays open, so readiness here means the fragment is still
    # queued. Never feed the next fragment until the receive queue is empty.
    while select.select([reader], [], [], 0)[0]:
        if time.monotonic() >= deadline:
            raise TimeoutError(f"{directory}: parser did not consume its fragment")
        time.sleep(0.001)


def run_stream(executable, directory, fragments, *, close_write, trailing=b""):
    """Check split sends and optionally prove a queued suffix remains unread."""
    want = prepare_case(directory, b"".join(fragments))
    (directory / "stream.json").write_text(json.dumps({
        "fragment_sizes": list(map(len, fragments)),
        "wait_for_fragment_drain": True,
        "write_half_closed": close_write,
        "unread_suffix_hex": trailing.hex(),
    }) + "\n")
    reader, writer = socket.socketpair()
    with reader, writer:
        reader.setblocking(False)
        writer.settimeout(TIMEOUT)
        command = [str(executable), "stream-fd", str(reader.fileno())]
        (directory / "command.json").write_text(json.dumps(command) + "\n")
        with subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            pass_fds=(reader.fileno(),),
        ) as child:
            try:
                # Queue each nonfinal fragment on its own to force a short
                # read. The final fragment may intentionally include a suffix.
                for fragment in fragments[:-1]:
                    writer.sendall(fragment)
                    wait_for_drain(reader, directory)
                writer.sendall(fragments[-1])
                # Truncation fixtures signal EOF. Other fixtures keep the peer
                # live through completion to witness parsing before EOF.
                if close_write:
                    writer.shutdown(socket.SHUT_WR)
                stdout, stderr = child.communicate(timeout=TIMEOUT)
            except BaseException:
                # Reap the child after interruption, failure, or deadline.
                child.kill()
                stdout, stderr = child.communicate()
                (directory / "stdout").write_bytes(stdout)
                (directory / "stderr").write_bytes(stderr)
                (directory / "exit-status").write_text(f"{child.returncode}\n")
                raise
        result = subprocess.CompletedProcess(command, child.returncode, stdout, stderr)
        record_result(directory, result)
        check_output(directory, result, want)
        # The parent reads from its duplicate only after the child exits. A
        # remaining sentinel witnesses the parser's fixed-header read bound.
        if trailing:
            reader.settimeout(TIMEOUT)
            actual = reader.recv(len(trailing))
            (directory / "unread-suffix.bin").write_bytes(actual)
            if actual != trailing:
                raise ValueError(f"{directory}: expected queued suffix {trailing!r}, got {actual!r}")


def stream_fixtures(header):
    """Create short-read, truncation, and held-open rejection witnesses."""
    # Send one octet at a time so the timestamp spans four separate reads.
    # The final sentinel must still be queued after completion.
    fragments = [bytes((octet,)) for octet in header[:-1]] + [header[-1:] + b"X"]
    return {
        "split-header": (fragments, False, b"X"),
        "split-eof": ([header[:5], header[5:9]], True, b""),
        "open-peer-rejection": ([b"\x1f", b"\x00"], False, b""),
    }


def write_flat_control(output):
    """Write equivalent source with the imported namespace in the entry file."""
    flat = output / "flattened.holo"
    source = SOURCE.read_text()
    using_line = "using holo.bytes.read\n"
    call = "read.u32_le("
    # Ensure the control removes a real directive and resolves a real library call.
    if source.count(using_line) != 1 or call not in source:
        raise ValueError(f"{SOURCE}: expected the endian reader import and call")
    source = source.replace(using_line, "").replace(call, "u32_le(")
    namespace_line = "namespace holo.bytes.read\n"
    contributors = []
    # Match package discovery order, including declarations unused by gzip.
    for path in sorted(READ_NAMESPACE.glob("*.holo")):
        library = path.read_text()
        if library.count(namespace_line) != 1:
            raise ValueError(f"{path}: expected one namespace holo.bytes.read header")
        library = library.replace(namespace_line, "")
        for declaration in ("func", "error", "struct"):
            library = library.replace(f"public {declaration} ", f"{declaration} ")
        contributors.append(library)
    flat.write_text("\n".join([source, *contributors]))
    return flat


def verify(holo, output):
    """Compare LLVM and run the fixed corpus across all native layout modes."""
    output.mkdir(parents=True)
    flat = write_flat_control(output)
    files = file_fixtures()
    streams = stream_fixtures(files["named-member"][:FIXED.size])
    # Use the same corpus for each layout to expose representation-dependent bugs.
    for layout in LAYOUTS:
        directory = output / layout
        options = ["--layout", layout]
        module = options + ["--package", str(MANIFEST), "--locked"]
        module_ir = run_command(directory / "module-ir", [str(holo), "emit-llvm", *module])
        flat_ir = run_command(directory / "flat-ir", [str(holo), "emit-llvm", *options, str(flat)])
        # Exact LLVM identity checks the namespace boundary before optimization.
        if module_ir.stdout != flat_ir.stdout:
            raise ValueError(f"{directory}: LLVM differs from flattened control")
        # O0 and O3 must report the same results for every input and transport.
        for mode, optimization in (("o0", []), ("o3", ["--release"])):
            native = directory / mode
            executable = native / "gzip-header"
            command = [str(holo), "build", *module, *optimization, "-o", str(executable)]
            run_command(native / "build", command, timeout=30)
            # A fresh process and immutable descriptor fixture isolate each case.
            for name, wire in files.items():
                run_file(executable, native / name, wire)
            # Each stream has its own child and independently held peer.
            for name, (fragments, close_write, trailing) in streams.items():
                run_stream(
                    executable, native / name, fragments,
                    close_write=close_write, trailing=trailing,
                )
        print(f"{layout}: identical LLVM and {2 * (len(files) + len(streams))} native cases passed")


def main():
    """Read the CLI path and the new artifact directory from arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--holo", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    verify(args.holo.resolve(), args.output.resolve())


# Importing the oracle for a separate check has no subprocess effects.
if __name__ == "__main__":
    main()
