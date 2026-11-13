# Native runtime support

This directory owns small native definitions used by generated Holo programs.
It is separate from the Rust compiler and from the libraries written in Holo.
There is no independently built runtime package or alternate execution engine.

| Component | Purpose and boundary |
|---|---|
| [llvm/prelude.ll](llvm/prelude.ll) | Embedded native declarations, fixed region/batch metadata types, and terminating assertions. |
| [measurement.c](measurement.c) | Optional monotonic clock and opaque optimizer boundary for explicit measurement operations. |
| [refill](refill/README.md) | Conditional hosted entry, checked single-attempt input and output, explicit readiness waits, and optional serial loopback sessions. |
| [socket](socket/README.md) | Standalone macOS socket readiness comparisons and correctness fixtures for the experimental socket library. |

The [LLVM backend](../compiler/src/backend/llvm/README.md) includes the prelude in
emitted modules, then generates construction, checked access, publication, rollback,
and fixed-marker cleanup. Those algorithms stay in the compiler; the prelude does
not walk records or implement a hidden allocator per object.

Generated safety/assertion failures now transfer directly through LLVM's target
trap, before any Holo formatting, libc termination, resource cleanup or callback.
Successful checks return normally. The failed operation cannot resume; subsequent
host exception/signal/debugger handling is outside Holo execution contracts.
This is an explicit fatal-check boundary, not a blanket exemption for nonreturning
foreign calls. On the audited Darwin arm64 target the trap is BRK/SIGTRAP, replacing
the previous libc abort/SIGABRT behavior. Other targets use LLVM's target trap;
this observation is not a portable promise of one signal or debugger policy.
Recoverable append statuses and ordinary lexical cleanup are unchanged.

The [native driver](../compiler/src/driver.rs) compiles `measurement.c` only when
emitted programs call its clock/barrier operations. It supplies no record-processing
kernel, owns no Holo payload, and makes no purity/non-aliasing promise. Instrumented
measurements must disclose these boundaries and their overhead.
Its own clock failure still uses libc abort: explicit uncontracted measurement
support is separate from the generated safety-check boundary.

The supported native path uses matched LLVM tools on the documented 64-bit macOS/
Linux hosts and the host C ABI. The backing and numeric policies remain those in
the [specification](../docs/spec.md). Hosted input is a separate experimental
profile. It owns descriptor setup and close independently of Holo storage.

Native contract acceptance is narrower than build support: the
[accepted M1 profile](../docs/implementation-plan.md#m1-acceptance-decision--november-10)
pins Darwin arm64 tools, eager chained imports, selected platform memory helpers
and hardware/environment obligations. Loader resolution occurs at startup before
contracted calls. Profile checking belongs to host acceptance orchestration and
adds no per-function check; arbitrary other native builds remain unestablished.

From the repository root, `cargo test --offline --workspace` exercises original,
optimized, and ASan native programs, including failure and cleanup coverage. Run
`cargo test --offline --test native` for focused native-driver checks. Missing LLVM
is a failure, not silently skipped coverage.
