//! Deterministic native lifecycle tests for the checked TCP owner.
//!
//! The real close adapter is linked against a test-only syscall witness.
//! A real stream pair makes release observable at its peer, and a real
//! replacement fd catches retries after injected interruption. No listener
//! bind, elevated networking permission, or production hook is needed.

use holo::driver::{NativeToolchain, TempDir};
use holo::frontend::parse_experimental;
use holo::ir::{Expr, Op, Program};
use holo::{check, llvm};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Observe the exact explicit close status after consuming the owner.
const EXPLICIT_CLOSE: &str = r#"
func main returns u64 {
    with core.tcp_stream() as stream {
        let closed = core.close(stream)
        core.print(closed)
        return 0
    }
}
"#;

/// Leave a nested region on early entry return, closing its outer owner.
const EARLY_RETURN: &str = r#"
func main returns u64 {
    with core.tcp_stream() as stream {
        let memory = core.region(64) else { return 0 }
        if 1 == 1 {
            core.print(9)
            return 0
        }
        return 0
    }
}
"#;

/// Readiness spans two native generations of the same descriptor number.
/// The current generation returns early. A reported wait error reaches an
/// explicit consuming close and preserves the original status for the oracle.
const SUCCESSIVE_STREAMS: &str = r#"
func main returns u64 {
    with core.tcp_stream() as stream {
        let ready = core.wait_input(stream)
        if ready == 0 {
            core.print(ready)
            return 0
        }
        let closed = core.close(stream)
        core.assert(closed == 0)
        core.print(ready)
        return 0
    }
}
"#;

/// Run a finite native build command and retain its output for assertions.
fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{command:?}: {output:?}");
    output
}

/// Discover the Darwin SDK when needed by the same installed tools as the driver.
/// Linux has no external SDK selection for the native fixture.
fn sdk() -> Option<String> {
    // Homebrew Clang can otherwise point to an SDK absent from this host.
    cfg!(target_os = "macos").then(|| {
        let output = success(Command::new("xcrun").arg("--show-sdk-path"));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    })
}

/// Create a native compiler command with the selected host target and SDK.
fn compile(tools: &NativeToolchain, sdk: Option<&str>, optimized: bool) -> Command {
    let mut command = Command::new(&tools.clang);
    command.args(["-target", tools.target_triple(), "-std=c17"]);
    // Preserve the configured native profile on targets requiring an SDK.
    if let Some(sdk) = sdk {
        command.args(["-isysroot", sdk]);
    }
    // The Linux TCP host links pthread support for its opt-in broker.
    if cfg!(target_os = "linux") {
        command.arg("-pthread");
    }
    command.arg(if optimized { "-O3" } else { "-O0" });
    command
}

/// Native objects with test-only substitution of host and readiness calls.
struct NativeObjects {
    /// The real accept loop and descriptor release boundary.
    host: PathBuf,
    /// The real readiness adapter, with Darwin event delivery observable.
    wait: PathBuf,
}

/// Compile the real native adapters with deterministic substitutions limited
/// to their respective test objects. Other adapters link in their normal form.
fn adapter(
    tools: &NativeToolchain,
    sdk: Option<&str>,
    runtime: &Path,
    scratch: &Path,
    optimized: bool,
) -> NativeObjects {
    let level = if optimized { "o3" } else { "o0" };
    let host = scratch.join(format!("tcp-{level}.o"));
    let wait = scratch.join(format!("tcp-wait-{level}.o"));
    success(
        compile(tools, sdk, optimized)
            .args([
                "-Dmain=holo_fixture_host_main",
                "-Dclose=holo_fixture_close",
                "-Dbind=holo_fixture_bind",
                "-Dlisten=holo_fixture_listen",
                "-Daccept=holo_fixture_accept",
                "-Dholo_refill_admit_experimental=holo_fixture_admit",
                "-c",
            ])
            .arg(runtime.join("tcp.c"))
            .arg("-o")
            .arg(&host),
    );
    success(
        compile(tools, sdk, optimized)
            .args(["-Dkevent=holo_fixture_kevent", "-c"])
            .arg(runtime.join("tcp_wait.c"))
            .arg("-o")
            .arg(&wait),
    );
    NativeObjects { host, wait }
}

