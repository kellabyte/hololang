//! Hosted adapter syscall witnesses and checked-source native boundary tests.
//!
//! The deterministic adapter fixtures substitute read, receive, send, and poll
//! outcomes. Real sockets check the output path and its disconnect policy.
//! Source tests use the production driver and host to check batch publication
//! and named or compiler-selected scoped construction from received views.
//! Separate builds inject scalar adapter results to prove the boundary guards fail closed.
//! Temporary outputs and test probes are outside the normal compiler path.

#[path = "../support/hosted_stream.rs"]
mod hosted_stream;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::layout::LayoutMode;
use hosted_stream::read_stream;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// One parsed slot and one checked attempt. The helper reads only a byte formal.
const SOURCE: &str = r#"
// A reusable caller-owned input slot with known initialized capacity.
struct Slot { wire Bytes }
// Construct the inline slot at its final destination with a poisoned suffix.
func slot with noblock returns Slot { return Slot { wire: core.bytes(4, 99) } }
// Report precisely how many fresh bytes reached the ordinary helper.
func extent arg input Bytes as readonly reference returns u64 { return core.len(input) }
// Own the slot for one native attempt. Display the fresh extent or copied status.
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let items = core.batch(memory, Slot)
    let made = core.append(items, slot())
    core.assert(made == 0)
    let buffer = core.borrow_readwrite(items, 0)
    refill buffer.wire, 2 as input { core.print(extent(input)) } else status { core.print(status) }
    return 0
}
"#;

/// A live readonly payload and explicit suffix bound for one hosted send.
const SEND_SOURCE: &str = r#"
// Store the wire bytes inline, including zero and a byte outside ASCII.
struct Reply { wire Bytes }
// Build the output directly in its owned region before lending it to send.
func reply with noblock returns Reply {
    return Reply { wire: core.bytes(5, core.sub_wrapping(core.index(), 2)) }
}
// Wait once and then send a proper subrange without a temporary buffer.
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let replies = core.batch(memory, Reply)
    let made = core.append(replies, reply())
    core.assert(made == 0)
    let prepared = core.borrow_readonly(replies, 0)
    let ready = core.wait_output()
    let sent = core.send(prepared.wire, 1, 3)
    core.print(ready)
    core.print(sent)
    return 0
}
"#;

/// Test two successive clients, preserving ordinary Holo state and resetting
/// the receive capability at each iteration of the explicit session loop.
const SESSION_SOURCE: &str = r#"
// The input and reply are disjoint fields in one already initialized record.
struct SessionSlot { input Bytes, reply Bytes }
// Construct the reusable slot once in its region.
func session_slot with noblock returns SessionSlot {
    return SessionSlot { input: core.bytes(1, 0), reply: core.bytes(2, 0) }
}
// Keep the region through both clients while giving each read its own view.
func main returns u64 {
    let memory = core.region(128) else { return 0 }
    let slots = core.batch(memory, SessionSlot)
    let added = core.append(slots, session_slot())
    core.assert(added == 0)
    let slot = core.borrow_readwrite(slots, 0)
    for round in core.range(2) {
        let readable = core.wait_input()
        core.assert(readable == 0)
        refill slot.input, 1 as received {
            slot.reply[0] = slot.input[0]
            slot.reply[1] = round
            let writable = core.wait_output()
            core.assert(writable == 0)
            let sent = core.send(slot.reply, 0, 2)
            core.assert(sent == 2)
        } else status {
            core.assert(false)
        }
        if round == 0 {
            let next = core.next_session()
            core.assert(next == 0)
        }
    }
    return 0
}
"#;

/// One complete frame followed by a final attempt missing two payload bytes.
/// The second attempt overwrites the first payload position in the reused slot.
const TYPED_FRAME_WIRE: &[u8] = &[0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 3, 0xDD];

/// Run a finite build/setup command, returning output for assertions or diagnostics.
fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{command:?}: {output:?}");
    output
}

