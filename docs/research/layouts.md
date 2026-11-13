# Layouts: move fewer bytes

Choose a physical representation that matches the work while preserving the
logical values and the guarantees that the program relies on.

**Research note — November 10, 2026.** Holo has a narrow experimental layout
planner, but general columns, blocks, codecs, and relative-link structures remain
design or research work. This note proposes experiments, not new approved syntax
or a universal representation. Read the [research guide](README.md),
[specification](../spec.md#9-layouts-encodings-and-collections), and
[M2 scope](../implementation-plan.md#m2--first-finite-compiler-improvement-accepted) first.

## Why layout can matter more than arithmetic

A CPU fetches data through a memory hierarchy. If an operation only needs a small
part of every record, arranging the records poorly can bring much unrelated data
toward the CPU. Pointer indirection and oversized metadata can add more work.

An illustrative dataset has 1,048,576 records. Each record occupies 64 bytes,
including one eight-byte reading and 56 bytes of other fields. The record extent
is 64 MiB; the readings alone occupy 8 MiB. Grouping readings together gives a
scan a compact field stream. Those sizes are arithmetic for this example, not
measurements of memory traffic or a prediction of an eightfold speedup.

The best layout depends on the operation. A lookup that consumes every field of
one record may prefer keeping those fields together. A scan of one field may
prefer keeping that field together across records.

## Four ideas, with ordinary names

| Idea | What changes | Useful case | Cost to investigate |
|---|---|---|---|
| Rows, also called array of structures | Fields of each record are adjacent. | Whole-record lookup or update. | Scanning one field may touch unrelated data. |
| Columns, also called structure of arrays | One field's values are adjacent across records. | Filters, aggregates, and SIMD scans. | A whole record spans several streams; growth and updates need coordination. |
| Blocks, often called array of structures of arrays | Small groups have column-style storage. | Vector work with some nearby-record locality. | Block size, partial tails, padding, and append behavior. |
| Compact encodings | Metadata or values use fewer bits where their ranges permit. | Bandwidth- or capacity-limited data. | Decode work, overflow, mutation, and reference capabilities. |

A hot/cold split is another useful arrangement: frequently used fields stay
together while large, rarely read payloads live in a separately described part.
It must expose any extra link, allocation, or lookup cost.

## An example without proposed Holo syntax

This diagram describes logical records and alternative byte arrangements. It is
not executable source, an ABI, or a promise that the compiler supports conversion.

```text
Logical records:
  (id A, reading 10), (id B, reading 20), (id C, reading 30)

Rows:
  [A, 10] [B, 20] [C, 30]

Columns:
  ids:      [A, B, C]
  readings: [10, 20, 30]

Two-record blocks:
  block 0: ids [A, B], readings [10, 20]
  block 1: ids [C],    readings [30]
```

A block's unused physical slots must never become readable uninitialized logical
elements. Its valid count, capacity, alignment, and stepping rules need a contract.

## What Holo has today

The accepted finite M1 core has one schema-driven planner with explicit
[`wide`, `derived`, and `compact` formats](../spec.md#experimental-physical-layouts).
For the documented nested `Entry` fixture, their fixed prefixes are 32, 16, and
12 bytes respectively. Those sizes describe that schema, not all Holo records.
`derived` removes eligible size headers; `compact` also uses bounded u32 byte
lengths. Region offsets remain u64. Current accesses are packed/alignment 1.

The source CLI defaults to `derived`; `compact` remains explicit. Its length
overflow rejects construction at the specified point rather than truncating or
silently switching format. The
[M1 acceptance](../implementation-plan.md#m1-acceptance-decision--november-10)
does not implement general columns, blocking, automatic narrow-width inference,
arbitrary relative links, or serialized images.

## What this could do for Holo

The useful language boundary is a distinction between **logical value**, **physical
mapping**, and **operation capability**. A compressed field may produce a decoded
value or an accessor instead of an ordinary native reference. Selecting its layout
cannot automatically promise direct mutable field access.

The compiler should know enough about common mappings to specialize address
calculation and retain range, alignment, and initialization facts. Libraries can
provide collection algorithms and exact codecs. The runtime provides explicit
backing and placement policies. Arbitrary mapping callbacks must not acquire an
unearned promise of safety, zero dispatch, or `noalloc`.

Relative links are a separate candidate: a checked base plus a narrow offset can
replace a wider pointer in eligible structures. Offset width, base identity,
overflow, bounds, alignment, and validation all need rules. A relative-link graph
is not automatically a network message or a portable file format.

## Start here: one small prototype

1. **Choose one real operation.** A fixed-width telemetry filter is a reasonable
   first candidate. Define exact input values, filter order, result order,
   arithmetic, and output construction before choosing a representation.
2. **Hold the algorithm fixed.** Compare rows and one explicit blocked layout,
   initially with the same scalar algorithm. Add a full-column option only if it
   answers a distinct question. Keep compression out of this first comparison.
3. **Use tuned controls.** Implement the same logical work in C and Rust with
   explicit flat storage. Include the intended pinned Goose checkout where
   supported. Mark unsupported Holo-new layouts unavailable until actual checked
   source or IR exists; a C representation study is feasibility evidence only.
4. **Separate layout from SIMD.** Compare each layout with scalar and eligible
   vectorized execution. This helps distinguish fewer bytes touched from better
   instruction selection. Include working sets below and above cache capacity.
5. **Measure the whole lifecycle.** Build directly into final storage where
   possible. Time construction, queries, updates, any conversion, and logical
   release separately. A scan win does not pay for an unreported rebuild.

Use independently generated expected results. Check field values and output
ordering, not only a checksum shared with the optimized algorithm. Keep batch
size, payload distribution, and capacity policy explicit.

### A first fixture with the choices already made

These numbers are starting points for an experiment, not intended defaults:

- Use 1,048,579 logical records, indexed from zero. Each has eight u64 fields:
  `id`, `reading`, `time`, and five auxiliary fields. A row contains 64 field bytes.
- Set `id` to its index, `reading` to `(17 × id + 3) modulo 1000`, and `time` to
  `10 × id`. Give auxiliary field number `j` the value `id + j`, for `j` from 1
  through 5. These inputs do not overflow u64.
- Filter records whose reading is at least 900 and output their IDs in original
  order. Preallocate output capacity for all IDs and reuse it between measured
  calls. Report that capacity and keep its allocation outside filter timing.
- Compare rows with blocked columns of 16, 64, and 256 records. The chosen record
  count leaves three valid records in the final block for each size. Mark unused
  slots inaccessible; do not count padded slots as records.
- Build the independent expected ID list directly from the numeric formula,
  without reading the candidate layout. Validate the full list and its length.
  Add a whole-record lookup or checksum phase to reveal the cost of splitting
  fields that another operation uses together.

## Correctness and capability cases

| Case | Required behavior |
|---|---|
| Empty input and a partial final block | No read of padding or uninitialized lanes; exact logical length. |
| Narrow metadata at its maximum and just beyond it | Correct value at the boundary; explicit rejection before truncation. |
| Failed append after earlier private fields were built | Specified suffix rollback; existing records and aliases remain valid. |
| Retained reference during supported growth | Stable address and access behavior, including failure from finite capacity. |
| Encoded field requested as an ordinary mutable reference | Supply that capability only when physically valid; otherwise reject or require an explicit accessor. |
| Two legal aliases access the same value | Preserve M1 semantics; compactness does not establish uniqueness. |
| Relative link with the wrong base, bad alignment, or an out-of-bounds target | Reject validation or checked access under the proposed representation's rules. |
| Explicit layout conversion fails | Defined ownership and cleanup behavior; never conceal relocation of live referenced data. |

An optimization cannot reorder field evaluation or change error precedence merely
because a column layout makes another order convenient. A lossy codec also needs
separate explicit numeric permission; it is not exact storage with a smaller name.

## What to measure and inspect

Report payload bytes, metadata, padding, capacity slack, copied bytes, and
allocations separately. Where measurable, distinguish reserved, committed, mapped,
and resident backing. Compact logical data does not prove compact resident memory.

Inspect offsets and alignment with the existing `holo inspect-layout` facility for
supported formats. Inspect generated assembly for address calculation, unaligned
accesses, checks, scalar fallback, and vector loads. Compare query throughput and
latency distributions, update costs, construction, conversion, and retained backing
after logical release.

Run measured competitors sequentially with varied order, matching safety and
numeric obligations. Preserve raw samples, input hashes, source/tool identities,
and exact flags. Stop or narrow the candidate if query gains disappear after
conversion or updates, if benefit depends on one convenient dataset, or if it
requires weakening flatness, references, growth, or reclamation guarantees.

## Compatibility with the storage baseline

Use the full [Goose checklist](../spec.md#goose-performance-compatibility-checklist).
This note's relevant design and implementation ledger is:

| Mechanism | Status, restriction, and acceptance evidence |
|---|---|
| Narrow metadata and representation inspection | Partly implemented by M1's explicit formats; existing scope is in the layout specification. New mappings need independent size/offset evidence. |
| Flatness and destination construction | Required eligible-path benefits. Count buffers and copies; several separately allocated columns do not establish one flat message. |
| Stable cheap growth and data-stack backing | Preserve supported retained references. A bounded fixed set of column regions is a candidate, not an approved universal growth scheme. Inspect reservations, capacity checks, and slack. |
| Fixed-marker logical release | Preserve object-count-independent reset for eligible data. Codec cleanup or per-block walks cannot silently become mandatory. |
| Static callbacks, SIMD, and check elimination | Later composition with established legality; inspect dispatch and actual generated checks. |
| Tagged alternatives and errors | Deferred beyond the selected experiment; document indexability, dispatch, failure order, and cleanup before adding them. |
| Images, queue eligibility, and stable-handle pools | Separate deferred mechanisms; byte compactness alone supplies neither validation nor stale-identity rules. |

## My assessment and open questions

Layout is one of Holo's strongest long-term opportunities because it affects
bandwidth, cache capacity, vectorization, and backing size together. It is also
easy to oversell: different operations prefer different representations.

I would keep mappings explicit initially, preserve a simple row control, and add
one useful alternative at a time. The key open questions are which layouts can
retain native element references, how independent columns grow without relocation,
and which costs belong in collection APIs. Those decisions should follow a small
prototype and a concrete capability table.

## References worth opening

- [LLAMA mappings](https://llama-doc.readthedocs.io/en/latest/pages/mappings.html)
  distinguishes physical, computed, and composable mappings, including row and
  column arrangements. Its compile-time mapping approach is useful compiler input.
- [Arrow columnar format](https://arrow.apache.org/docs/format/Columnar.html)
  gives concrete buffers, offset widths, validity representations, and mutation
  tradeoffs. Use it as a reference design, not a required Holo ABI.
- [Parquet encodings](https://parquet.apache.org/docs/file-format/data-pages/encodings/)
  describes dictionary, delta, and run/bit-packed encodings. File-format mechanisms
  need separate evaluation before use as mutable in-memory representations.
- [Holo layout rules and proposals](../spec.md#9-layouts-encodings-and-collections)
  owns the distinction between representation, encoding, and collection operations.
- [Compiler memory facts](../compiler.md#3-memory-facts-the-compiler-must-retain)
  explains which established facts must survive lowering.

For adjacent experiments, see [batches](batches.md) and the
[research index](README.md).
