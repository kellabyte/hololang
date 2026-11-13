#!/usr/bin/env python3
"""Compare the maintained Holo request parser with a Python RESP2 oracle.

The generated package uses the parser declarations from the server source and
imports the maintained byte and decimal libraries. Its entry stages at most
512 initialized bytes, retries after actual bounded file reads,
and prints exact metadata and payload octets. Python owns only fixtures and
verification. Each invocation retains wire input, commands, and observations.
"""

import argparse
import json
import os
from pathlib import Path
import random
import re
import subprocess


SOURCE = Path(__file__).resolve().parent / "src" / "main.holo"
STDLIB = SOURCE.parents[3] / "lib" / "holo"
PARSER_BEGIN = "// Parse a bounded RESP2"
PARSER_END = "// The parser declarations above"
CAPACITY = 512
MORE = 1024
MALFORMED = 2048
LIMIT = 2049
OVERFLOW = 2050
DECIMAL = re.compile(rb"[0-9]+")
LAYOUTS = ("wide", "derived", "compact")


# The generated driver holds received bytes only during the refill arm and
# gives parsing a checked prefix of the accumulated initialized slot.
ENTRY = r"""
// Replay a file whose first byte supplies a test-only read bound. The
// following wire bytes accumulate in one fixed slot for exact observation.
// This driver holds one reusable metadata record and performs no request copies.
struct Wire {
    // Initialized slot used either for staging or a fresh native read.
    data Bytes
}

// Initialize one inline buffer in its receiving destination.
func empty_wire
    with noblock
    returns Wire
{
    return Wire { data: core.bytes(512, 0) }
}

// Return the exact logical extent of a live readonly receive view.
func receive_extent
    with noalloc, noblock
    arg input Bytes as readonly reference
    returns u64
{
    return core.len(input)
}

// Read one checked byte without retaining a reference.
func receive_byte
    with noalloc, noblock
    arg input Bytes as readonly reference
    arg index u64 as readonly copy
    returns u64
{
    return input[index]
}

// Retry after each read and drain complete requests before taking more bytes.
// Fixture payloads fit in one logical slot, so absolute offsets stay stable.
// Each reported result starts with seven decimal lines. Successful results
// then include exactly the key and value octets in wire order.
func main
    returns u64
{
    let wire_memory = core.region(2048) else { core.assert(false); return 0 }
    let wires = core.batch(wire_memory, Wire)
    let stage_status = core.append(wires, empty_wire())
    let read_status = core.append(wires, empty_wire())
    core.assert(stage_status == 0)
    core.assert(read_status == 0)
    let staged = core.borrow_readwrite(wires, 0)
    let incoming = core.borrow_readwrite(wires, 1)
    let request_memory = core.region(128) else { core.assert(false); return 0 }
    let requests = core.batch(request_memory, Request)
    let request_status = core.append(requests, empty_request())
    core.assert(request_status == 0)
    let request = core.borrow_readwrite(requests, 0)

    var quantum = 0
    // Every fixture starts with one positive read bound, outside the wire.
    refill incoming.data, 1 as size {
        quantum = receive_byte(size, 0)
    } else status {
        core.assert(false)
        return 0
    }
    core.assert(quantum > 0)

    var filled = 0
    var cursor = 0
    var ended = 0
    // A pass either consumes a whole request, extends the live prefix,
    // marks EOF, or terminates. Unfinished metadata is never observed as success.
    loop {
        var need_read = 1
        var result = 0
        if cursor == filled {
            // There is no incomplete request at a clean request boundary.
            if ended == 1 {
                return 0
            }
        } else {
            result = parse_request_status(request, core.prefix(staged.data, filled), cursor)
            // Complete or terminal outcomes can be reported immediately.
            // A short prefix is reported only when its source has ended.
            if result <= 512 {
                need_read = 0
            } else {
                if result >= 2048 {
                    need_read = 0
                }
                if ended == 1 {
                    need_read = 0
                }
            }
        }
        if need_read == 0 {
            core.print(result)
            // Map named commands to the replay protocol's numeric observations.
            if request.command == Command.Get {
                core.print(1)
            } else {
                if request.command == Command.Set {
                    core.print(2)
                } else {
                    core.assert(request.command == Command.Unset)
                    core.print(0)
                }
            }
            core.print(request.consumed)
            core.print(request.key_offset)
            core.print(request.key_length)
            core.print(request.value_offset)
            core.print(request.value_length)
            // Failure leaves the last successful metadata intact. Only an
            // accepted result authorizes reads through those offsets here.
            if result > 512 {
                return 0
            }
            for index in core.range(request.key_length) {
                core.print(staged.data[core.add_wrapping(request.key_offset, index)])
            }
            for index in core.range(request.value_length) {
                core.print(staged.data[core.add_wrapping(request.value_offset, index)])
            }
            cursor = core.add_wrapping(cursor, result)
        } else {
            let remaining = core.sub_wrapping(512, filled)
            var limit = quantum
            // Each read is bounded by both the fixture and the free suffix.
            if remaining < limit {
                limit = remaining
            }
            if limit == 0 {
                // Fixtures contain at most 512 wire bytes. A full slot is EOF.
                ended = 1
            } else {
                refill incoming.data, limit as chunk {
                    let received = receive_extent(chunk)
                    // Copy only initialized bytes in this positive receive view.
                    for index in core.range(received) {
                        staged.data[core.add_wrapping(filled, index)] = receive_byte(chunk, index)
                    }
                    filled = core.add_wrapping(filled, received)
                } else status {
                    // The independent corpus uses regular files, so only EOF is valid.
                    core.assert(status == 0)
                    ended = 1
                }
            }
        }
    }
}
"""


