# Architecture & System Design: git-cortex (ECP v0.1)

`git-cortex` is a local-first epistemic memory engine for AI coding agents. It records why edits failed, what was tried, which invariants cannot be violated, and packages clean cross-session handoffs inside Git without modifying source code or polluting working tree files.

---

## 1. High-Level Component Topology

```
+-----------------------------------------------------------------------------------+
|                                  AI AGENT / USER                                  |
|         (Cursor, Claude Code, Windsurf, Copilot, Custom Orchestrators)            |
+------------------------------------+----------------------------------------------+
                                     |
              +----------------------+----------------------+
              | JSON-RPC 2.0 (stdio)                        | CLI Commands
              v                                             v
+-----------------------------+               +-----------------------------+
|         cortex-mcp          |               |         cortex-cli          |
|  (Tools: inspect, tombstone,|               |  (init, exec, inspect,      |
|   checkpoint, handoff,      |               |   handoff, rewind, sync,    |
|   verify, record_invariant) |               |   hook, doctor, mcp-serve)  |
+--------------+--------------+               +--------------+--------------+
               |                                             |
               +----------------------+----------------------+
                                      v
                      +-------------------------------+
                      |         cortex-engine         |
                      |  - AST Symbol Invalidation    |
                      |  - Epistemic Handoff Packing  |
                      |  - Checkpoint & Rewind        |
                      |  - Invariant Verification     |
                      |  - Ledger Synchronization     |
                      +---------------+---------------+
                                      |
         +----------------------------+----------------------------+
         v                                                         v
+------------------+                                      +------------------+
|    cortex-ast    |                                      |  cortex-capture  |
| - Tree-sitter    |                                      | - Command Runner |
| - Rust/TS/Py/Go  |                                      | - Error Parser   |
| - AST Hash Gen   |                                      | - Revert Monitor |
+------------------+                                      +------------------+
         |                                                         |
         +----------------------------+----------------------------+
                                      v
                      +-------------------------------+
                      |        cortex-storage         |
                      |  - Tier 1: SQLite WAL DB      |
                      |  - Tier 2: refs/cortex/ledger |
                      |  - Tree Snapshots & Backups   |
                      |  - Worktree Common Directory  |
                      +---------------+---------------+
                                      |
                                      v
                      +-------------------------------+
                      |          cortex-core          |
                      |  - ECP Types & ULID Identifiers|
                      |  - Canonical JSON (RFC 8785)  |
                      |  - Credential Redactor        |
                      |  - Prompt Injection Defense   |
                      +-------------------------------+
```

---

## 2. Two-Tier Storage Architecture

`git-cortex` uses a dual-tier storage strategy designed to balance millisecond agent query latencies with distributed, Git-native persistence:

```
+----------------------------------------------------------------------------------+
| TIER 1: Local Session Storage (Ephemeral / Epistemic Cache)                      |
| Location: .git/cortex/state.db (SQLite WAL Mode)                                 |
| Purpose:  Sub-5ms index reads, candidate tracking, call-graph cache, fast edits  |
+----------------------------------------------------------------------------------+
                                        |
                          promote / reconcile / sync
                                        v
+----------------------------------------------------------------------------------+
| TIER 2: Distributed Epistemic Ledger (Persistent / Shared)                       |
| Location: refs/cortex/ledger (Orphan Git Tree / Commit History)                  |
| Purpose:  Immutable JSON records pushed/fetched across git remotes               |
| Sharding: v1/<kind>/<shard>/<id>.json (e.g. v1/tombstones/ab/tmb_01H...json)    |
+----------------------------------------------------------------------------------+
```

### 2.1 Tier 1: SQLite WAL State Database
- Located at `.git/cortex/state.db` (or `<commondir>/cortex/state.db` when operating inside a linked git worktree via `resolve_commondir`).
- Configured with `PRAGMA journal_mode = WAL;`, `PRAGMA synchronous = NORMAL;`, `PRAGMA foreign_keys = ON;`, and `PRAGMA busy_timeout = 5000;`.
- Migrations are version-gated via `PRAGMA user_version = 1` inside `BEGIN IMMEDIATE` transaction boundaries to protect against race conditions under concurrent worker processes.
- Tables:
  - `records`: Local cache of tombstones, handoffs, invariants, and events.
  - `checkpoints`: Saved working tree checkpoints with snapshot refs and graph counts.
  - `executions`: Command execution outputs, exit codes, signatures, and durations.
  - `candidates`: Pre-commit modifications tracking dirty working tree hashes and touched symbols.
  - `file_reads`: Session read-tracking for context assembly.
  - `call_graph_edges`: Inferred caller/callee relations across symbols.
  - `hypotheses`: Active failure hypotheses under investigation with validated confidence [0.0..1.0].

