//! Test-only driver for one real, nonblocking hosted Unix stream.
//!
//! Small fixtures queue their wire data before launch, making the read caps
//! deterministic. This helper owns both endpoints outside the generated
//! program and records no timing or memory observations.

use std::io::Write;
use std::net::Shutdown;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Queue `wire` and pass a connected endpoint to `binary` as native input.
/// A half-close exposes EOF after data. Otherwise the peer stays live while
/// the child runs. Return the completed child output, failing with diagnostics
/// on setup or execution failure. The parent descriptor table stays untouched.
pub fn read_stream(binary: &Path, wire: &[u8], half_close: bool) -> Output {
    let (reader, mut peer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    peer.write_all(wire).unwrap();
    // A half-close reports End after queued bytes. With the peer open,
    // an empty queue exposes the adapter's WouldBlock status.
    if half_close {
        peer.shutdown(Shutdown::Write).unwrap();
    }
    let inherited: OwnedFd = reader.into();
    // The shell duplicates its inherited stdin onto fd 3 for the native host.
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec 3<&0; exec \"$1\" stream-fd 3", "typed-refill"])
        .arg(binary)
        .stdin(Stdio::from(inherited));
    let output = command.output().unwrap();
    assert!(output.status.success(), "{command:?}: {output:?}");
    output
}