/// Require the platform's LLVM trap signal and no output from an outcome body.
fn assert_trap(output: &Output) {
    // AArch64 uses BRK and x86-64 uses UD2 for the generated target trap.
    let signal = if cfg!(target_arch = "aarch64") { 5 } else { 4 };
    assert_eq!(output.status.signal(), Some(signal), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

/// Compile the real adapter against deterministic syscall witnesses. Both the
/// direct-write call shape and all nondata outcomes are checked by the C fixture.
#[test]
fn native_adapter_issues_one_call_and_checks_outcomes() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let runtime = scratch.path().join("refill");
    fs::create_dir(&runtime).unwrap();
    fs::write(
        runtime.join("refill.h"),
        include_str!("../../runtime/refill/refill.h"),
    )
    .unwrap();
    let adapter = runtime.join("refill.c");
    fs::write(&adapter, include_str!("../../runtime/refill/refill.c")).unwrap();
    let probe = scratch.path().join("probe.c");
    fs::write(&probe, include_str!("../support/refill_adapter.c")).unwrap();
    let binary = scratch.path().join("probe");
    let mut compile = Command::new(&tools.clang);
    compile.args([
        "-target",
        tools.target_triple(),
        "-std=c11",
        "-O3",
        "-Wall",
        "-Wextra",
        "-Werror",
    ]);
    // The SDK must be selected on Darwin, matching the production native driver.
    if cfg!(target_os = "macos") {
        let sdk = success(Command::new("xcrun").arg("--show-sdk-path"));
        compile
            .arg("-isysroot")
            .arg(String::from_utf8(sdk.stdout).unwrap().trim());
    }
    success(
        compile
            .args([
                "-Dread=holo_test_read",
                "-Drecv=holo_test_recv",
                "-Dpoll=holo_test_poll",
                "-I",
            ])
            .arg(&runtime)
            .arg(&adapter)
            .arg(&probe)
            .arg("-o")
            .arg(&binary),
    );
    success(&mut Command::new(&binary));
    // Inconsistent native responses cannot expose bytes or reach status handling.
    for invalid in ["oversize", "negative", "bad-errno"] {
        assert_trap(&Command::new(&binary).arg(invalid).output().unwrap());
    }
}

/// Compile the real send/poll adapter against deterministic and socketpair
/// witnesses. The controlled cases count syscalls and the real peer closure
/// verifies that a send can return EPIPE without terminating the process.
#[test]
fn native_send_adapter_returns_partial_progress_and_disconnect() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let runtime = scratch.path();
    fs::write(
        runtime.join("refill.h"),
        include_str!("../../runtime/refill/refill.h"),
    )
    .unwrap();
    let adapter = runtime.join("send.c");
    fs::write(&adapter, include_str!("../../runtime/refill/send.c")).unwrap();
    let probe = runtime.join("probe.c");
    fs::write(&probe, include_str!("../support/send_adapter.c")).unwrap();
    let binary = runtime.join("probe");
    let mut compile = Command::new(&tools.clang);
    compile.args([
        "-target",
        tools.target_triple(),
        "-std=c11",
        "-O3",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Dsend=holo_test_send",
        "-Dpoll=holo_test_poll",
    ]);
    // SDK selection matches the compiler driver's Darwin target.
    if cfg!(target_os = "macos") {
        let sdk = success(Command::new("xcrun").arg("--show-sdk-path"));
        compile
            .arg("-isysroot")
            .arg(String::from_utf8(sdk.stdout).unwrap().trim());
    }
    success(compile.arg(&adapter).arg(&probe).arg("-o").arg(&binary));
    success(&mut Command::new(&binary));
    // Separate processes make the contradictions observable as native traps.
    for invalid in ["oversize", "zero", "negative", "bad-errno"] {
        assert_trap(&Command::new(&binary).arg(invalid).output().unwrap());
    }
}

