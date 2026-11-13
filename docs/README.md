# git-cortex documentation

## Human developer

Install the CLI, initialize a repository, and check command flags or setup troubleshooting.

| Document | Summary |
|---|---|
| [Project overview](../README.md) | Problem summary, visual lifecycle overview, installation options, and command tables |
| [Getting started](GETTING-STARTED.md) | Prerequisites, install paths, Windows build setup, first session walkthrough, worktrees, and troubleshooting |
| [CLI reference](CLI-REFERENCE.md) | Syntax, flags, defaults, JSON output formats, and exit codes for all 17 commands |
| [FAQ](FAQ.md) | Answers on storage footprint, linked worktrees, `IMPORT` envelopes, `pre-commit` test execution, and packaging |

## AI agent and harness builder

Configure MCP clients, read fenced epistemic output safely, and manage 4,096-byte context handoffs.

| Document | Summary |
|---|---|
| [Agent guide](AGENT-GUIDE.md) | Session protocol, `IMPORT` envelope workflow, provenance fencing rules, and copyable harness snippets |
| [MCP and integration](MCP-AND-INTEGRATION.md) | Stdio JSON-RPC 2.0 transport, 6 MCP tool schemas, client recipes, and chained Git hook behavior |
| [Security policy](../SECURITY.md) | Execution boundaries, ANSI/Bidi/comment sanitization, secret redaction rules, and advisory reporting |

## Protocol and architecture reader

Study how SQLite WAL storage, the `refs/cortex/ledger` tree, Tree-sitter AST hashing, and ECP v0.1 work internally.

| Document | Summary |
|---|---|
| [Concepts and storage](CONCEPTS-AND-STORAGE.md) | Tier 1 SQLite tables, Tier 2 ledger sharding, AST hashing rules, dynamic invalidation, passive capture, and rewind |
| [Architecture](ARCHITECTURE.md) | Seven-crate workspace topology, crate boundaries, data models, and release hashing benchmarks |
| [Schemas guide](SCHEMAS.md) | Prefixed ULID format, RFC 8785 JCS canonicalization, field tables, A2A wrapping, and schema alignment notes |
| [ECP v0.1 specification](spec/ECP-v0.1.md) | Normative Epistemic Context Protocol v0.1 rules and v0.1.0 implementation notes |
| [Architecture and MVP specification (PDF)](spec/git-cortex%20Architecture%20&%20MVP%20Specification.pdf) | Original architecture blueprint and MVP specification |
| [Technical audit and remediation specification (PDF)](../git-cortex-Technical-Audit-and-Remediation-Specification.pdf) | Audit findings `GC-001` through `GC-039` and remediation tracking |
| [Visual philosophy](assets/VISUAL-PHILOSOPHY.md) | Design system notes for the diagrams and plates in `docs/assets/` |

## JSON Schemas (v0.1)

Draft 2020-12 JSON Schema definitions live under [`schemas/v0.1/`](../schemas/v0.1/).

| Schema | Summary |
|---|---|
| [`tombstone.json`](../schemas/v0.1/tombstone.json) | `TombstoneRecord` (`tmb_...`) structure for failed approaches and verification evidence |
| [`handoff.json`](../schemas/v0.1/handoff.json) | `EpistemicHandoff` (`hnd_...`) envelope with call-graph edges, bounded hypotheses, and unchanged files |
| [`invariant.json`](../schemas/v0.1/invariant.json) | `InvariantContract` (`inv_...`) structure linking guarded symbols and upstream AST dependencies |
| [`event.json`](../schemas/v0.1/event.json) | `LifecycleEvent` (`evt_...`) append-only `REVOKE`, `REAFFIRM`, and `SUPERSEDE` event schema |
| [`checkpoint.json`](../schemas/v0.1/checkpoint.json) | `CheckpointRecord` (`chk_...`) schema and alignment notes against `refs/cortex/checkpoints/` |

## Contributor

Build the workspace, run all 25 test suites, and check release history.

| Document | Summary |
|---|---|
| [Contributing guide](../CONTRIBUTING.md) | Toolchain setup, Windows drive workarounds, test commands, formatting/clippy gates, and PR rules |
| [Changelog](../CHANGELOG.md) | Verified release notes and benchmark baseline for `v0.1.0` |
| [MIT license](../LICENSE-MIT) | MIT license terms |
| [Apache-2.0 license](../LICENSE-APACHE) | Apache License 2.0 terms |
