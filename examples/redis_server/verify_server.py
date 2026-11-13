#!/usr/bin/env python3
"""Check a built Holo RESP2 server against an independent byte-and-dictionary oracle.

The connected cases use inherited Unix sockets and run without a TCP listener.
An optional loopback run exercises the installed redis-cli and verifies that
the Holo table survives successive clients. All generated inputs are bounded.
"""

import argparse
import errno
import json
import os
import socket
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path


# Mirror the public application limits, independently of its representation.
KEY_LIMIT = 32
VALUE_LIMIT = 128
TABLE_LIMIT = 16
TIMEOUT = 8


@dataclass(frozen=True)
class Case:
    """One complete connection's write fragments and expected reply bytes."""

    # A stable name used for assertion diagnostics and optional records.
    name: str
    # The exact writes, with an input half-close after the final fragment.
    pieces: tuple[bytes, ...]
    # All replies in wire order, including any terminal protocol error.
    expected: bytes


def bulk(value: bytes) -> bytes:
    """Encode one binary RESP bulk string without decoding its payload."""
    return b"$" + str(len(value)).encode("ascii") + b"\r\n" + value + b"\r\n"


def request(*parts: bytes) -> bytes:
    """Encode the given command and argument bytes as one RESP request array."""
    return b"*" + str(len(parts)).encode("ascii") + b"\r\n" + b"".join(map(bulk, parts))


def command_replies(commands: list[tuple[bytes, ...]]) -> bytes:
    """Evaluate complete valid fixture commands against a fresh Python dictionary."""
    entries: dict[bytes, bytes] = {}
    output = bytearray()
    # Commands arrive in wire order. Only accepted SET commands mutate the oracle.
    for command, key, *values in commands:
        if command.upper() == b"GET":
            # Dictionary membership distinguishes an empty value from a miss.
            value = entries.get(key)
            output.extend(b"$-1\r\n" if value is None else bulk(value))
        else:
            assert command.upper() == b"SET" and len(values) == 1
            value = values[0]
            assert len(key) <= KEY_LIMIT and len(value) <= VALUE_LIMIT
            # An existing key can still be overwritten when every slot is full.
            if key not in entries and len(entries) == TABLE_LIMIT:
                output.extend(b"-ERR 5\r\n")
            else:
                entries[key] = value
                output.extend(b"+OK\r\n")
    return bytes(output)


def fixtures() -> tuple[Case, ...]:
    """Build binary, capacity, fragmentation, and terminal failure witnesses."""
    key = b"\x00key\r\n\xff"
    value = b"\xffv\x00\r\n\x80"
    limit_key = bytes(range(KEY_LIMIT))
    limit_value = bytes(range(256 - VALUE_LIMIT, 256))
    commands = [
        (b"GET", b"missing"),
        (b"sEt", key, value),
        (b"gET", key),
        (b"SET", b"", b"x"),
        (b"GET", b""),
        (b"SET", key, b""),
        (b"GET", key),
        (b"SET", limit_key, limit_value),
        (b"GET", limit_key),
    ]
    wire = b"".join(request(*command) for command in commands)
    # The full pipeline exceeds one pending buffer. Responses also distinguish
    # overwritten long, empty, and missing values after input reuse.
    pipeline = commands + [(b"GET", limit_key)] * 20
    pipeline_wire = b"".join(request(*command) for command in pipeline)
    full = [
        (b"SET", bytes((index,)), bytes((255 - index,)))
        for index in range(TABLE_LIMIT)
    ] + [
        (b"SET", b"overflow", b"must not appear"),
        (b"SET", b"\x00", b"overwritten"),
        (b"GET", b"\x00"),
        (b"GET", b"overflow"),
        (b"GET", bytes((TABLE_LIMIT - 1,))),
    ]
    completed = [(b"SET", key, value), (b"GET", key)]
    completed_wire = b"".join(request(*command) for command in completed)
    completed_replies = command_replies(completed)
    # Failure cases specify independent expected results and send trailing work
    # in the same chunk when the malformed request should close the stream.
    return (
        Case("empty-stream", (), b""),
        Case("pipeline", (pipeline_wire,), command_replies(pipeline)),
        Case("one-byte-writes", tuple(bytes((byte,)) for byte in wire), command_replies(commands)),
        Case(
            "full-and-overwrite",
            (b"".join(request(*command) for command in full),),
            command_replies(full),
        ),
        Case("malformed-header", (b"+invalid\r\n" + request(b"GET", key),), b"-ERR 2\r\n"),
        Case("malformed-arity", (request(b"GET", key, b"extra"),), b"-ERR 2\r\n"),
        Case(
            "malformed-trailer",
            (completed_wire + request(b"SET", key, b"bad")[:-2] + b"\rX" + request(b"GET", key),),
            completed_replies + b"-ERR 2\r\n",
        ),
        Case(
            "oversized-key",
            (request(b"GET", b"x" * (KEY_LIMIT + 1)),),
            b"-ERR 3\r\n",
        ),
        Case(
            "oversized-overwrite",
            (completed_wire + request(b"SET", key, b"x" * (VALUE_LIMIT + 1)),),
            completed_replies + b"-ERR 3\r\n",
        ),
        Case(
            "numeric-overflow",
            (b"*2\r\n$3\r\nGET\r\n$18446744073709551616\r\n",),
            b"-ERR 4\r\n",
        ),
        Case(
            "truncated-eof",
            (completed_wire, request(b"SET", key, b"partial")[:-3]),
            completed_replies + b"-ERR 1\r\n",
        ),
    )


