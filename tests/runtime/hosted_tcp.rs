//! Source and native conformance for the lexical TCP stream.
//! The loopback witness retains one bounded buffer through short writes,
//! explicit readiness waits, and input half-close. The concurrent witness
//! holds multiple owners live, checks admission bounds, and reuses slots.
//! A one-permit host must let another checked owner run during either wait.

#[path = "../support/tcp_close.rs"]
mod tcp_close;
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "../support/tcp_parked_reset.rs"]
mod tcp_parked_reset;

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::ir::{Op, Program};
use holo::layout::LayoutMode;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Input half-close followed by a large finite reply with independently
/// verifiable wire bytes and exact progress through the checked API.
const BACKPRESSURE: &str = r#"
// Reuse the initialized backing for every attempt.
struct Slot { input Bytes, output Bytes }

// Construct the record at its destination. The output repeats every 256 bytes.
func slot with noblock returns Slot {
    return Slot { input: core.bytes(2, 99), output: core.bytes(8388608, core.index()) }
}

// Inspect the checked fresh prefix without retaining it or copying its bytes.
func extent with noalloc, noblock arg received Bytes as readonly reference returns u64 {
    return core.len(received)
}

// Check the one-byte receive, input EOF, and the still-live output half.
func main returns u64 {
    with core.tcp_stream() as stream {
        let memory = core.region(8392704) else { return 0 }
        let slots = core.batch(memory, Slot)
        let added = core.append(slots, slot())
        core.assert(added == 0)
        let buffer = core.borrow_readwrite(slots, 0)
        let readable = core.wait_input(stream)
        core.assert(readable == 0)
        refill buffer.input, 2 from stream as received {
            core.assert(extent(received) == 1)
            core.assert(buffer.input[0] == 165)
            core.assert(buffer.input[1] == 99)
        } else status { core.assert(false) }
        let eof = core.wait_input(stream)
        core.assert(eof == 0)
        refill buffer.input, 2 from stream as received {
            core.assert(false)
        } else status { core.assert(status == 0) }

        var cursor = 0
        var short_writes = 0
        var blocks = 0
        // Keep backing live and retry the suffix the kernel did not accept.
        loop {
            if cursor == 8388608 {
                core.assert(short_writes > 0)
                core.assert(blocks > 0)
                core.print(short_writes)
                core.print(blocks)
                return 0
            }
            let remaining = core.sub_wrapping(8388608, cursor)
            let sent = core.send(stream, buffer.output, cursor, remaining)
            if sent == 18446744073709551614 {
                blocks = core.add_wrapping(blocks, 1)
                let writable = core.wait_output(stream)
                core.assert(writable == 0)
            } else {
                core.assert(sent > 0)
                core.assert(sent <= remaining)
                if sent < remaining {
                    short_writes = core.add_wrapping(short_writes, 1)
                }
                cursor = core.add_wrapping(cursor, sent)
            }
        }
    }
}
"#;

