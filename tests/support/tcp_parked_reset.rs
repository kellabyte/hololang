//! Exercise real TCP resets on checked owners parked in read and write waits.
//! The client injects abortive close only on its own descriptors. An orderly
//! peer proves progress before each reset and verifies isolation and EOF.

use super::{
    await_shutdown, build_tcp_source, check_echo, finish_echo, open_peer, parked_modes,
    start_server,
};
use holo::driver::TempDir;
use std::ffi::{c_int, c_uint, c_void};
use std::fs;
use std::io;
use std::net::TcpStream;
use std::os::fd::AsRawFd;

/// Native TCP reset errno from the supported platform's socket headers.
const RESET_ERRNO: u64 = if cfg!(target_os = "macos") { 54 } else { 104 };
/// Socket-level option family on the two supported platform ABIs.
const SOL_SOCKET: c_int = if cfg!(target_os = "macos") { 0xffff } else { 1 };
/// Socket linger option. Both native ABIs interpret a zero duration as abortive.
const SO_LINGER: c_int = if cfg!(target_os = "macos") { 0x80 } else { 13 };

/// Native `struct linger` for the fixture's client-only abortive close.
#[repr(C)]
struct Linger {
    /// Enable the close policy for this peer.
    enabled: c_int,
    /// Zero means the local close sends a reset instead of draining.
    duration: c_int,
}

unsafe extern "C" {
    /// Set one socket option from a caller-owned native value for this call.
    fn setsockopt(
        fd: c_int,
        level: c_int,
        option: c_int,
        value: *const c_void,
        len: c_uint,
    ) -> c_int;
}

/// Consume one client peer with `SO_LINGER={1,0}`, causing a real TCP reset.
fn reset_peer(peer: TcpStream) {
    let linger = Linger {
        enabled: 1,
        duration: 0,
    };
    // SAFETY: the borrowed fd remains owned by peer during this call. The
    // initialized C layout and exact socklen_t match Darwin and Linux.
    let result = unsafe {
        setsockopt(
            peer.as_raw_fd(),
            SOL_SOCKET,
            SO_LINGER,
            (&linger as *const Linger).cast(),
            size_of::<Linger>() as c_uint,
        )
    };
    assert_eq!(
        result,
        0,
        "could not arm abortive close: {}",
        io::Error::last_os_error()
    );
    drop(peer);
}

/// Cycle three checked owners through one execution permit and three live
/// slots. Advancing the third peer proves both earlier owners yielded before
/// reset, including the large sender whose reply the client leaves unread.
#[test]
#[ignore = "requires permission to bind an ephemeral IPv4 loopback listener"]
fn checked_parked_slots_recover_after_read_and_write_resets() {
    const WAVES: u8 = 48;
    let work = TempDir::new().unwrap();
    let fixture = work.path().join("parked-reset.holo");
    // Specialize only the expected platform errno. The real native adapter
    // supplies the observed result through the existing tagged status ABI.
    let tagged = (1u64 << 63) | RESET_ERRNO;
    let source = format!(
        "// Supply the native target's tagged connection reset.\nfunc reset_status with noalloc, noblock returns u64 {{ return {tagged} }}\n{}",
        include_str!("tcp_parked_reset.holo")
    );
    fs::write(&fixture, source).unwrap();
    let binary = build_tcp_source(&work, &fixture);
    let count = (usize::from(WAVES) * 3).to_string();
    for &mode in parked_modes() {
        let (mut server, port, stdout) = start_server(&binary, &[mode, "0", &count, "3", "1"]);
        for wave in 0..WAVES {
            let mut reader = open_peer(port);
            check_echo(&mut reader, &[1]);
            let mut writer = open_peer(port);
            check_echo(&mut writer, &[2]);
            // Leave the large response unread. A third independent owner
            // can acknowledge only after the first two release their permit.
            let mut survivor = open_peer(port);
            check_echo(&mut survivor, &[3]);
            reset_peer(reader);
            reset_peer(writer);
            check_echo(&mut survivor, &[wave ^ 0xa5]);
            finish_echo(survivor);
        }
        assert!(await_shutdown(&mut server), "{mode}");
        let mut outcomes = [0u8; 3];
        for line in stdout.join().unwrap().lines() {
            let status = line.parse::<usize>().unwrap();
            assert!(
                (1..=3).contains(&status),
                "unexpected terminal outcome {status}"
            );
            outcomes[status - 1] += 1;
        }
        assert_eq!(outcomes, [WAVES; 3], "{mode}");
        eprintln!("{mode}: {WAVES} read resets, {WAVES} write resets, {WAVES} exact orderly peers");
    }
}