def write_record(directory: Path | None, files: dict[str, bytes]) -> None:
    """Retain one exact case in a fresh directory when recording is enabled."""
    # An omitted record keeps all fixture inputs in the verifier process.
    if directory is None:
        return
    directory.mkdir()
    # Fixed local names preserve bytes without shell quoting or decoding.
    for name, contents in files.items():
        (directory / name).write_bytes(contents)


def read_to_close(stream: socket.socket) -> bytes:
    """Read the finite reply until EOF or reset, preserving every received byte."""
    output = bytearray()
    # The peer closes at input EOF or after one terminal protocol error.
    while True:
        try:
            chunk = stream.recv(8192)
        except ConnectionResetError:
            # A terminal peer can close with unread request bytes. The exact
            # output comparison still requires every expected response.
            return bytes(output)
        if not chunk:
            return bytes(output)
        output.extend(chunk)


def exchange(stream: socket.socket, pieces: tuple[bytes, ...]) -> bytes:
    """Send the fixture, close only the write half, and collect all replies."""
    stream.settimeout(TIMEOUT)
    # Briefly separate successive writes to exercise the wait/refill path.
    # The kernel can coalesce them, so the oracle assumes no read boundaries.
    for index, piece in enumerate(pieces):
        if index:
            time.sleep(0.003)
        stream.sendall(piece)
    try:
        stream.shutdown(socket.SHUT_WR)
    except OSError as error:
        # A peer that already sent its terminal error may have closed first.
        if error.errno not in (errno.ENOTCONN, errno.ECONNRESET):
            raise
    return read_to_close(stream)