/// Compile checked source or checked IR into the isolated witness.
/// LLVM verification runs before Clang links the real native close adapter.
fn link(
    tools: &NativeToolchain,
    sdk: Option<&str>,
    runtime: &Path,
    objects: &NativeObjects,
    llvm: &str,
    binary: &Path,
    optimized: bool,
) {
    let ir = binary.with_extension("ll");
    fs::write(&ir, llvm).unwrap();
    success(
        Command::new(&tools.opt)
            .args(["-passes=verify", "-disable-output"])
            .arg(&ir),
    );
    success(
        compile(tools, sdk, optimized)
            .arg("-I")
            .arg(runtime)
            .arg(&ir)
            .arg(&objects.host)
            .arg(&objects.wait)
            .arg(runtime.join("refill.c"))
            .arg(runtime.join("send.c"))
            .arg(runtime.join("tcp_concurrent.c"))
            .arg(runtime.join("tcp_permits.c"))
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/support/tcp_close_adapter.c"))
            .arg("-o")
            .arg(binary),
    );
}

/// Exercise source-level explicit close and nested early return and the raw
/// checked IR's normal fallthrough. Both native optimization levels see
/// real close success, then injected EINTR after the same descriptor is reused.
#[test]
fn checked_tcp_closes_once_across_all_exit_edges() {
    let tools = NativeToolchain::discover().unwrap();
    let sdk = sdk();
    let scratch = TempDir::new().unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/refill");
    let explicit = parse_experimental(EXPLICIT_CLOSE).unwrap();
    let early = parse_experimental(EARLY_RETURN).unwrap();
    // Source entry requires a return. Verify the separately supported IR
    // fallthrough boundary without claiming it is accepted source syntax.
    let fallthrough = Program {
        records: vec![],
        producers: vec![],
        body: vec![Op::TcpScope {
            stream: "stream".into(),
            body: vec![Op::Print(Expr::Const(8))],
        }],
    };
    let cases = [
        ("explicit", explicit.check().unwrap(), b"0\n".as_slice()),
        ("early", early.check().unwrap(), b"9\n".as_slice()),
        (
            "fallthrough",
            check::check(&fallthrough).unwrap(),
            b"8\n".as_slice(),
        ),
    ];
    // The native fixture itself asserts one close and correct fd identity.
    // Check language-visible statuses on both native optimization profiles.
    for optimized in [false, true] {
        let object = adapter(&tools, sdk.as_deref(), &runtime, scratch.path(), optimized);
        let level = if optimized { "o3" } else { "o0" };
        for (case, checked, stdout) in &cases {
            let ir = llvm::emit(checked, tools.target_triple());
            let name = format!("{case}-{level}");
            let binary = scratch.path().join(&name);
            link(
                &tools,
                sdk.as_deref(),
                &runtime,
                &object,
                &ir,
                &binary,
                optimized,
            );
            let normal = success(Command::new(&binary).arg("normal"));
            assert_eq!(normal.stdout, *stdout, "{name}");
            assert!(normal.stderr.is_empty(), "{normal:?}");
            let interrupted = success(Command::new(&binary).arg("interrupt"));
            // EINTR is Darwin and Linux errno 4. It is a terminal tagged
            // result only for explicit close, and implicit cleanup ignores it.
            let expected = if *case == "explicit" {
                b"9223372036854775812\n".as_slice()
            } else {
                stdout
            };
            assert_eq!(interrupted.stdout, expected, "{name}");
            assert!(interrupted.stderr.is_empty(), "{interrupted:?}");
            // An injected setup failure remains native-owned and publishes
            // no Holo print, including when checked code would print on exit.
            if *case == "explicit" {
                let rejected = Command::new(&binary).arg("reject").output().unwrap();
                assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
                assert!(rejected.stdout.is_empty(), "{rejected:?}");
                assert_eq!(
                    rejected.stderr, b"unsupported accepted TCP descriptor or mode\n",
                    "{rejected:?}",
                );
            }
        }
    }
}

