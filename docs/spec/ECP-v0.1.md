# Epistemic Context Protocol (ECP v0.1.1)

> Normative Target Specification for Git-Native Epistemic Working Memory, Negative-Knowledge Tombstones, and Causal Invariant Locks for Multi-Agent Software Engineering.

---

## 1. Specification Attributes

| Attribute | Normative Target |
|---|---|
| **Protocol & Binary Name** | `git-cortex` (CLI & Git Subcommand) / `ECP v0.1` |
| **Target Interoperability** | Model Context Protocol (MCP), Google Agent2Agent (A2A), `AGENTS.md`, `CLAUDE.md`, Claude Code, Codex CLI, Gemini CLI, Cursor, OpenCode |
| **Storage Architecture** | **Tier 1**: Local SQLite WAL Index (`.git/cortex/state.db`)<br/>**Tier 2**: Distributed Conflict-Free Ledger (`refs/cortex/ledger`) |
| **AST Binding Engine** | Tree-sitter Structural Symbol Hashing (64-bit truncated SHA-256 over normalized, comment-insensitive AST subtrees) |

---

## 2. Executive Summary & Architectural Motivation

Autonomous coding agents operating in software repositories routinely face three fundamental systemic failures:

1. **The Context Compaction Death Spiral**: When conversation history exceeds model context limits, compaction prunes negative knowledge first ("why refactoring symbol X caused a downstream race condition"). Minutes later, the agent re-implements the exact same failed attempt.
2. **The Cross-Harness & Sub-Agent Handoff Tax**: Switching between harnesses or delegating tasks to sub-agents forces the incoming agent to restart from scratch, spending up to 70% of initial tokens re-reading files to reconstruct verified call graphs.
3. **Silent Semantic Merge Conflicts**: Standard Git tracks line-level syntax diffs. When an upstream dependency alters a behavioral precondition, Git merges cleanly while introducing silent runtime regressions.

### Architectural Thesis
Source control must version not only code that succeeded (syntax diffs), but also:
- **Hypotheses proved false** (`Tombstones`)
- **Verified symbol relationships and working hypotheses** (`Epistemic Checkpoints`)
- **Cross-file behavioral preconditions that must hold** (`Causal Invariant Locks`)

---

## 3. Two-Tier Storage Architecture

To prevent repository bloat while supporting multi-agent collaboration, `git-cortex` separates high-frequency ephemeral memory from immutable repository history:

```
┌─────────────────────────────────────────────────────────────┐
│ Tier 1: Local Ephemeral SQLite Cache (.git/cortex/state.db) │
│ - Branch-scoped working memory & candidate snapshots        │
│ - Reconciles Edit → Fail → Revert loops                     │
│ - Fully rebuildable via `git cortex reindex`               │
└──────────────────────────────┬──────────────────────────────┘
                               │ Promoted on commit or sync
┌──────────────────────────────▼──────────────────────────────┐
│ Tier 2: Distributed Ledger Ref (refs/cortex/ledger)         │
│ - Immutable commit tree holding canonical RFC 8785 JSON     │
│ - Path: v1/<kind>/<shard>/<id>.json                         │
│ - Set-union tree merges never conflict                      │
└─────────────────────────────────────────────────────────────┘
```

### Protocol Deviation Note (ECP v0.1.1 Refinement)
The original blueprint proposed storing records in `refs/notes/cortex` as well as sub-notes `refs/notes/cortex/tombstones`. In Git's ref storage engine, a ref cannot simultaneously exist as a loose file and a directory (a directory/file conflict). Furthermore, notes are keyed by Git commit SHA rather than by symbol URI, requiring expensive full scans on symbol lookup.

**ECP v0.1.1 Normative Resolution**:
All distributed records are committed to an independent branch reference `refs/cortex/ledger`. Records are stored at `v1/<kind>/<shard>/<id>.json` where `<shard>` is derived from the ULID. Merges between collaborating agents are purely additive set-unions that **cannot produce merge conflicts**.

---

## 4. Deterministic AST Symbol Addressing (`ast_hash`)

Binding epistemic metadata to file line numbers fails because edits shift line offsets. `git-cortex` uses embedded Tree-sitter parsers to address symbols canonically:

1. **Canonical Symbol URI**: `<relative_filepath>#<scope_chain>`
   - Rust: `src/pool/connection.rs#ConnectionPool::acquire`
   - TypeScript/JavaScript: `src/auth/token_manager.ts#TokenManager.refreshSession`
   - Python: `app/worker.py#Worker.process`
   - Go: `pkg/queue/worker.go#Worker.Dispatch`
   - Unsupported text files: `<relative_filepath>#<file>` fallback.

2. **Normalized AST Hash Calculation**:
   - Strips all comments (`comment`, `line_comment`, `block_comment`, doc comments, and Python docstrings).
   - Normalizes whitespace (CRLF and formatting changes do not alter the hash).
   - Serializes CST/AST node kinds and identifiers into a canonical byte stream.
   - Computes SHA-256 and truncates to 16 hex characters: `ast:sha256:4b91e08a72c5d301`.

3. **Status Derivation**:
   Record status (`ACTIVE`, `STALE`, `ORPHANED`) is evaluated at query time by comparing stored hashes with current AST hashes. When underlying code is refactored, stale tombstones transition automatically without conflicting edits.

---

## 5. Normative Data Schemas

All persistent records serialize to canonical JSON (RFC 8785):

- **TombstoneRecord**: disproved hypothesis or failed approach. Schema: `https://git-cortex.dev/schemas/v0.1/tombstone.json`.
- **EpistemicHandoff**: deterministic <=4 KB envelope for cross-harness and sub-agent task transfer. Schema: `https://git-cortex.dev/schemas/v0.1/handoff.json`.
- **InvariantContract**: causal invariant lock guarding architectural assumptions. Schema: `https://git-cortex.dev/schemas/v0.1/invariant.json`.
- **LifecycleEvent**: append-only event recording manual reaffirmation, revocation, or superseding. Schema: `https://git-cortex.dev/schemas/v0.1/event.json`.

---

## 6. Model Context Protocol (MCP) Server

`git-cortex` runs as a zero-latency stdio JSON-RPC 2.0 server (`git-cortex mcp-serve`). It exposes five normative tools:

1. `cortex_inspect_symbol`: Inspect a file or symbol before planning changes. Returns active Tombstones and Invariant Contracts formatted in fenced markdown with provenance tags.
2. `cortex_record_tombstone`: Record a disproved hypothesis or failed code edit.
3. `cortex_checkpoint_state`: Checkpoint verified call graph edges and active hypotheses; create a working tree snapshot commit.
4. `cortex_handoff_pack`: Export or import an `EpistemicHandoff` payload (<= 4 KB).
5. `cortex_verify_invariants`: Verify causal invariant locks against staged, working tree, or branch diffs.
6. `cortex_record_invariant`: Create an architectural invariant lock protecting symbols against upstream dependency drift.

---

## 7. Passive Capture Engine (`git cortex exec`)

Rather than relying solely on voluntary agent logging, `git cortex exec -- <cmd>` passively observes execution:
1. **Dirty Diff Snapshot**: Parses modified files against HEAD to map touched symbols (`symbol_uri` + base and dirty `ast_hash`).
2. **Failure Correlation**: If the command exits with non-zero status, extracts a normalized error signature and records a candidate failure.
3. **Revert Promotion**: If the agent subsequently reverts those files back to their base AST hash, `git-cortex` automatically promotes the candidate into an active `AUTO_REVERT_DETECTED` Tombstone.

---

## 8. Split-Brain Safe Rollback (`git cortex rewind`)

Running `git cortex rewind <checkpoint-id>` executes an atomic four-step recovery:
1. Creates an automatic backup of the current dirty state to `refs/cortex/backups/<id>`.
2. Synthesizes a summary TombstoneRecord covering the aborted changes.
3. Restores working tree and index cleanly to `<checkpoint-id>`.
4. Attaches the newly synthesized Tombstone to provide immediate immunity against repeating the dead end.

---

## 9. Security Defenses

- **Safe Test Commands**: Invariant verification test commands are never automatically executed without explicit user confirmation.
- **Prompt Injection Fencing**: External or collaborator tombstones are stripped of ANSI sequences, Unicode Bidi overrides, capped in length, and wrapped in strict provenance comments (`<!-- CORTEX_TOMBSTONE_BEGIN ... -->`).
- **Secret Redaction**: Terminal outputs and commands pass through automated credential scrubbing (AWS, GitHub, Slack, Bearer tokens, private keys, high-entropy tokens) before persistence.
- **Tamper Detection**: Content hashes (`sha256(canonical_json)`) protect against corrupted or modified ledger blobs.