def connected_case(executable: Path, case: Case, record: Path | None) -> None:
    """Give one inherited endpoint to the executable and verify exact replies."""
    reader, writer = socket.socketpair()
    with reader, writer:
        reader.setblocking(False)
        command = [str(executable), "stream-fd", str(reader.fileno())]
        child = subprocess.Popen(
            command, pass_fds=(reader.fileno(),), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        reader.close()
        actual = b""
        transport_error = None
        try:
            actual = exchange(writer, case.pieces)
            stdout, stderr = child.communicate(timeout=TIMEOUT)
        except (OSError, subprocess.TimeoutExpired) as error:
            # Stop a stalled process before waiting. Preserve its output for diagnosis.
            transport_error = error
            child.kill()
            stdout, stderr = child.communicate(timeout=TIMEOUT)
        directory = record / case.name if record is not None else None
        write_record(directory, {
            "command.json": (json.dumps(command) + "\n").encode(),
            "input.bin": b"".join(case.pieces),
            "send-sizes.json": (json.dumps(list(map(len, case.pieces))) + "\n").encode(),
            "expected.bin": case.expected,
            "actual.bin": actual,
            "stdout": stdout,
            "stderr": stderr,
            "exit-status": f"{child.returncode}\n".encode(),
        })
        # Transport failure, nonzero exit, and protocol mismatch remain distinct.
        if transport_error is not None:
            raise AssertionError(f"{case.name}: {transport_error}") from transport_error
        assert child.returncode == 0, (case.name, child.returncode, stderr)
        assert not stdout, (case.name, stdout)
        assert actual == case.expected, (case.name, actual, case.expected)
    print(f"{case.name}: {len(actual)} exact reply bytes")


def reserve_port() -> int:
    """Find a free IPv4 loopback port for the immediately following listener launch."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


def wait_for_listener(child: subprocess.Popen, port: int) -> None:
    """Wait up to the test deadline for a working listener on the selected port."""
    deadline = time.monotonic() + TIMEOUT
    # A successful empty client confirms listen and exercises first-client EOF.
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError(f"listener exited with status {child.returncode}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                return
        except (ConnectionRefusedError, TimeoutError):
            time.sleep(0.02)
    raise TimeoutError(f"listener did not start on loopback port {port}")


def redis_command(
    base: list[str], arguments: list[str], expected: bytes, name: str,
    record: Path | None, env: dict[str, str], input_data: bytes = b"",
) -> None:
    """Run a real redis-cli invocation and compare its display output exactly."""
    command = base + arguments
    result = subprocess.run(
        command, input=input_data, capture_output=True, env=env, timeout=TIMEOUT,
    )
    directory = record / name if record is not None else None
    write_record(directory, {
        "command.json": (json.dumps(command) + "\n").encode(),
        "stdin": input_data,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "expected": expected,
        "exit-status": f"{result.returncode}\n".encode(),
    })
    result.check_returncode()
    assert result.stdout == expected, (name, result.stdout, result.stderr, expected)
    assert not result.stderr, (name, result.stderr)


def listener_cases(executable: Path, redis_cli: str, record: Path | None) -> None:
    """Exercise one loopback listener with real Redis sessions and a failed overwrite."""
    # Use only local anonymous connections and keep redis-cli's user state untouched.
    env = dict(os.environ)
    env.pop("REDISCLI_AUTH", None)
    env.update(REDISCLI_HISTFILE=os.devnull, REDISCLI_RCFILE=os.devnull)
    version = subprocess.check_output([redis_cli, "--version"], env=env, timeout=TIMEOUT)
    print(version.decode().strip())
    port = reserve_port()
    listener_command = [str(executable), "listen-loopback", str(port)]
    client = [redis_cli, "-2", "-h", "127.0.0.1", "-p", str(port)]
    child = subprocess.Popen(listener_command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_for_listener(child, port)
        redis_command(
            client, ["--raw"], b"OK\nfirst\n", "redis-one-session", record, env,
            b"SET demo first\nGET demo\n",
        )
        redis_command(
            client, ["--raw", "SET", "demo", "second"], b"OK\n", "redis-overwrite", record, env,
        )
        oversized = request(b"SET", b"demo", b"x" * (VALUE_LIMIT + 1))
        with socket.create_connection(("127.0.0.1", port), timeout=TIMEOUT) as stream:
            actual = exchange(stream, (oversized,))
        directory = record / "listener-rejected-overwrite" if record is not None else None
        write_record(directory, {
            "input.bin": oversized, "actual.bin": actual, "expected.bin": b"-ERR 3\r\n",
        })
        assert actual == b"-ERR 3\r\n", actual
        redis_command(
            client, ["--raw", "GET", "demo"], b"second\n", "redis-reconnect-hit", record, env,
        )
        redis_command(
            client, ["--no-raw", "GET", "missing"], b"(nil)\n", "redis-reconnect-miss", record, env,
        )
    finally:
        # The server normally remains in its accept loop after each client.
        if child.poll() is None:
            child.terminate()
        try:
            stdout, stderr = child.communicate(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            child.kill()
            stdout, stderr = child.communicate(timeout=TIMEOUT)
        directory = record / "listener" if record is not None else None
        write_record(directory, {
            "command.json": (json.dumps(listener_command) + "\n").encode(),
            "redis-cli-version": version,
            "stdout": stdout,
            "stderr": stderr,
            "exit-status": f"{child.returncode}\n".encode(),
        })
    print("redis-cli: one session, reconnect, rejected overwrite, and nil passed")


def main() -> None:
    """Parse the local verifier options, run every case, and report exact success."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path, help="built Holo RESP2 server")
    parser.add_argument("--record", type=Path, help="fresh directory for raw verification evidence")
    parser.add_argument(
        "--listener", action="store_true", help="also run redis-cli over loopback TCP",
    )
    parser.add_argument(
        "--redis-cli", default="redis-cli", help="redis-cli executable for --listener",
    )
    args = parser.parse_args()
    executable = args.executable.resolve(strict=True)
    # Existing evidence remains untouched. The root directory must be fresh.
    if args.record is not None:
        args.record.mkdir(parents=True)
    # Every inherited-stream case starts with a fresh server and empty table.
    for case in fixtures():
        connected_case(executable, case, args.record)
    if args.listener:
        listener_cases(executable, args.redis_cli, args.record)


if __name__ == "__main__":
    main()
