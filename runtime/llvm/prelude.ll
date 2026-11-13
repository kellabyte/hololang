; Runtime helpers for the checked-region LLVM backend.
;
; This file owns native declarations, fixed metadata types, and assertion failure.
; storage.rs emits ordered reservation checks directly at their producer sites.
; Typed layout-specific record lookup is emitted by representation.rs. The
; emitter owns lexical cleanup and record publication. Backing is acquired once
; per region, independently of the emitted bump reservations.
; Stored sizes/lengths and user fields may be unaligned, so loads use align 1.
; No helper infers non-aliasing from a source access permission.
; Failed safety checks transfer directly to the target trap. No Holo cleanup,
; formatting, libc termination or callback runs before that transfer; subsequent
; host signal/debugger handling is outside the Holo execution-contract boundary.

; Region metadata: base owns the sole backing allocation; capacity is its logical
; byte limit; top is the next-unused offset. This fixed header is never inline data.
%holo.region = type { ptr, i64, i64 }
; Batch metadata: region is its backing owner; start is the saved entry marker;
; end is the published byte end; count is the number of fully initialized records.
%holo.batch = type { ptr, i64, i64, i64 }

; Acquire uninitialized backing bytes once per region; null means acquisition
; failed and no owner may be exposed. The caller handles a zero logical capacity.
declare ptr @malloc(i64)
; Release an acquired backing allocation after its lexical owner ends. Logical
; batch reclamation does not call this per record or per nested field.
declare void @free(ptr)
; Transfer a failed required check directly to the target's fatal trap instruction.
; This declaration carries LLVM's intrinsic meaning, not a general exemption for
; arbitrary nonreturning callees from allocation/blocking contracts.
declare void @llvm.trap()
; Complete entry execution after emitter-generated lexical cleanup. Host exit
; flushes explicitly printed buffered output and never resumes the Holo program.
declare void @exit(i32) noreturn
; Print the explicitly observed unsigned value through the host C ABI. Output is
; outside constructors and any memory-kernel performance measurement.
declare i32 @printf(ptr, ...)
; Observe monotonic native time for explicit measurement operations only. The C
; support file contains clock/barrier plumbing, never a construction/scan kernel.
declare i64 @holo_now()
; Expose actual backing to an opaque memory barrier so timed work cannot vanish.
; No attributes promise purity or non-aliasing across this instrumentation call.
declare void @holo_opaque(ptr)
; Compute unsigned arithmetic and its overflow bit before using the result.
declare { i64, i1 } @llvm.uadd.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.usub.with.overflow.i64(i64, i64)
declare { i64, i1 } @llvm.umul.with.overflow.i64(i64, i64)

@holo.number_format = private unnamed_addr constant [6 x i8] c"%llu\0A\00"

; Enforce a source/runtime assertion without returning through invalid state.
; condition is the established predicate; true returns without external work and
; false traps before the protected operation. The fatal edge owns no recovery or
; resource cleanup and cannot introduce libc abort's signal/stdio/sleep machinery.
define internal void @holo.assert(i1 %condition) {
entry:
  ; The failure edge must terminate before any protected memory operation.
  br i1 %condition, label %valid, label %invalid
invalid:
  call void @llvm.trap()
  unreachable
valid:
  ret void
}

