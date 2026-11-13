# `telemetry.frame` namespace

[`decode.holo`](decode.holo) decodes payload lengths through
[`telemetry.wire`](../wire.holo) and owns the private frame signature.
[`validate.holo`](validate.holo) exposes the signature required by the caller's
check and declares the accepted length limit. Both files contribute
to the same namespace. Callers use `using telemetry.frame` to access its public
functions. The private signature stays within the namespace. The helpers operate on copied scalars
and retain no input buffer. See the [worker guide](../../README.md) to build
and verify the package.