### 2.2 Tier 2: `refs/cortex/ledger`
- Stored as an independent, orphan commit graph under the dedicated git reference `refs/cortex/ledger`.
- Tracked safely via the remote-tracking refspec `+refs/cortex/ledger:refs/remotes/origin/cortex/ledger`.
- Sharded path layout using trailing 2 characters of the monotonic ULID:
  ```
  v1/
    tombstones/
      2r/
        tmb_01JBYN8W8X26Z6C8A79P0K4Z2R.json
    handoffs/
      b8/
        hnd_01JBYN99F4478K1P096RTVXMB8.json
    invariants/
      zc/
        inv_01JBYN9QWW847KP3308RT65NZC.json
    events/
      tv/
        evt_01JBYNA97T5632N7B60Q123RTV.json
  ```
- **Merge Strategy**: Conflict-free Set-Union with CAS retry backoff. Because every record filename is uniquely keyed by a monotonic ULID and immutable, remote merges never produce file-content conflicts. A three-way tree merge simply takes the union of all JSON blobs. Invariant reaffirmations log append-only `LifecycleEvent` records (`evt_...`) to preserve the immutability of historical ledger commits.

---

## 3. AST Extraction & Normalized Hashing (`cortex-ast`)

Code symbols (functions, methods, classes, interfaces, structs, impl blocks) are indexed using tree-sitter grammars.

### 3.1 Normalization Algorithm
To make symbols immune to superficial edits (e.g., formatting, renaming internal parameters, changing comments):
1. **Comment Stripping**: All comment nodes are omitted during token traversal.
2. **Whitespace Collapsing**: All non-significant whitespace sequences are normalized to single spaces.
3. **Canonical Hash**: Normalized token streams are hashed via SHA-256. The hash format is:
   ```
   ast:sha256:<16-hex-chars>
   ```

### 3.2 Dynamic Invalidation Model
Unlike static systems that flag records as invalid using mutable database flags, `git-cortex` calculates record status dynamically at query time:
- **`ACTIVE`**: Symbol still exists in working tree with identical `ast_hash`.
- **`STALE`**: Symbol exists at target path, but its `ast_hash` differs from stored hash.
- **`ORPHANED`**: Symbol or file no longer exists in working tree.

---

## 4. Epistemic Primitives

### 4.1 Tombstones (Negative Knowledge)
Tombstones record dead ends, failed assumptions, and unviable refactorings.
- Fields: `id`, `created_at`, `symbol_id`, `file_path`, `ast_hash`, `failed_approach`, `failure_reason`, `reproduction_cmd`, `error_signature`, `confidence`, `author`.
- Injected into agent context with strict anti-injection fencing to avoid executing or trusting unverified strings.

### 4.2 Causal Invariant Locks
Invariants declare non-negotiable behavioral or architectural rules anchored to AST symbols.
- Fields: `id`, `symbol_id`, `file_path`, `ast_hash`, `assertion`, `enforcement_level` (`Advisory`, `WarnCommit`, `BlockCommit`), `verification_test_cmd`.
- Verified at git hook intervals (`pre-commit`).
- **Safety**: `verification_test_cmd` is NEVER automatically invoked in arbitrary subshells without explicit operator opt-in.

### 4.3 Epistemic Handoffs
When an agent session concludes or context resets, `create_handoff_envelope` compresses current epistemic state into a strict ≤4096 byte budget:
- Summarizes modified symbols and active branch state.
- Embeds top active tombstones and invariant assertions.
- Formats structured agent-to-agent markdown blocks for zero-loss handoffs.

---

## 5. Checkpoints & Revert Engine

1. **Checkpoints (`cortex checkpoint`)**:
   - Commits working tree state as a lightweight Git tree object under `refs/cortex/checkpoints/<ULID>`.
   - Records checkpoint metadata in Tier 1 database.
2. **Rewinds (`cortex rewind <id>`)**:
   - Creates a safety backup of dirty working state under `refs/cortex/backups/<ULID>`.
   - Restores the exact checkpoint working tree.
   - Computes diff against abandoned state and automatically synthesizes candidate tombstones.

---

## 6. Model Context Protocol (MCP) Interface

The MCP server (`git-cortex mcp-serve`) implements JSON-RPC 2.0 over standard input/output (`stdio`):
- `cortex_inspect_symbol`: Fetches symbol AST hash, status, active tombstones, and invariant locks.
- `cortex_record_tombstone`: Records an approach failure with error signatures.
- `cortex_checkpoint_state`: Saves non-destructive checkpoint.
- `cortex_handoff_pack`: Generates compact context envelope.
- `cortex_verify_invariants`: Runs checks against staged or working tree symbols.
- `cortex_record_invariant`: Declares invariant rules for code symbols.