/// Exercise exactly one owner at a time while the real host retains its
/// listener. On Darwin replace one real wait result with the previous
/// client's event after reusing its descriptor and retaining the queue.
#[test]
fn checked_tcp_reuses_descriptors_and_rejects_stale_events() {
    let tools = NativeToolchain::discover().unwrap();
    let sdk = sdk();
    let scratch = TempDir::new().unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/refill");
    let source = parse_experimental(SUCCESSIVE_STREAMS).unwrap();
    let checked = source.check().unwrap();
    let ir = llvm::emit(&checked, tools.target_triple());
    // The same Holo entry must work when one session closes implicitly and
    // another consumes its owner explicitly after an invalid readiness event.
    for optimized in [false, true] {
        let objects = adapter(&tools, sdk.as_deref(), &runtime, scratch.path(), optimized);
        let level = if optimized { "o3" } else { "o0" };
        let binary = scratch.path().join(format!("successive-{level}"));
        link(
            &tools,
            sdk.as_deref(),
            &runtime,
            &objects,
            &ir,
            &binary,
            optimized,
        );
        let normal = success(Command::new(&binary).arg("many"));
        assert_eq!(normal.stdout, b"0\n0\n0\n", "{level}");
        assert!(normal.stderr.is_empty(), "{normal:?}");

        // Rejected admission after one successful handoff releases only host
        // resources, and the third session must never begin.
        let rejected = Command::new(&binary).arg("reject-second").output().unwrap();
        assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
        assert_eq!(rejected.stdout, b"0\n", "{rejected:?}");
        assert_eq!(
            rejected.stderr, b"unsupported accepted TCP descriptor or mode\n",
            "{rejected:?}",
        );

        // Only Darwin exposes the optional persistent event queue.
        if cfg!(target_os = "macos") {
            let stale = success(Command::new(&binary).arg("stale"));
            let expected = format!("0\n{}\n0\n", (1u64 << 63) | 5);
            assert_eq!(stale.stdout, expected.as_bytes(), "{level}");
            assert!(stale.stderr.is_empty(), "{stale:?}");

            // An attachment failure keeps the next client host-owned. It
            // closes that client, the persistent queue, and the listener.
            let failed = Command::new(&binary)
                .arg("fail-attachment")
                .output()
                .unwrap();
            assert_eq!(failed.status.code(), Some(2), "{failed:?}");
            assert_eq!(failed.stdout, b"0\n", "{failed:?}");
            assert_eq!(
                failed.stderr, b"could not configure TCP readiness queue\n",
                "{failed:?}"
            );
        }
    }
}

/// A partial worker startup joins only launched threads. A later broker
/// failure wakes idle admission instead of waiting for another connection.
#[test]
fn concurrent_tcp_releases_host_resources_on_native_failures() {
    let tools = NativeToolchain::discover().unwrap();
    let sdk = sdk();
    let scratch = TempDir::new().unwrap();
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/refill");
    let object = scratch.path().join("broker.o");
    let binary = scratch.path().join("faults");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/support/tcp_concurrent_adapter.c");
    // Only the real broker object uses the substitutions. The witness still
    // forwards successful thread operations to the installed C runtime.
    success(
        compile(&tools, sdk.as_deref(), true)
            .args([
                "-Dpthread_create=tcp_fault_create",
                "-Dpthread_join=tcp_fault_join",
                "-Dpoll=tcp_fault_poll",
                "-c",
            ])
            .arg(runtime.join("tcp_concurrent.c"))
            .arg("-o")
            .arg(&object),
    );
    success(
        compile(&tools, sdk.as_deref(), true)
            .arg("-I")
            .arg(&runtime)
            .arg(fixture)
            .arg(object)
            .arg(runtime.join("tcp_permits.c"))
            .arg("-o")
            .arg(&binary),
    );
    let permits = success(Command::new(&binary).arg("permit-fifo"));
    assert!(permits.stdout.is_empty(), "{permits:?}");
    // Both modes share teardown, even with parking permits configured.
    for profile in ["held", "parked"] {
        for mode in ["startup", "broker"] {
            let output = success(Command::new(&binary).args([mode, profile]));
            assert!(output.stdout.is_empty(), "{output:?}");
        }
    }
}