/// Pass a real connected nonblocking descriptor to the production host.
/// Return the child output and exact received bytes. The short timeout
/// bounds a runtime or test bug without altering the admitted descriptor.
fn receive_sent_bytes(binary: &std::path::Path) -> (Output, Vec<u8>) {
    let (endpoint, mut peer) = UnixStream::pair().unwrap();
    endpoint.set_nonblocking(true).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let inherited: OwnedFd = endpoint.into();
    let mut child = Command::new("sh")
        .args(["-c", "exec 3<&0; exec \"$1\" stream-fd 3", "typed-send"])
        .arg(binary)
        .stdin(Stdio::from(inherited))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut payload = Vec::new();
    // A broken send or wait must not hang the test or leave its child alive.
    if let Err(error) = peer.read_to_end(&mut payload) {
        child.kill().unwrap();
        let output = child.wait_with_output().unwrap();
        panic!("socket receive failed: {error}: {output:?}");
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    (output, payload)
}

/// The accepted program passes the same binary payload directly at O0, O3,
/// and under ASan. A regular-file host reports the encoded native error
/// without replacing the file input or writing to standard output as a socket.
#[test]
fn hosted_send_writes_exact_suffix_at_native_boundary() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let module = llvm(SEND_SOURCE, &tools);
    assert_eq!(
        module.matches("call i64 @holo_send_experimental(").count(),
        1
    );
    assert!(!module.contains("@llvm.memcpy"));
    let file = scratch.path().join("file");
    fs::write(&file, [42]).unwrap();
    // The driver stages the optional output adapter in every checked build mode.
    for mode in ["o0", "o3", "asan"] {
        let binary = scratch.path().join(mode);
        if mode == "asan" {
            tools.build_sanitized(&module, &binary).unwrap();
        } else {
            tools.build(&module, &binary, mode == "o3").unwrap();
        }
        let (output, payload) = receive_sent_bytes(&binary);
        assert_eq!(output.stdout, b"0\n3\n");
        assert_eq!(payload, [255, 0, 1]);
        let output = success(Command::new(&binary).arg("file").arg(&file));
        let codes: Vec<u64> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0], codes[1]);
        assert!(codes[0] > (1_u64 << 63) && codes[0] <= (1_u64 << 63) | (i32::MAX as u64));
        assert_eq!(fs::read(&file).unwrap(), [42]);
    }
}

/// Invalid source shape or an unsupported owner is rejected before emission.
/// A checked field still needs the runtime range guard for all scalar operands.
#[test]
fn hosted_send_rejects_unchecked_source_and_ranges() {
    // Neither an unsigned scalar nor a mutable status may bypass source checking.
    for (source, reason) in [
        (SEND_SOURCE.replace("prepared.wire, 1, 3", "3, 1, 3"), "binding name"),
        (SEND_SOURCE.replace("let sent =", "var sent ="), "with let"),
        (
            "func invalid with noblock arg bytes Bytes as readonly reference returns u64 { let wait = core.wait_output(); return wait }\nfunc main returns u64 { return 0 }".into(),
            "unavailable",
        ),
    ] {
        let error = parse_experimental(&source).unwrap_err();
        assert!(error.message.contains(reason), "{}", error.message);
    }

    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let file = scratch.path().join("file");
    fs::write(&file, [42]).unwrap();
    // Invalid offset, zero count, and overrun must trap even in file mode.
    for (index, operands) in ["18446744073709551615, 1", "1, 0", "4, 2"]
        .iter()
        .enumerate()
    {
        let source =
            SEND_SOURCE.replace("prepared.wire, 1, 3", &format!("prepared.wire, {operands}"));
        let binary = scratch.path().join(index.to_string());
        tools.build(&llvm(&source, &tools), &binary, true).unwrap();
        let actual = Command::new(&binary)
            .arg("file")
            .arg(&file)
            .output()
            .unwrap();
        assert_trap(&actual);
    }
}

