# Compiler integration tests

These suites check the public compiler pipeline. Compiler-private unit tests
live with their owning source modules. Cross-component fixtures and independent
helpers are shared through [fixtures](../fixtures/README.md) and
[support](../support/README.md).

| Group | Coverage |
| --- | --- |
| [Frontend](frontend/README.md) | Source tools, parsing, checked source, and evaluation order |
| [Checked IR](check/README.md) | Definitions, diagnostics, access rules, and lifetimes |
| [Storage](storage/README.md) | Layouts, reservation, initialization, and borrowed bytes |
| [Backend](backend/README.md) | Optimization selection and native equivalence |
| [Packages](packages/README.md) | Module resolution, namespace visibility, and lockfiles |

Each group lists its Cargo targets and native tool requirements. See the
[test index](../README.md) for workspace commands.
