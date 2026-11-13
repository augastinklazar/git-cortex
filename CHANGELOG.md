# Changelog

This file records notable changes to `git-cortex`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-09

### Added

#### Core models and canonical serialization (`cortex-core`)

- Typed record structs (`TombstoneRecord`, `EpistemicHandoff`, `InvariantContract`, `LifecycleEvent`, `DerivedStatus`).
- Monotonic ULID identifiers with kind prefixes (`tmb_`, `hnd_`, `inv_`, `chk_`, `cand_`, `evt_`) and strict uppercase Crockford Base32 validation.
- Deterministic RFC 8785 canonical JSON serialization via `serde_jcs` and `sha256:` content digests.
- Automated secret scrubber (`cortex_core::redact::scrub`) with 13 regex rules (AWS `AKIA`/`ASIA` keys, GitHub `gh[pousr]_` and 82-char `github_pat_` tokens, Bearer tokens, key/secret/token/password assignments, PEM private keys, connection URI credentials, Slack tokens, OpenAI/Anthropic/Google API keys) plus a Shannon entropy (`> 3.5`, `>= 32` chars) pass with a camelCase identifier filter.
- Prompt injection sanitizer and wrapper (`cortex_core::fence::sanitize_text`, `fence_tombstone`, `fence_invariant`) stripping ANSI/CSI/OSC/DCS/APC/PM/SOS escapes, C0/C1 controls, Bidi overrides, and Unicode Tags, neutralizing HTML delimiters and fake fence prefixes, capping rendered fields (`600`/`800`/`300`/`100` chars), and stamping `Provenance` tags.

#### AST parsing and normalized hashing (`cortex-ast`)

- Multi-language Tree-sitter extraction for Rust, TypeScript (`.ts`, `.mts`, `.cts`), TSX, JavaScript (`.js`, `.mjs`, `.cjs`, `.jsx`), Python (`.py`, `.pyi`), and Go, plus `#<file>` trimmed-line fallback and `#<binary>` SHA-256 fallback.
- Normalized AST hashing (`ast:sha256:<16hex>`) that ignores comments, Python docstrings, indentation, and CRLF line-ending differences.
- Measured release hashing latency of 6.7 ms to 15.1 ms on ~1,100 to 1,200 LOC files (TypeScript 1,080 LOC: 6.7 ms; Rust 1,200 LOC: 12.9 ms; Python 1,200 LOC: 15.1 ms, verified under a `<= 35 ms` release assertion) and ~659 ms on a 10,002-line / 2,001-symbol TypeScript stress test.

#### Two-tier storage (`cortex-storage`)

- Tier 1 SQLite WAL state database (`.git/cortex/state.db`) with `busy_timeout = 5000`, `foreign_keys = ON`, and linked `git worktree` sharing via `commondir` resolution.
- Tier 2 distributed Git ledger (`refs/cortex/ledger`) storing records under trailing-2-char sharded paths (`v1/<kind>/<shard>/<id>.json`), conflict-free set-union tree merges, and OID mismatch tamper detection.
- Non-destructive working-tree snapshots using an isolated temporary Git index under `refs/cortex/checkpoints/*`, `refs/cortex/backups/*`, and `refs/cortex/untracked/*`.

#### Epistemic engine (`cortex-engine`)

- Dynamic AST invalidation computed on demand in `inspect_symbol` (marking records active, stale/drifted, or revoked; `DerivedStatus::Orphaned` is defined in `cortex-core` while deleted symbols currently appear in `stale_*` lists).
- Checkpoint and rewind workflow synthesizing a `REWIND_SYNTHESIZED` failure tombstone for modified symbols on rollback.
- Epistemic handoff packer (`EpistemicHandoff`) enforcing a 4,096-byte budget via a 7-tier pruning cascade, salience scoring, `file_reads` staleness tracking, and Google A2A `tasks/send` formatting.
- Invariant verification across working tree, staged index, and `HEAD` tree scopes, with optional passing-test auto-reaffirmation (`verify_and_reaffirm_invariants`) used by `pre-commit`.

#### Passive capture (`cortex-capture`)

- Command runner (`execute_command`) with optional process-group (Unix) and process-tree (Windows) timeout (`124`), concurrent stdout and stderr draining to memory, terminal forwarding, and 4,096-byte UTF-8 `output_tail` persistence in SQLite.
- Normalized error signature extraction (`0x<HEX>`, `<TIME>`, 300-byte UTF-8 safe truncation).
- Clean-tree baseline failure suppression and partial-revert candidate reconciliation detecting `Edit -> Fail -> Revert` cycles.

#### Model Context Protocol server (`cortex-mcp`)

- Dual-framed (`Content-Length` and NDJSON) stdio JSON-RPC 2.0 server (`protocolVersion: "2024-11-05"`) exposing `cortex_inspect_symbol`, `cortex_record_tombstone`, `cortex_checkpoint_state`, `cortex_handoff_pack`, `cortex_verify_invariants`, and `cortex_record_invariant`.

#### CLI and Git subcommand (`cortex-cli`)

- `git-cortex` and `cortex` binaries implementing 17 subcommands: `init`, `status`, `inspect`, `symbols`, `exec`, `tombstone`, `invariant`, `checkpoint`, `rewind`, `handoff`, `verify-invariants`, `promote`, `sync`, `reindex`, `hook`, `mcp-serve`, and `doctor`.
- Chained Git hook installation (`pre-commit`, `post-commit`, `post-checkout`, `post-merge`), safe remote-tracking fetch refspec setup, and `.mcp.json` / `AGENTS.md` / `CLAUDE.md` bootstrapping.

#### Schemas, specifications, and packaging

- Five Draft 2020-12 JSON Schemas under `schemas/v0.1/` (`tombstone.json`, `handoff.json`, `invariant.json`, `event.json`, `checkpoint.json`) and the Epistemic Context Protocol specification (`docs/spec/ECP-v0.1.md`).
- Release workflow (`.github/workflows/release.yml`) building `git-cortex` binaries with SHA-256 manifests for Linux x64/arm64, macOS x64/arm64, and Windows x64 MSVC, plus an `npm` wrapper under `npm/` and a source Homebrew formula template under `packaging/homebrew/git-cortex.rb`.