class ParseFailure(Exception):
    """Carry one observable status from an incomplete or rejected request."""

    def __init__(self, status):
        """Retain the exact witness status without modifying fixture state."""
        super().__init__(status)
        self.status = status


def unsigned_line(wire, position, marker, bound, over_bound):
    """Read a canonical header with regex boundaries and unbounded Python ints."""
    # Missing bytes extend the required prefix by one. An available wrong
    # byte is already terminal and requires no read beyond the supplied wire.
    if position == len(wire):
        raise ParseFailure(MORE + position + 1)
    if wire[position] != marker:
        raise ParseFailure(MALFORMED)
    digits = DECIMAL.match(wire, position + 1)
    if digits is None:
        if position + 1 == len(wire):
            raise ParseFailure(MORE + len(wire) + 1)
        raise ParseFailure(MALFORMED)
    spelling = digits.group()
    # Canonical spelling rejects leading zeroes before size classification.
    if len(spelling) > 1 and spelling.startswith(b"0"):
        raise ParseFailure(MALFORMED)
    value = int(spelling)
    if value > (1 << 64) - 1:
        raise ParseFailure(OVERFLOW)
    following = wire[digits.end():digits.end() + 2]
    # An observed delimiter mismatch is terminal. A matching short prefix
    # of CRLF retains the exact next logical byte required.
    if not b"\r\n".startswith(following):
        raise ParseFailure(MALFORMED)
    if len(following) != 2:
        raise ParseFailure(MORE + len(wire) + 1)
    if value > bound:
        raise ParseFailure(over_bound)
    return value, digits.end() + 2


def payload(wire, offset, length):
    """Return the exact bulk bytes and next offset after a checked trailer."""
    end = offset + length
    next_offset = end + 2
    observed = wire[end:next_offset]
    # The slice is empty if payload bytes are missing. A present wrong first
    # trailer byte still wins over an absent second trailer byte.
    if not b"\r\n".startswith(observed):
        raise ParseFailure(MALFORMED)
    if len(observed) != 2:
        raise ParseFailure(MORE + next_offset)
    return wire[offset:end], next_offset


def request(wire, start):
    """Decode one request into metadata and its independently sliced payloads."""
    arity, command_header = unsigned_line(wire, start, ord("*"), 3, MALFORMED)
    # Only GET and SET's array shapes are accepted.
    if arity < 2:
        raise ParseFailure(MALFORMED)
    name_length, name_offset = unsigned_line(wire, command_header, ord("$"), 3, MALFORMED)
    if name_length != 3:
        raise ParseFailure(MALFORMED)
    name, key_header = payload(wire, name_offset, name_length)
    if name.upper() != {2: b"GET", 3: b"SET"}[arity]:
        raise ParseFailure(MALFORMED)
    key_length, key_offset = unsigned_line(wire, key_header, ord("$"), 32, LIMIT)
    key, end = payload(wire, key_offset, key_length)
    value_length, value_offset = 0, 0
    value = b""
    # Only SET has a second binary argument. GET reports no value offset.
    if arity == 3:
        value_length, value_offset = unsigned_line(wire, end, ord("$"), 128, LIMIT)
        value, end = payload(wire, value_offset, value_length)
    return (arity - 1, end - start, key_offset, key_length, value_offset, value_length), key, value