/// Connect to the test-owned loopback endpoint after child startup. Bound the
/// retries and stop early if the listener exits. Both tests and production
/// own only their own sockets.
fn connect_listener(address: SocketAddrV4, child: &mut Child) -> io::Result<TcpStream> {
    let start = Instant::now();
    // Failed attempts are local startup probes, never retries in production.
    loop {
        match TcpStream::connect(address) {
            Ok(client) => return Ok(client),
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                if start.elapsed() >= Duration::from_secs(3) || child.try_wait()?.is_some() {
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Read one response through EOF so next_session must release the old client
/// before this test connects the next one. Timeouts prevent a stuck socket
/// from delaying test cleanup.
fn listener_round(address: SocketAddrV4, child: &mut Child, byte: u8) -> io::Result<Vec<u8>> {
    let mut client = connect_listener(address, child)?;
    client.set_read_timeout(Some(Duration::from_secs(3)))?;
    client.set_write_timeout(Some(Duration::from_secs(3)))?;
    client.write_all(&[byte])?;
    let mut reply = Vec::new();
    client.read_to_end(&mut reply)?;
    Ok(reply)
}

/// Check the complete native loopback admission, refill, send and replacement
/// path with two independent TCP clients. Application state in the ordinary
/// Holo region remains live across the host session transition.
#[test]
#[ignore = "requires permission to bind an IPv4 loopback TCP socket"]
fn hosted_listener_replaces_client_and_keeps_holo_state() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let binary = scratch.path().join("listener");
    tools
        .build(&llvm(SESSION_SOURCE, &tools), &binary, true)
        .unwrap();
    // Port validation rejects malformed values before any native bind or accept.
    for invalid in ["", "0", "65536", "-1", " 6321"] {
        let mut command = Command::new(&binary);
        command.args(["listen-loopback", invalid]);
        assert_rejected(command);
    }
    let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let std::net::SocketAddr::V4(address) = reserved.local_addr().unwrap() else {
        unreachable!("the reserved loopback port is IPv4");
    };
    drop(reserved);
    let mut child = Command::new(&binary)
        .arg("listen-loopback")
        .arg(address.port().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let results = listener_round(address, &mut child, 255)
        .and_then(|first| listener_round(address, &mut child, 0).map(|second| (first, second)));
    // Reap a failed listener even if it is blocked waiting for a missing peer.
    if results.is_err() {
        let _ = child.kill();
    }
    let output = child.wait_with_output().unwrap();
    let (first, second) = results.unwrap_or_else(|error| panic!("{error}: {output:?}"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(first, [255, 0]);
    assert_eq!(second, [0, 1]);
}

/// The hosted checker requires the old receive continuation to end before
/// replacing its descriptor. A one-descriptor invocation reports End.
#[test]
fn hosted_next_session_rejects_live_view_and_reports_single_descriptor_end() {
    let invalid = SESSION_SOURCE.replace(
        "let writable = core.wait_output()",
        "let abandoned = core.next_session(); let writable = core.wait_output()",
    );
    let parsed = parse_experimental(&invalid).unwrap();
    let error = parsed.check().unwrap_err();
    assert!(error.message.contains("received view is live"), "{error:?}");

    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let binary = scratch.path().join("one-session");
    let source =
        "func main returns u64 { let ended = core.next_session(); core.print(ended); return 0 }";
    tools.build(&llvm(source, &tools), &binary, true).unwrap();
    let file = scratch.path().join("file");
    fs::write(&file, []).unwrap();
    assert_eq!(
        success(Command::new(&binary).arg("file").arg(&file)).stdout,
        b"1\n"
    );
    assert_eq!(read_stream(&binary, &[], false).stdout, b"1\n");
}

/// Parse and check the fixture, then emit the production compact layout for this host.
fn llvm(source: &str, tools: &NativeToolchain) -> String {
    let parsed = parse_experimental(source).unwrap();
    let checked = parsed.check().unwrap();
    holo::llvm::emit_with_layout(&checked, tools.target_triple(), LayoutMode::Compact32).unwrap()
}

/// Run a setup expected to exit without reaching Holo. Bound waiting so a FIFO
/// admission regression cannot hang the suite, and reap the child if it fails.
fn assert_rejected(mut command: Command) {
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    // Poll only the test process. The production host must reject within the deadline.
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(2), "{command:?}: {status}");
            break;
        }
        // A hung setup is cleaned up before the test reports failure.
        if start.elapsed() > Duration::from_secs(3) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("host setup hung: {command:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Build parsed source through the real ASan driver, verify a short read, and
/// reject malformed invocation, a directory, and an unopened FIFO.
#[test]
fn hosted_entry_admission_and_short_file_are_address_sanitized() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let ir = llvm(SOURCE, &tools);
    assert_eq!(ir.matches("call i64 @holo_refill_experimental(").count(), 1);
    assert!(!ir.contains("@llvm.memcpy"));
    let binary = scratch.path().join("hosted");
    tools.build_sanitized(&ir, &binary).unwrap();
    let file = scratch.path().join("one");
    fs::write(&file, [13]).unwrap();
    let actual = success(Command::new(&binary).args(["file"]).arg(&file));
    assert_eq!(actual.stdout, b"1\n");
    let fifo = scratch.path().join("pipe");
    success(Command::new("mkfifo").arg(&fifo));
    // Each supplied path must fail admission before the Holo body can read.
    for path in [fifo.as_path(), scratch.path()] {
        let mut command = Command::new(&binary);
        command.arg("file").arg(path);
        assert_rejected(command);
    }
    // Reject malformed and standard-stream descriptor selectors at host setup.
    for text in ["", "-3", "2", " 3", "2147483648", "99999"] {
        let mut command = Command::new(&binary);
        command.args(["stream-fd", text]);
        assert_rejected(command);
    }
}

/// Compile the runnable typed frame example across layouts and native modes.
/// Real files and sockets check exact extents, rollback, owned payloads and
/// untouched nondata status through the same production host and adapter.
#[test]
fn typed_frame_append_uses_fresh_refill_extent() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let parsed = parse_experimental(include_str!("../fixtures/typed_refill_frame.holo")).unwrap();
    let checked = parsed.check().unwrap();
    let wire = scratch.path().join("typed-wire");
    let empty = scratch.path().join("empty-wire");
    fs::write(&wire, TYPED_FRAME_WIRE).unwrap();
    fs::write(&empty, []).unwrap();
    // Exercise each physical length/header policy with identical native input.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let ir = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        assert_eq!(ir.matches("call i64 @holo_refill_experimental(").count(), 3);
        // Unoptimized, optimized and instrumented builds consume the same checked IR.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch.path().join(format!("{}-{mode}", layout.name()));
            // Only the instrumented build adds sanitizer code to the native body.
            if mode == "asan" {
                tools.build_sanitized(&ir, &binary).unwrap();
            } else {
                tools.build(&ir, &binary, mode == "o3").unwrap();
            }
            let complete = b"7\n6\n0\n204\n";
            assert_eq!(
                success(Command::new(&binary).arg("file").arg(&wire)).stdout,
                complete
            );
            assert_eq!(
                read_stream(&binary, TYPED_FRAME_WIRE, true).stdout,
                complete
            );
            assert_eq!(
                success(Command::new(&binary).arg("file").arg(&empty)).stdout,
                b"0\n"
            );
            assert_eq!(
                read_stream(&binary, &[], false).stdout,
                format!("{}\n", u64::MAX - 1).as_bytes()
            );
        }
    }
}

/// Shared producers for checked scoped construction from a hosted data view.
/// The parent writes its channel before the child can reject on input extent.
const SCOPED_REFILL_DEFINITIONS: &str = r#"
// A six-byte initialized wire slot can retain stale bytes after a short read.
struct Slot { wire Bytes }
struct Frame { sequence u64, payload Bytes }
struct Received { channel u64, frame Frame }
error FrameError { Truncated(received u64) }
error ReceiveError { from FrameError as Decode }

// Acquire the reusable input slot once, before the native reader runs.
func wire_slot returns Slot {
    return Slot { wire: core.bytes(6, 0xEE) }
}

// The caller chooses either a complete three-byte payload or a one-byte
// preview. Reject before reading past the received extent in either case.
func decode arg input Bytes as readonly reference arg payload_size u64 as readonly copy
    returns Result<Frame, FrameError> {
    // This fixture passes only one or three, so the required count fits u64.
    if core.len(input) < core.add_wrapping(3, payload_size) {
        return error FrameError.Truncated(core.len(input))
    }
    return ok Frame:
        sequence = input[1]
        payload = core.bytes(payload_size, input[core.add_wrapping(3, core.index())])
}

// Write the channel before calling the child in its final nested destination.
// A failed child leaves this private write for the enclosing rollback.
func receive arg input Bytes as readonly reference arg payload_size u64 as readonly copy
    returns Result<Received, ReceiveError> {
    return ok Received:
        channel = 1
        frame = try decode(input, payload_size)
}
"#;

/// Lend the actual native count to a scoped producer, including a short read
/// whose requested extent matches the six-byte slot. A nested rejection must
/// restore the tail before a one-byte preview retries through the same view.
#[test]
fn scoped_frame_construct_retries_after_a_short_refill() {
    let source = [
        SCOPED_REFILL_DEFINITIONS,
        r#"
// Read a complete frame, a short final frame, then EOF. Each outcome ends
// before another refill reuses the slot. Allocation failures fail the fixture.
func main returns u64 {
    let slot_memory = core.region(64) else { core.assert(false); return 0 }
    let slots = core.batch(slot_memory, Slot)
    let initialized = core.append(slots, wire_slot())
    core.assert(initialized == 0)
    let slot = core.borrow_readwrite(slots, 0)
    let memory = core.region(64) else { core.assert(false); return 0 }
    let start = core.used(memory)
    var first_address = 0

    // Mutate the source byte while the owned result's borrow remains usable.
    refill slot.wire, 6 as input {
        core.construct(memory, receive(input, 3))
            on ok(record) then {
                core.assert(core.used(memory) > start)
                first_address = core.address(record)
                slot.wire[3] = 0xFE
                core.assert(record.channel == 1)
                core.assert(core.len(record.frame.payload) == 3)
                core.print(record.frame.sequence)
                core.print(record.frame.payload[0])
            }
            on error then { core.assert(false) }
    } else status { core.assert(false) }
    core.assert(core.used(memory) == start)

    // The read requests six but gets four. Its two stale backing bytes are
    // initialized and accessible through the slot, outside the fresh view.
    refill slot.wire, 6 as input {
        core.assert(slot.wire[4] == 0xBB)
        core.assert(slot.wire[5] == 0xCC)
        core.construct(memory, receive(input, 3))
            on ok then { core.assert(false) }
            on Rejected Decode.Truncated(received) then {
                core.assert(core.used(memory) == start)
                core.print(received)
                // The preview is valid with four bytes. The retry owns the
                // same tail, and no stale byte becomes part of its payload.
                core.construct(memory, receive(input, 1))
                    on ok(preview) then {
                        core.assert(core.address(preview) == first_address)
                        core.assert(core.used(memory) > start)
                        core.assert(preview.channel == 1)
                        core.assert(core.len(preview.frame.payload) == 1)
                        core.print(preview.frame.sequence)
                        core.print(preview.frame.payload[0])
                    }
                    on error then { core.assert(false) }
                core.assert(core.used(memory) == start)
            }
            on error then { core.assert(false) }
    } else status { core.assert(false) }
    core.assert(core.used(memory) == start)

    // The exhausted descriptor runs only the nondata continuation.
    refill slot.wire, 6 as input {
        core.assert(false)
    } else status {
        core.assert(status == 0)
        core.print(status)
    }
    return 0
}
"#,
    ]
    .concat();
    let parsed = parse_experimental(&source).unwrap();
    let checked = parsed.check().unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let wire = scratch.path().join("scoped-wire");
    fs::write(&wire, TYPED_FRAME_WIRE).unwrap();
    // The wire supplies sequence 7 and 0xAA, a four-byte short count, sequence
    // 8 and 0xDD in the preview, then End. No expected value depends on layout.
    let expected = b"7\n170\n4\n8\n221\n0\n";
    // Exercise both marker-based and deferred cursor producer conventions.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let ir = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        // Run the same checked fixture with and without optimization and ASan.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch
                .path()
                .join(format!("scoped-{}-{mode}", layout.name()));
            // The production driver adds instrumentation only for the ASan case.
            if mode == "asan" {
                tools.build_sanitized(&ir, &binary).unwrap();
            } else {
                tools.build(&ir, &binary, mode == "o3").unwrap();
            }
            let file = success(Command::new(&binary).arg("file").arg(&wire));
            let stream = read_stream(&binary, TYPED_FRAME_WIRE, true);
            assert_eq!(file.stdout, expected, "{layout:?} {mode} file");
            assert_eq!(stream.stdout, expected, "{layout:?} {mode} stream");
        }
    }
}

