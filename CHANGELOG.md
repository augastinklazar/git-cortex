# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 2026-11-09

### Added
- **Core Models & RFC 8785 Serialization (`cortex-core`)**:
  - Implemented typed `TombstoneRecord`, `EpistemicHandoff`, `InvariantContract`, `LifecycleEvent`, and `DerivedStatus`.
  - Monotonic ULID identifiers with kind prefixes (`tmb_`, `hnd_`, `inv_`, `chk_`, `cand_`, `evt_`).
  - Deterministic canonical JSON serialization via `serde_jcs`.
  - Secret scrubber (`cortex_core::redact::scrub`) masking API keys, JWTs, AWS credentials, and private keys.
  - Prompt injection shielding (`fence_tombstone`, `fence_invariant`) stripping ANSI codes and Unicode Bidirectional overrides.
- **AST Parsing & Normalized Hashing (`cortex-ast`)**:
  - Multi-language Tree-sitter parsers supporting Rust, TypeScript, TSX, JavaScript, Python, and Go, plus generic line-based fallback.
  - Comment- and whitespace-insensitive token stream hashing (`ast:sha256:<16hex>`).
  - High performance: verified sub-15ms extraction per file (53ms for 10,000 lines / 2,001 symbols).
- **Two-Tier Storage (`cortex-storage`)**:
  - Tier 1: Local SQLite WAL state database (`.git/cortex/state.db`) with linked worktree resolution (`git_common_dir`).
  - Tier 2: Distributed Git ledger (`refs/cortex/ledger`) storing immutable JSON records under sharded paths (`v1/<kind>/<shard>/<id>.json`) with conflict-free set-union merging.
  - Non-destructive Git tree snapshots for checkpoints (`refs/cortex/checkpoints/*`) and rewind backups (`refs/cortex/backups/*`).
- **Epistemic Engine (`cortex-engine`)**:
  - Dynamic AST symbol invalidation (`ACTIVE`, `STALE`, `ORPHANED`) computed on demand without database race conditions.
  - Checkpoint and rewind engine with automatic failure tombstone synthesis on revert.
  - Compact epistemic handoff packaging strictly bounded to ≤4,096 bytes with symbol diffs and fenced Markdown envelopes.
  - Invariant contract verification across working tree and staged index changes.
- **Passive Capture Engine (`cortex-capture`)**:
  - Command runner capturing exit codes, durations, stdout, and stderr with bounded buffers.
  - Automated regex error signature parsing for compilers and test runners.
  - Dirty-tree candidate tracking detecting `Edit -> Fail -> Revert` cycles.
- **Model Context Protocol (`cortex-mcp`)**:
  - Synchronous stdio JSON-RPC 2.0 MCP server exposing:
    - `cortex_inspect_symbol`
    - `cortex_record_tombstone`
    - `cortex_checkpoint_state`
    - `cortex_handoff_pack`
    - `cortex_verify_invariants`
    - `cortex_record_invariant`
- **CLI & Git Subcommand (`cortex-cli`)**:
  - Dual binary `cortex` / `git-cortex` supporting commands:
    `init`, `status`, `inspect`, `symbols`, `exec`, `tombstone`, `invariant`, `checkpoint`, `rewind`, `handoff`, `verify-invariants`, `promote`, `sync`, `reindex`, `hook`, `mcp-serve`, and `doctor`.
  - Non-destructive chained git hook installation (`pre-commit`, `post-commit`, `post-checkout`, `post-merge`).
  - Agent workspace bootstrapping generating `.mcp.json`, `AGENTS.md`, and `CLAUDE.md`.
- **JSON Schemas & Specifications**:
  - Published JSON Schema drafts (`v0.1`) for tombstones, handoffs, invariants, and events.
  - Formal Epistemic Context Protocol specification (`docs/spec/ECP-v0.1.md`).
