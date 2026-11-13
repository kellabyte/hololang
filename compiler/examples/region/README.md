# Region example workload

`program.rs` constructs the typed IR workload used by the runnable example and
native integration tests. It defines ordinary logical schemas and producer calls;
the compiler contains no names or kernels specific to this example.
Producer signatures use explicit `Parameter::U64` entries and calls wrap numeric
expressions in `Argument::Scalar`. This workload remains scalar-input construction;
it does not measure borrowed byte-input copying.

The format for this increment uses packed eight-byte record-size and byte-length
metadata. This is inspectable initial machinery, not a compactness or speed win
over Goose. Values and failure outcomes are independently checked by the tests.

`measurement.rs` supplies a parameterized lifecycle input and an independent
logical checksum calculation for the native measurement driver. It reuses only
the example's ordinary schema/producer definitions, not compiler internals.
Records contain id=i and i&31 bytes, with byte j=(i+j)&255. Backing is bounded at
64 bytes per record; the actual initial representation occupies 32+(i&31).

The workload uses `ForEach` for linear traversal, accumulates append failure tags,
and verifies checksum/zero failures/zero marker after its end timestamp. Backing
allocation/free and output are outside timing. The clock and opaque barrier use
the small native support boundary; no foreign data-processing kernel is involved.