/// The same inferred site accepts the checked byte borrow on every live
/// refill, including a short view whose stale tail remains initialized.
/// Matched explicit scoping checks the hosted LLVM before native execution.
#[test]
fn automatic_scoped_placement_borrows_fresh_refill_data() {
    let prefix = [
        SCOPED_REFILL_DEFINITIONS,
        r#"
func main returns u64 {
    let slot_memory = core.region(64) else { core.assert(false); return 0 }
    let slots = core.batch(slot_memory, Slot)
    let initialized = core.append(slots, wire_slot())
    core.assert(initialized == 0)
    let slot = core.borrow_readwrite(slots, 0)
"#,
    ]
    .concat();
    let body = r#"
        var first_address = 0
        // One static construction site gets a fresh borrowed view on each visit.
        for attempt in core.range(2) {
            refill slot.wire, 6 as input {
                core.construct(receive(input, 3))
                    on ok(record) then {
                        if attempt == 0 {
                            first_address = core.address(record)
                        } else {
                            core.assert(core.address(record) == first_address)
                        }
                        // The record owns its bytes even while the caller reuses input.
                        slot.wire[3] = 0xFE
                        core.print(record.frame.sequence)
                        core.print(record.frame.payload[0])
                    }
                    on Rejected Decode.Truncated(received) then { core.print(received) }
                    on error then { core.assert(false) }
            } else status { core.assert(false) }
        }
"#;
    let continuation = r#"
    // The region has ended before the outer slot observes stream EOF.
    refill slot.wire, 6 as input {
        core.assert(false)
    } else status {
        core.assert(status == 0)
        core.print(status)
    }
    return 0
}
"#;
    let automatic = format!(
        "{prefix}    with core.region(64) {{{body}    }} else {{ core.assert(false) }}{continuation}"
    );
    let explicit_body = body.replace("core.construct(receive(", "core.construct(memory, receive(");
    let explicit = format!(
        "{prefix}    {{\n        let memory = core.region(64) else {{ core.assert(false) }}{explicit_body}    }}{continuation}"
    );
    let automatic = parse_experimental(&automatic).unwrap();
    let explicit = parse_experimental(&explicit).unwrap();
    let automatic = automatic.check().unwrap();
    let explicit = explicit.check().unwrap();
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let complete_wire = [0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 3, 0xDD, 0xEE, 0xFF];
    let cases = [
        ("short", TYPED_FRAME_WIRE, &b"7\n170\n4\n0\n"[..]),
        (
            "complete",
            complete_wire.as_slice(),
            &b"7\n170\n8\n221\n0\n"[..],
        ),
    ];
    // Both wire shapes stay inside the deterministic six-byte read request.
    for (label, wire, _) in cases {
        fs::write(scratch.path().join(label), wire).unwrap();
    }
    // The backends share the borrowed-input lowering but use different layouts.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let emit = |checked| holo::llvm::emit_with_layout(checked, tools.target_triple(), layout);
        let module = emit(&automatic).unwrap();
        assert_eq!(module, emit(&explicit).unwrap(), "{layout:?}");
        // Native builds consume the common checked module once per configuration.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch
                .path()
                .join(format!("placed-{}-{mode}", layout.name()));
            // ASan validates the same reads with instrumentation.
            if mode == "asan" {
                tools.build_sanitized(&module, &binary).unwrap();
            } else {
                tools.build(&module, &binary, mode == "o3").unwrap();
            }
            // Compare independent exact bytes for files and real Unix streams.
            for (label, wire, expected) in cases {
                let file = success(
                    Command::new(&binary)
                        .arg("file")
                        .arg(scratch.path().join(label)),
                );
                let stream = read_stream(&binary, wire, true);
                assert_eq!(file.stdout, expected, "{layout:?} {mode} {label} file");
                assert_eq!(stream.stdout, expected, "{layout:?} {mode} {label} stream");
            }
        }
    }
}