def expected_output(wire):
    """Serialize oracle results and retain prior metadata on the first failure."""
    previous = (0,) * 6
    position = 0
    numbers = []
    # Each successful request advances by its complete array extent.
    while position < len(wire):
        try:
            fields, key, value = request(wire, position)
        except ParseFailure as failure:
            # Failure carries only a status and leaves the prior output record intact.
            numbers.extend((failure.status, *previous))
            break
        previous = fields
        position += fields[1]
        numbers.extend((fields[1], *fields, *key, *value))
    return "".join(f"{number}\n" for number in numbers).encode()


def encode_request(*arguments):
    """Encode one binary-safe fixture using ordinary RESP framing."""
    return b"*%d\r\n" % len(arguments) + b"".join(
        b"$%d\r\n" % len(argument) + argument + b"\r\n" for argument in arguments
    )


def fixtures():
    """Generate deterministic complete, incomplete, invalid, and pipelined input."""
    valid = (
        encode_request(b"gEt", b""),
        encode_request(b"GeT", b"x\0\r\n\xff" * 6 + b"xx"),
        encode_request(b"SeT", b"a\0b", b""),
        encode_request(b"sEt", b"\x01" * 32, bytes(range(128))),
    )
    cases = {"empty": (b"", 255)}
    # Test every nonempty proper prefix of these valid requests.
    for index, wire in enumerate(valid):
        cases[f"complete-{index}"] = (wire, 255)
        for size in range(1, len(wire)):
            cases[f"prefix-{index}-{size}"] = (wire[:size], 255)
    rng = random.Random(281)
    # Binary randomized inputs stay within the approved preview limits.
    for index in range(16):
        key = rng.randbytes(rng.randrange(33))
        value = rng.randbytes(rng.randrange(129))
        cases[f"random-{index}"] = (
            encode_request(b"SEt", key, value) + encode_request(b"get", key),
            255,
        )
    pipeline = b"".join(valid)
    # Read caps exercise real reassembly of headers and bodies across calls.
    for quantum in (1, 2, 7, 31, 255):
        cases[f"pipeline-{quantum}"] = (pipeline, quantum)
        cases[f"short-pipeline-{quantum}"] = (pipeline[:-1], quantum)
    command = b"*2\r\n$3\r\nGET\r\n"
    invalid = {
        "wrong-arity": b"*4\r\n",
        "wrong-command": b"*2\r\n$3\r\nset\r\n",
        "non-ascii-command": encode_request(b"\xc7ET", b"key"),
        "nul-in-command": encode_request(b"G\x00T", b"key"),
        "wrong-command-length": b"*2\r\n$4\r\nGETX\r\n",
        "leading-zero": b"*02\r\n",
        "no-digits": b"*\r\n",
        "line-feed-only": b"*2\n",
        "wrong-header-trailer": b"*2\rX",
        "wrong-payload-trailer": command + b"$1\r\na\rZ",
        "missing-key": command,
        "oversized-key": command + b"$33\r\n",
        "oversized-value": b"*3\r\n$3\r\nSET\r\n$0\r\n\r\n$129\r\n",
        "max-length": command + b"$18446744073709551615\r\n",
        "max-length-awaiting-cr": command + b"$18446744073709551615",
        "max-length-awaiting-lf": command + b"$18446744073709551615\r",
        "overflow-length": command + b"$18446744073709551616\r\n",
        "overflow-before-late-cr": command + b"$184467440737095516150X\r\n",
        "malformed-before-late-cr": command + b"$18446744073709551615X0\r\n",
        "overflow-array": b"*18446744073709551616\r\n",
        "negative-length": command + b"$-1\r\n",
        "pipeline-before-error": valid[3] + command + b"$1\r\naX",
    }
    # Truncate at an absolute offset of 512. The last request may require a
    # larger absolute extent even though its local size fits the wire budget.
    capacity_witness = valid[0] * (CAPACITY // len(valid[0])) + valid[3]
    cases["full-prefix"] = (capacity_witness[:CAPACITY], 31)
    cases.update({name: (wire, 7) for name, wire in invalid.items()})
    return cases


def build_driver(holo, output, manifest, layout, mode):
    """Build the checked fixture and save the complete compiler observation."""
    directory = output / layout / mode
    directory.mkdir(parents=True)
    executable = directory / "parser"
    command = [
        str(holo), "build", "--locked", "--strict-names", "--layout", layout,
        "-o", str(executable), "--package", str(manifest),
    ]
    # Keep the exact production compiler and switch only its optimization level.
    if mode == "o3":
        command.append("--release")
    result = subprocess.run(command, capture_output=True, timeout=60)
    (directory / "command.json").write_text(json.dumps(command) + "\n")
    (directory / "build.stdout").write_bytes(result.stdout)
    (directory / "build.stderr").write_bytes(result.stderr)
    result.check_returncode()
    return executable


def verify(holo, output):
    """Compile the real parser into a harness and compare every observation."""
    output.mkdir(parents=True)
    source = SOURCE.read_text()
    # Unique source markers prevent a stale or duplicate test parser.
    if source.count(PARSER_BEGIN) != 1 or source.count(PARSER_END) != 1:
        raise ValueError(f"{SOURCE}: expected unique parser extraction markers")
    start, end = source.index(PARSER_BEGIN), source.index(PARSER_END)
    if start >= end:
        raise ValueError(f"{SOURCE}: parser extraction markers are out of order")
    package = output / "driver"
    sources = package / "src"
    sources.mkdir(parents=True)
    (sources / "main.holo").write_text(
        "using holo.bytes\nusing holo.decimal\n\n" + source[start:end] + ENTRY
    )
    manifest = package / "holo.toml"
    stdlib = json.dumps(os.path.relpath(STDLIB, package), ensure_ascii=False)
    manifest.write_text(
        '[package]\nname = "resp_parser_replay"\nversion = "0.1.0"\n'
        'source = "src"\nentry = "main.holo"\n\n[dependencies]\n'
        f'holo = {{ path = {stdlib}, version = "^0.1" }}\n'
    )
    subprocess.run([str(holo), "lock", "--package", str(manifest)], check=True, timeout=60)
    wires = output / "inputs"
    wires.mkdir()
    cases = {}
    # Keep the exact read-bound byte and original payload for each replay.
    for name, (wire, quantum) in fixtures().items():
        tape = wires / f"{name}.bin"
        tape.write_bytes(bytes((quantum,)) + wire)
        cases[name] = (tape, expected_output(wire))
    total = 0
    # Replay the identical corpus with all supported physical layouts and levels.
    for layout in LAYOUTS:
        for mode in ("o0", "o3"):
            executable = build_driver(holo, output, manifest, layout, mode)
            with (executable.parent / "observations.jsonl").open("w") as log:
                # Record raw stdout/stderr before rejecting any differing result.
                for name, (tape, expected) in cases.items():
                    command = [str(executable), "file", str(tape)]
                    child = subprocess.run(command, capture_output=True, timeout=10)
                    log.write(json.dumps({
                        "case": name,
                        "command": command,
                        "status": child.returncode,
                        "stdout": child.stdout.decode(errors="backslashreplace"),
                        "stderr": child.stderr.decode(errors="backslashreplace"),
                        "expected": expected.decode(),
                    }) + "\n")
                    if child.returncode != 0 or child.stdout != expected or child.stderr:
                        raise ValueError(f"{layout}/{mode}/{name}: observation differs from oracle")
                    total += 1
            print(f"{layout}/{mode}: {len(cases)} exact request observations", flush=True)
    print(f"{total} native parser checks passed")


def main():
    """Read the existing Holo CLI path and a new evidence working directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--holo", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    verify(args.holo.resolve(strict=True), args.output.resolve())


if __name__ == "__main__":
    main()