/// Run the source frontend and checker with the diagnostic left intact.
fn checked_source(source: &str) -> Result<(), String> {
    parse_experimental(source)
        .map_err(|error| format!("{error:?}"))?
        .check()
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

/// Reject copying, owner shadowing, nested explicit close, implicit host
/// access, and a raw IR attempt to pass off a consuming action as a borrow.
#[test]
fn tcp_owner_rejects_bypasses_and_conflicting_scope_state() {
    for (body, message) in [
        ("let copied = stream\nreturn 0", "scalar"),
        (
            "let status = core.wait_input()\nreturn 0",
            "explicit live stream",
        ),
        (
            "let status = core.next_session()\nreturn 0",
            "explicit live stream",
        ),
        (
            "if 1 == 1 { let status = core.close(stream) }\nreturn 0",
            "directly inside",
        ),
        (
            "let closed = core.close(stream)\nlet wait = core.wait_input(stream)\nreturn 0",
            "already closed",
        ),
        (
            "let first = core.close(stream)\nlet second = core.close(stream)\nreturn 0",
            "already closed",
        ),
        (
            "{ let stream = 1\nlet wait = core.wait_output(stream) }\nreturn 0",
            "TCP stream",
        ),
        (
            "with core.tcp_stream() as other { return 0 }\nreturn 0",
            "only executable scope",
        ),
    ] {
        let source =
            format!("func main returns u64 {{ with core.tcp_stream() as stream {{\n{body}\n}} }}");
        let error = checked_source(&source).unwrap_err();
        assert!(error.contains(message), "{source}\n{error}");
    }
    let raw = Program {
        body: vec![Op::TcpScope {
            stream: "stream".into(),
            body: vec![Op::TcpIo {
                stream: "stream".into(),
                operation: Box::new(Op::TcpClose {
                    stream: "stream".into(),
                    result: "result".into(),
                }),
            }],
        }],
        ..Program::default()
    };
    assert!(
        holo::check::check(&raw)
            .unwrap_err()
            .to_string()
            .contains("invalid operation")
    );
}

/// Own the test server and reap it even when the client reports a failure.
struct Server(Child);

impl Drop for Server {
    /// Stop any unfinished process and release its stdio and listener.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Bound worker and broker shutdown after all peers reach their terminal EOF.
fn await_shutdown(server: &mut Server) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // A completed child has released the listener and joined the workers.
        if let Some(status) = server.0.try_wait().unwrap() {
            return status.success();
        }
        assert!(
            Instant::now() < deadline,
            "TCP host did not finish shutdown"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Launch the checked host and read its ephemeral port with a timeout.
/// The reader drains application output until the child has finished.
fn start_server(binary: &Path, arguments: &[&str]) -> (Server, u16, thread::JoinHandle<String>) {
    let mut server = Server(
        Command::new(binary)
            .args(arguments)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = server.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut output = BufReader::new(stdout);
        let mut port = String::new();
        output.read_line(&mut port).unwrap();
        send.send(port).unwrap();
        let mut rest = String::new();
        output.read_to_string(&mut rest).unwrap();
        rest
    });
    let port = receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .trim()
        .parse::<u16>()
        .unwrap();
    (server, port, reader)
}

/// Open a bounded test connection, leaving both halves available to the caller.
fn open_peer(port: u16) -> TcpStream {
    let client =
        TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(5))
            .unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client
}

/// Send the provided wire bytes and half-close the output half. The caller
/// owns the remaining input half and must validate the received bytes.
fn connect(port: u16, bytes: &[u8]) -> TcpStream {
    let mut client = open_peer(port);
    client.write_all(bytes).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    client
}

/// Exercise both readiness backends with one owner and with two simultaneous
/// senders. Delay reads so each nonblocking sender encounters backpressure.
#[test]
#[ignore = "requires permission to bind an ephemeral IPv4 loopback listener"]
fn checked_tcp_partial_writes_and_half_close() {
    let tools = NativeToolchain::discover().unwrap();
    let work = TempDir::new().unwrap();
    let parsed = parse_experimental(BACKPRESSURE).unwrap();
    let checked = parsed.check().unwrap();
    let ir = holo::llvm::emit_with_layout(&checked, tools.target_triple(), LayoutMode::Derived64)
        .unwrap();
    let binary = work.path().join("backpressure");
    tools.build(&ir, &binary, true).unwrap();
    // Malformed admission must stop before the Holo owner begins.
    let rejected = Command::new(&binary)
        .args(["accept-loopback", "invalid"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    let mut modes = vec![
        vec!["accept-loopback", "0"],
        vec!["accept-loopback-concurrent", "0", "2", "2"],
    ];
    if cfg!(target_os = "macos") {
        modes.extend([
            vec!["accept-loopback-kqueue", "0"],
            vec!["accept-loopback-kqueue-concurrent", "0", "2", "2"],
        ]);
    }
    for arguments in modes {
        let mode = arguments[0];
        let owners = if arguments.len() == 4 { 2 } else { 1 };
        let (mut server, port, stdout) = start_server(&binary, &arguments);
        let peers: Vec<_> = (0..owners).map(|_| connect(port, &[165])).collect();
        // Connect all peers before reading, so their send waits can overlap.
        thread::sleep(Duration::from_millis(250));
        let readers: Vec<_> = peers
            .into_iter()
            .map(|mut peer| {
                thread::spawn(move || {
                    let mut reply = Vec::new();
                    peer.read_to_end(&mut reply).unwrap();
                    assert_eq!(reply.len(), 8 * 1024 * 1024, "{mode}");
                    assert!(
                        reply
                            .iter()
                            .enumerate()
                            .all(|(index, &byte)| index as u8 == byte),
                        "{mode}"
                    );
                })
            })
            .collect();
        // Join only after starting every reader, so one peer cannot starve
        // another peer whose send window is also full.
        for reader in readers {
            reader.join().unwrap();
        }
        assert!(server.0.wait().unwrap().success(), "{mode}");
        let output = stdout.join().unwrap();
        let counts: Vec<u64> = output.lines().map(|line| line.parse().unwrap()).collect();
        assert_eq!(counts.len(), 2 * owners, "{mode}");
        assert!(counts.iter().all(|&count| count > 0), "{mode}: {counts:?}");
        eprintln!(
            "{mode}: {owners} owners × 8388608 exact bytes, positive short/blocked counts {counts:?}"
        );
    }
}

/// Run the same locked application through repeated checked admissions.
/// Distinct bytes and reply EOF witness isolation between client sessions.
#[test]
#[ignore = "requires permission to bind an ephemeral IPv4 loopback listener"]
fn checked_tcp_hosts_successive_connections() {
    let work = TempDir::new().unwrap();
    let binary = work.path().join("echo");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/tcp_echo/holo.toml");
    let build = Command::new(env!("CARGO_BIN_EXE_holo"))
        .args(["build", "--locked", "--release", "--package"])
        .arg(manifest)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(build.status.success(), "{build:?}");

    let modes = if cfg!(target_os = "macos") {
        &["accept-loopback-many", "accept-loopback-kqueue-many"][..]
    } else {
        &["accept-loopback-many"][..]
    };
    for mode in modes {
        // Validate count before binding so invalid invocations publish no port.
        for count in ["0", "-1", "18446744073709551616", "1x", ""] {
            let rejected = Command::new(&binary)
                .args([mode, "0", count])
                .output()
                .unwrap();
            assert_eq!(rejected.status.code(), Some(2), "{mode} {count:?}");
            assert!(rejected.stdout.is_empty(), "{rejected:?}");
            assert_eq!(rejected.stderr, b"invalid loopback connection count\n");
        }

        let (mut server, port, stdout) = start_server(&binary, &[mode, "0", "32"]);
        // Each checked entry must close before the client's next exchange.
        for index in 0..32 {
            let expected = [255 - index];
            let mut client = connect(port, &expected);
            let mut reply = Vec::new();
            client.read_to_end(&mut reply).unwrap();
            assert_eq!(reply, expected, "{mode} session {index}");
        }
        assert!(server.0.wait().unwrap().success(), "{mode}");
        assert_eq!(stdout.join().unwrap(), "1\n0\n".repeat(32), "{mode}");
        eprintln!("{mode}: 32 isolated byte echoes and successful closes");
    }
}

/// Compile one TCP source in scratch for the selected native host modes.
fn build_tcp_source(work: &TempDir, source: &Path) -> std::path::PathBuf {
    let binary = work.path().join("tcp-worker");
    let output = Command::new(env!("CARGO_BIN_EXE_holo"))
        .args(["build", "--release"])
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    binary
}

/// Build a maintained TCP example through the same checked native driver.
fn build_tcp_example(work: &TempDir, name: &str) -> std::path::PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/tcp_echo")
        .join(name);
    build_tcp_source(work, &source)
}

/// List the explicit parked profiles available to this native test host.
fn parked_modes() -> &'static [&'static str] {
    // The queue variant uses Darwin's kevent broker. Both platforms use poll.
    if cfg!(target_os = "macos") {
        &["accept-loopback-parked", "accept-loopback-kqueue-parked"]
    } else {
        &["accept-loopback-parked"]
    }
}

/// Reject unsupported bounds before opening a listener or starting workers.
#[test]
fn checked_tcp_concurrent_rejects_invalid_bounds() {
    let work = TempDir::new().unwrap();
    let binary = build_tcp_example(&work, "stream.holo");
    // All entries are invalid even on a host where local binding is allowed.
    for (count, limit) in [("0", "4"), ("3", "0"), ("3", "65"), ("3", "-1"), ("3", "x")] {
        let output = Command::new(&binary)
            .args(["accept-loopback-concurrent", "0", count, limit])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{count} {limit}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }
    // Parking additionally checks the relationship between its two bounds.
    for &mode in parked_modes() {
        for (count, live, active) in [
            ("0", "4", "1"),
            ("3", "0", "1"),
            ("3", "65", "1"),
            ("3", "4", "0"),
            ("3", "4", "5"),
            ("3", "4", "-1"),
        ] {
            let output = Command::new(&binary)
                .args([mode, "0", count, live, active])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{count} {live} {active}");
            assert!(output.stdout.is_empty(), "{output:?}");
        }
    }
}

/// Send and verify a distinct message while keeping the stream alive.
fn check_echo(peer: &mut TcpStream, message: &[u8]) {
    peer.write_all(message).unwrap();
    let mut reply = vec![0; message.len()];
    peer.read_exact(&mut reply).unwrap();
    assert_eq!(reply, message);
}

/// Close the client send half and require the checked owner to release its end.
fn finish_echo(mut peer: TcpStream) {
    peer.shutdown(Shutdown::Write).unwrap();
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

/// Prove that active owners overlap while remaining isolated. Once four
/// owners are live, a fifth connection must wait for a released slot. The
/// parked profiles give all four owners one shared execution permit.
#[test]
#[ignore = "requires permission to bind an ephemeral IPv4 loopback listener"]
fn checked_tcp_concurrent_routes_active_and_idle_clients() {
    let work = TempDir::new().unwrap();
    let binary = build_tcp_example(&work, "stream.holo");
    let mut modes = vec![vec!["accept-loopback-concurrent", "0", "12", "4"]];
    if cfg!(target_os = "macos") {
        modes.push(vec!["accept-loopback-kqueue-concurrent", "0", "12", "4"]);
    }
    // One permit must be sufficient because each idle owner parks in its wait.
    modes.extend(
        parked_modes()
            .iter()
            .map(|mode| vec![*mode, "0", "12", "4", "1"]),
    );

    for args in modes {
        let mode = args[0];
        let (mut server, port, stdout) = start_server(&binary, &args);
        // Each handshake proves that the next owner runs even though all
        // earlier connections are still open and waiting for more bytes.
        let mut peers: Vec<_> = (0u8..4)
            .map(|index| {
                let mut peer = open_peer(port);
                check_echo(&mut peer, &[0xa1, index, 0xb2]);
                peer
            })
            .collect();

        // Queue a fifth connection at the kernel listener. The host may
        // accept it internally but cannot run another checked owner yet.
        let mut waiting = open_peer(port);
        waiting.write_all(&[0xc3, 0x04, 0xd4]).unwrap();
        waiting
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let error = waiting.peek(&mut [0]).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        finish_echo(peers.remove(0));
        waiting
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reply = [0; 3];
        waiting.read_exact(&mut reply).unwrap();
        assert_eq!(reply, [0xc3, 0x04, 0xd4], "{mode}");
        peers.push(waiting);

        // Keep several owners live across slot reuse. An event routed to
        // another owner cannot match its independently chosen payload.
        for index in 5u8..12 {
            finish_echo(peers.remove(0));
            let mut next = open_peer(port);
            check_echo(&mut next, &[0xe5, index, 0xf6]);
            check_echo(&mut peers[0], &[index, 0x17, 0x28]);
            peers.push(next);
        }
        // All twelve independently admitted owners reach orderly EOF.
        for peer in peers {
            finish_echo(peer);
        }
        assert!(await_shutdown(&mut server), "{mode}");
        assert!(stdout.join().unwrap().is_empty(), "{mode}");
        eprintln!("{mode}: 12 checked owners, limit 4, exact bytes and EOF");
    }
}

/// Keep a slow sender's initialized region live while a fast owner uses its
/// sole permit. Check the full backing and bounded replay window on the wire.
#[test]
#[ignore = "requires permission to bind an ephemeral IPv4 loopback listener"]
fn checked_tcp_parks_backpressured_owner_through_exact_reply() {
    for (source, require_short) in [
        ("mixed_readers.holo", true),
        ("streaming_readers.holo", false),
    ] {
        let work = TempDir::new().unwrap();
        let binary = build_tcp_example(&work, source);
        // Both workers must really park. A bounded attempt can fill its
        // whole window, so its positive short-send count can be zero.
        for &mode in parked_modes() {
            check_parked_backpressured_reply(&binary, source, mode, require_short);
        }
    }
}

/// Drain the slow peer only after the fast one reaches EOF, then verify the
/// complete slow response and the checked entry's exact pressure report.
fn check_parked_backpressured_reply(binary: &Path, source: &str, mode: &str, require_short: bool) {
    let (mut server, port, stdout) = start_server(binary, &[mode, "0", "2", "2", "1"]);
    let mut slow = connect(port, &[1]);
    let mut receipt = [0];
    slow.read_exact(&mut receipt).unwrap();
    assert_eq!(receipt, [1], "{mode}");

    // The slow entry already acknowledged. Its large reply is still
    // unread, so this fast entry needs the permit released by wait_output.
    let mut fast = connect(port, &[129]);
    let mut reply = Vec::new();
    fast.read_to_end(&mut reply).unwrap();
    assert_eq!(reply.len(), 1025, "{mode}");
    assert_eq!(reply[0], 129, "{mode}");
    assert!(
        reply[1..]
            .iter()
            .enumerate()
            .all(|(i, &byte)| byte == (i as u8 ^ 129))
    );

    // Draining restores write readiness to the original owner and backing.
    reply.clear();
    slow.read_to_end(&mut reply).unwrap();
    assert_eq!(reply.len(), 4 * 1024 * 1024, "{mode}");
    assert!(reply.iter().enumerate().all(|(i, &byte)| byte == i as u8));
    assert!(await_shutdown(&mut server), "{mode}");
    let lines: Vec<u64> = stdout
        .join()
        .unwrap()
        .lines()
        .map(|line| line.parse().unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{mode}: {lines:?}");
    assert_eq!(lines[0], 1, "{mode}: first blocked owner");
    assert_eq!(lines[1] >> 32, 1, "{mode}: completed owner");
    if require_short {
        assert_ne!((lines[1] >> 16) & 65535, 0, "{mode}: short writes");
    }
    assert_ne!(lines[1] & 65535, 0, "{mode}: blocked writes");
    eprintln!(
        "{source} {mode}: fast EOF preceded slow drain; exact 4 MiB, {} short and {} blocked writes",
        (lines[1] >> 16) & 65535,
        lines[1] & 65535,
    );
}