/// Assemble several frames from reads of at most two bytes. Files and real
/// nonblocking sockets must publish the same records at every physical layout.
/// Independent expected bytes cover empty bodies, truncated headers and bodies,
/// bad magic, EOF, and a stream whose following frame must remain unread.
#[test]
fn typed_stream_reader_assembles_frames_across_refills() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let source = include_str!("../fixtures/typed_stream_reader.holo");
    let parsed = parse_experimental(source).unwrap();
    let checked = parsed.check().unwrap();
    // A frame prints sequence and payload sum. The last three lines are the
    // terminal status, published frame count and partial frame's required extent.
    let cases: &[(&[u8], &[u8])] = &[
        (
            &[0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 0, 0x48, 9, 2, 1, 2],
            b"7\n561\n8\n0\n9\n3\n0\n3\n0\n",
        ),
        (
            &[0x48, 7, 3, 0xAA, 0xBB, 0xCC, 0x48, 8, 3, 0xDD],
            b"7\n561\n1\n1\n6\n",
        ),
        (&[0x48, 7], b"1\n0\n3\n"),
        (&[0x00, 7, 0, 0x48, 8, 0], b"2\n0\n0\n"),
        (&[], b"0\n0\n0\n"),
    ];
    let wire_file = scratch.path().join("incremental-wire");
    // Force all physical layout policies to make the same publication decisions.
    for layout in [
        LayoutMode::Wide,
        LayoutMode::Derived64,
        LayoutMode::Compact32,
    ] {
        let ir = holo::llvm::emit_with_layout(&checked, tools.target_triple(), layout).unwrap();
        // Each code generation mode consumes the identical checked program.
        for mode in ["o0", "o3", "asan"] {
            let binary = scratch
                .path()
                .join(format!("stream-{}-{mode}", layout.name()));
            // Sanitized builds instrument the same refill and destination path.
            if mode == "asan" {
                tools.build_sanitized(&ir, &binary).unwrap();
            } else {
                tools.build(&ir, &binary, mode == "o3").unwrap();
            }
            // Each fresh process starts with no published frames. Boundaries
            // split the header and payload even when all socket bytes are queued.
            for &(wire, expected) in cases {
                fs::write(&wire_file, wire).unwrap();
                assert_eq!(
                    success(Command::new(&binary).arg("file").arg(&wire_file)).stdout,
                    expected,
                    "{layout:?} {mode} file {wire:02x?}"
                );
                assert_eq!(
                    read_stream(&binary, wire, true).stdout,
                    expected,
                    "{layout:?} {mode} stream {wire:02x?}"
                );
            }
        }
    }
}

