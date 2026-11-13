# RESP2 Holo sources

`main.holo` implements the checked parser, session loop, table dispatch,
and reply formatting. The parser publishes scalar offsets only after
complete validation. The entry compacts just the unconsumed tail and keeps
the output slot stable across short sends. These operations use the
experimental compiler core and hosted descriptor.

`store.holo` exports the inline slot schema, its constructor, and bounded
byte comparison through `redis_server.store`. The entry owns one fixed table.
It validates both source extents before writing a new key or value and
publishes the logical length after the write. Every slot's physical
capacity is established by its constructor.

The package manifest and reproduction commands live in the
[example guide](../README.md).