/// Force impossible adapter outcomes after a valid file admission. No Data or
/// nondata continuation may execute before the LLVM boundary validates them.
#[test]
fn emitted_guards_reject_untrusted_native_outcomes() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let file = scratch.path().join("empty");
    fs::write(&file, []).unwrap();
    let emitted = llvm(SOURCE, &tools);
    // A count above the request, a stream-only outcome on a file, and a
    // tag without positive errno are each impossible for this attempt.
    for (index, result) in [3_i64, -2, i64::MIN].into_iter().enumerate() {
        let ir = emitted
            .replace("@holo_refill_experimental", "@test.invalid_refill")
            .replace(
                "declare i64 @test.invalid_refill(i64, i64, ptr, i64, i64)",
                &format!("define internal i64 @test.invalid_refill(i64 %kind, i64 %fd, ptr %slot, i64 %len, i64 %limit) {{\nentry:\n ret i64 {result}\n}}"),
            );
        let binary = scratch.path().join(format!("bad-{index}"));
        tools.build(&ir, &binary, true).unwrap();
        assert_trap(
            &Command::new(&binary)
                .arg("file")
                .arg(&file)
                .output()
                .unwrap(),
        );
    }
}

/// Invalid requests must trap before calling the adapter. A distinctive test
/// exit makes adapter invocation observable even without payload writes.
#[test]
fn emitted_limits_guard_the_native_call() {
    let tools = NativeToolchain::discover().unwrap();
    let scratch = TempDir::new().unwrap();
    let file = scratch.path().join("empty");
    fs::write(&file, []).unwrap();
    // Cover the positive request rule, slot limit, and native integer limit.
    for limit in [0, 5, i32::MAX as u64 + 1] {
        let source = SOURCE.replace(
            "refill buffer.wire, 2",
            &format!("refill buffer.wire, {limit}"),
        );
        let ir = llvm(&source, &tools)
            .replace("@holo_refill_experimental", "@test.call_reached")
            .replace(
                "declare i64 @test.call_reached(i64, i64, ptr, i64, i64)",
                "define internal i64 @test.call_reached(i64 %kind, i64 %fd, ptr %slot, i64 %len, i64 %limit) {\nentry:\n call void @exit(i32 81)\n unreachable\n}",
            );
        let binary = scratch.path().join(format!("invalid-limit-{limit}"));
        tools.build(&ir, &binary, true).unwrap();
        assert_trap(
            &Command::new(&binary)
                .arg("file")
                .arg(&file)
                .output()
                .unwrap(),
        );
    }
}
