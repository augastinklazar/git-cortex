# Schema and data model reference

`git-cortex` defines five JSON Schema Draft 2020-12 specifications under [schemas/v0.1](../schemas/v0.1/tombstone.json) alongside Rust serialization models in [crates/cortex-core/src/schema.rs](../crates/cortex-core/src/schema.rs) and SQLite tables in [crates/cortex-storage/src/migrations.rs](../crates/cortex-storage/src/migrations.rs).

| Title | Schema file | Rust type | Schema version constant |
| --- | --- | --- | --- |
| `TombstoneRecord` | [tombstone.json](../schemas/v0.1/tombstone.json) | `TombstoneRecord` | `0.1.0` |
| `EpistemicHandoff` | [handoff.json](../schemas/v0.1/handoff.json) | `EpistemicHandoff` | `0.1.0` |
| `InvariantContract` | [invariant.json](../schemas/v0.1/invariant.json) | `InvariantContract` | `0.1.0` |
| `LifecycleEvent` | [event.json](../schemas/v0.1/event.json) | `LifecycleEvent` | None in schema or struct |
| `CheckpointRecord` | [checkpoint.json](../schemas/v0.1/checkpoint.json) | None (`checkpoints` SQLite row) | None |

Every schema sets `"additionalProperties": true`, and the four Rust record structs include `#[serde(flatten)] pub extensions: BTreeMap<String, serde_json::Value>` so unknown extension fields survive deserialization and serialization.

## Prefixed ULID identifiers

[crates/cortex-core/src/id.rs](../crates/cortex-core/src/id.rs) defines `IdPrefix` and `PrefixedId`. Every identifier consists of a lowercase type prefix ending in `_` followed by a 26-character uppercase Crockford Base32 ULID.

| `IdPrefix` variant | Prefix | Purpose |
| --- | --- | --- |
| `Tombstone` | `tmb_` | Negative-knowledge Tombstone records |
| `Handoff` | `hnd_` | Cross-harness `EpistemicHandoff` envelopes |
| `Invariant` | `inv_` | Causal Invariant Lock contracts |
| `Checkpoint` | `chk_` | Working-tree snapshots (also reused for `execs.id` in `exec.rs`) |
| `Candidate` | `cand_` | Passive-capture failure candidates in SQLite `candidates` |
| `Event` | `evt_` | Append-only `LifecycleEvent` records |

Crockford Base32 allows characters `0-9`, `A-H`, `J-K`, `M-N`, `P-T`, and `V-Z` (omitting `I`, `L`, `O`, and `U`). Schema regexes match `^<prefix>[0-9A-HJKMNP-TV-Z]{26}$`.

`PrefixedId::new` generates IDs through a process-wide `LazyLock<Mutex<ulid::Generator>>` so IDs created within the same millisecond sort in strict lexicographic order, falling back to `Ulid::new()` if `generate()` errors or the mutex is poisoned.

`PrefixedId::parse`, `FromStr`, and `serde::Deserialize` reject lowercase characters, non-Crockford letters (`I`, `L`, `O`, `U`), any ULID suffix whose length is not `26`, and invalid ULID encodings via `Ulid::from_string`.

For distributed Git ledger storage in [ledger.rs](../crates/cortex-storage/src/ledger.rs), `LedgerRecord::shard()` takes the trailing 2 characters of an ID in lowercase (defaulting to `"00"` when length is under `2`), distributing blobs across `1,024` directories at `v1/{tombstones|invariants|events}/{shard}/{id}.json` under `refs/cortex/ledger`.

## Canonical JSON and hashing

### RFC 8785 canonical JSON

[crates/cortex-core/src/jcs.rs](../crates/cortex-core/src/jcs.rs) serializes records with `serde_jcs::to_string` (`to_canonical_json`), which sorts JSON object keys lexicographically and removes optional whitespace per RFC 8785.

`canonical_hash` computes a lowercase SHA-256 hex digest over the canonical JSON bytes and formats it as `sha256:<64hex>`. `sha256_digest` applies the same `sha256:<64hex>` format to raw byte slices such as ledger blobs and file contents checked against `file_reads.blob_oid`.

### Symbol URIs and AST hashes

[uri.rs](../crates/cortex-ast/src/uri.rs) formats symbol references as `<normalized_rel_path>#<scope_chain>`. `normalize_rel_path` converts `\` to `/`, strips leading Windows drive prefixes (`C:`), drops `.` and empty segments, and resolves `..` segments. `parse_symbol_uri` splits on the rightmost `#` with `rsplit_once('#')`.

Scope separators and special scopes:

- Rust (`.rs`) joins nested scopes with `::` (for example `src/pool.rs#Pool::acquire` or `Pool<as Display>::fmt`).
- TypeScript (`.ts`, `.mts`, `.cts`), TSX (`.tsx`), JavaScript (`.js`, `.mjs`, `.cjs`, `.jsx`), Python (`.py`, `.pyi`), and Go (`.go`) join scopes with `.`.
- Unsupported text files and supported files containing zero extracted symbols emit `<path>#<file>` (`SymbolKind::File`).
- Files larger than 2 MiB or containing a NUL byte in the first 1,024 bytes emit `<path>#<binary>` (`SymbolKind::File`).

`ast_hash` strings follow `^ast:sha256:[0-9a-f]{16}$` (the leading 16 lowercase hex characters of SHA-256). [hash.rs](../crates/cortex-ast/src/hash.rs) walks tree-sitter CST nodes, skips `comment`, `line_comment`, `block_comment`, `doc_comment`, `inner_doc_comment`, `outer_doc_comment`, and Python leading module/block docstrings, normalizes leaf tokens via `.trim().replace("\r\n", "\n")`, and hashes typed tree delimiters `(kind:...)` plus length-prefixed leaves. Unsupported `<file>` hashes trim each line, skip blank lines, and join lines with `\n`. Missing files or unresolved `<file>` scopes in `record_tombstone` and `record_invariant` store the sentinel `ast:sha256:0000000000000000`.

### Working tree dirty hash

`CortexRepo::working_tree_dirty_hash` in [repo.rs](../crates/cortex-storage/src/repo.rs#L125-L161) queries non-ignored Git statuses (modified, staged, and untracked). An empty status set produces `"wt:clean"`. Any non-empty status hashes each entry's path, debug status flags, and working-tree file bytes into `"wt:sha256:<16hex>"`.

## Record and envelope schemas

### TombstoneRecord

Schema file: [schemas/v0.1/tombstone.json](../schemas/v0.1/tombstone.json) (`https://git-cortex.dev/schemas/v0.1/tombstone.json`)

| Field | Type | Required | Constraints / Notes |
| --- | --- | --- | --- |
| `$schema` | `string` | Yes | Set to `https://git-cortex.dev/schemas/v0.1/tombstone.json` by `TombstoneRecord::new`. |
| `id` | `string` | Yes | Pattern `^tmb_[0-9A-HJKMNP-TV-Z]{26}$`. |
| `schema_version` | `string` | Yes | Constant `"0.1.0"` in `schema.rs`. |
| `created_at` | `string` | Yes | RFC 3339 `date-time` (`DateTime<Utc>`). |
| `author_harness` | `string` | Yes | Harness identifier (for example `"cortex-cli"`, `"git-cortex/passive-capture"`, `"git-cortex/rewind"`, or MCP `clientInfo`). |
| `branch_context` | `string` | Yes | Branch name when recorded (`"HEAD"` if detached, `"main"` fallback). |
| `base_commit_sha` | `string` | Yes | 40-char Git commit SHA (or 40 zeroes when `HEAD` is unborn). |
| `capture_mode` | `string` | Yes | Enum: `"AUTO_REVERT_DETECTED"`, `"MANUAL"`, `"REWIND_SYNTHESIZED"`, `"HOOK_ENFORCED"`. |
| `target_symbols` | `array` | Yes | Objects with required `symbol_uri` (`string`) and `ast_hash` (`^ast:sha256:[0-9a-f]{16}$`). |
| `attempted_approach` | `string` | Yes | Scrubbed failure approach text (capped to 600 chars when fenced). |
| `failure_reason` | `string` | Yes | Scrubbed failure explanation (capped to 800 chars when fenced). |
| `verification_evidence` | `object` | Yes | Required `command` (`string`), `exit_code` (`integer`), `error_signature` (`string`). |
| `invalidation_policy` | `object` | Yes | Required `invalidate_on_any_ast_change` (`string[]`); optional `status` (`string`, set to `"ACTIVE"` by `TombstoneRecord::new`). |

Example `TombstoneRecord`:

```json
{
  "$schema": "https://git-cortex.dev/schemas/v0.1/tombstone.json",
  "id": "tmb_01JCF8M9K2ABCDEFGHJKMNPQRS",
  "schema_version": "0.1.0",
  "created_at": "2026-11-13T12:00:00Z",
  "author_harness": "cortex-cli",
  "branch_context": "main",
  "base_commit_sha": "b143df9000000000000000000000000000000000",
  "capture_mode": "MANUAL",
  "target_symbols": [
    {
      "symbol_uri": "src/auth/token.rs#verify_jwt",
      "ast_hash": "ast:sha256:9f2c1b0a4d8e7711"
    }
  ],
  "attempted_approach": "Bypassed audience check when clock skew exceeded 5s",
  "failure_reason": "Security regression in token acceptance tests",
  "verification_evidence": {
    "command": "cargo test test_jwt_aud",
    "exit_code": 1,
    "error_signature": "AssertionError: rejected forged aud claim"
  },
  "invalidation_policy": {
    "invalidate_on_any_ast_change": [
      "src/auth/token.rs#verify_jwt"
    ],
    "status": "ACTIVE"
  }
}
```

### EpistemicHandoff and A2A task envelope

Schema file: [schemas/v0.1/handoff.json](../schemas/v0.1/handoff.json) (`https://git-cortex.dev/schemas/v0.1/handoff.json`)

| Field | Type | Required | Constraints / Notes |
| --- | --- | --- | --- |
| `$schema` | `string` | Yes | Set to `https://git-cortex.dev/schemas/v0.1/handoff.json` by `EpistemicHandoff::new`. |
| `handoff_id` | `string` | Yes | Pattern `^hnd_[0-9A-HJKMNP-TV-Z]{26}$`. |
| `schema_version` | `string` | Yes | Constant `"0.1.0"`. |
| `head_commit_sha` | `string` | Yes | 40-char `HEAD` SHA (or 40 zeroes when unborn). |
| `working_tree_dirty_hash` | `string` | Yes | `"wt:clean"` or `"wt:sha256:<16hex>"`. |
| `objective` | `string` | Yes | Task summary; truncated or cleared last under the 4,096-byte budget. |
| `verified_call_graph` | `array` | Yes | Objects with required `caller`, `callee`, `relationship`, `verified_ast_hash` (all `string`). |
| `active_hypotheses` | `array` | Yes | Objects with required `hypothesis_id` (`string`), `statement` (`string`), `confidence` (`number`, `[0.0, 1.0]`, finite in Rust), `status` (`string`), `next_verification_step` (`string`). |
| `active_tombstone_ids` | `array` | Yes | `string[]` ordered from highest to lowest salience. |
| `do_not_re_read_unchanged` | `array` | Yes | `string[]` of relative file paths whose disk `sha256_digest` matches `file_reads.blob_oid`. |

[handoff.rs](../crates/cortex-engine/src/handoff.rs) limits serialized `EpistemicHandoff` payloads to `MAX_HANDOFF_BYTES = 4096` bytes across 7 shedding tiers: (1) clear `do_not_re_read_unchanged`, (2) drop hypotheses with `confidence < 0.5`, (3) drop oldest `verified_call_graph` edges, (4) drop hypotheses with `confidence < 0.8`, (5) pop lowest-salience `active_tombstone_ids`, (6) drop remaining hypotheses, and (7) UTF-8-safe truncate or clear `objective`.

Example `EpistemicHandoff`:

```json
{
  "$schema": "https://git-cortex.dev/schemas/v0.1/handoff.json",
  "handoff_id": "hnd_01JCF8N1A4ABCDEFGHJKMNPQRT",
  "schema_version": "0.1.0",
  "head_commit_sha": "b143df9000000000000000000000000000000000",
  "working_tree_dirty_hash": "wt:clean",
  "objective": "Fix connection pool leak under cancellation",
  "verified_call_graph": [
    {
      "caller": "src/api/handler.rs#serve",
      "callee": "src/pool/conn.rs#acquire",
      "relationship": "CALLS",
      "verified_ast_hash": "ast:sha256:a1b2c3d4e5f60718"
    }
  ],
  "active_hypotheses": [
    {
      "hypothesis_id": "hyp_01",
      "statement": "Dropped futures leak permit counters before Drop runs",
      "confidence": 0.85,
      "status": "ACTIVE",
      "next_verification_step": "Run cargo test pool::cancel_safety"
    }
  ],
  "active_tombstone_ids": [
    "tmb_01JCF8M9K2ABCDEFGHJKMNPQRS"
  ],
  "do_not_re_read_unchanged": [
    "src/api/handler.rs"
  ]
}
```

#### Google A2A task envelope

`format_a2a_task_envelope` in [handoff.rs](../crates/cortex-engine/src/handoff.rs#L301-L323) wraps an `EpistemicHandoff` inside a JSON-RPC 2.0 `tasks/send` request under `params.metadata["https://git-cortex.dev/a2a-extension/v0.1"]`:

```json
{
  "jsonrpc": "2.0",
  "method": "tasks/send",
  "params": {
    "id": "task_handoff",
    "message": {
      "role": "user",
      "parts": [
        {
          "type": "text",
          "text": "Continue development"
        }
      ]
    },
    "metadata": {
      "https://git-cortex.dev/a2a-extension/v0.1": {
        "handoff_id": "hnd_01JCF8N1A4ABCDEFGHJKMNPQRT",
        "epistemic_handoff": {
          "$schema": "https://git-cortex.dev/schemas/v0.1/handoff.json",
          "handoff_id": "hnd_01JCF8N1A4ABCDEFGHJKMNPQRT",
          "schema_version": "0.1.0",
          "head_commit_sha": "b143df9000000000000000000000000000000000",
          "working_tree_dirty_hash": "wt:clean",
          "objective": "Fix connection pool leak under cancellation",
          "verified_call_graph": [],
          "active_hypotheses": [],
          "active_tombstone_ids": [],
          "do_not_re_read_unchanged": []
        }
      }
    }
  }
}
```

### InvariantContract

Schema file: [schemas/v0.1/invariant.json](../schemas/v0.1/invariant.json) (`https://git-cortex.dev/schemas/v0.1/invariant.json`)

| Field | Type | Required | Constraints / Notes |
| --- | --- | --- | --- |
| `$schema` | `string` | Yes | Set to `https://git-cortex.dev/schemas/v0.1/invariant.json`. |
| `invariant_id` | `string` | Yes | Pattern `^inv_[0-9A-HJKMNP-TV-Z]{26}$`. |
| `schema_version` | `string` | Yes | Constant `"0.1.0"`. |
| `commit_sha` | `string` | Yes | Commit SHA at creation or last reaffirmation. |
| `guarded_symbol` | `object` | Yes | Required `symbol_uri` (`string`) and `ast_hash` (`^ast:sha256:[0-9a-f]{16}$`). |
| `severity` | `string` | Yes | Enum: `"BLOCKING"`, `"WARNING"`. |
| `assertion` | `string` | Yes | Architectural contract description (capped to 800 chars when fenced). |
| `upstream_dependencies` | `array` | Yes | Objects with required `symbol_uri` (`string`) and `expected_ast_hash` (`^ast:sha256:[0-9a-f]{16}$`). |
| `verification_test_cmd` | `string` | Yes | Behavioral verification command (capped to 300 chars when fenced; executed with 30s timeout by `hook pre-commit` on drifted `BLOCKING` contracts). |

Example `InvariantContract`:

```json
{
  "$schema": "https://git-cortex.dev/schemas/v0.1/invariant.json",
  "invariant_id": "inv_01JCF8P2B5ABCDEFGHJKMNPQRV",
  "schema_version": "0.1.0",
  "commit_sha": "b143df9000000000000000000000000000000000",
  "guarded_symbol": {
    "symbol_uri": "src/api/handler.rs#serve",
    "ast_hash": "ast:sha256:1122334455667788"
  },
  "severity": "BLOCKING",
  "assertion": "handler::serve relies on acquire() returning RAII guard",
  "upstream_dependencies": [
    {
      "symbol_uri": "src/pool/conn.rs#acquire",
      "expected_ast_hash": "ast:sha256:a1b2c3d4e5f60718"
    }
  ],
  "verification_test_cmd": "cargo test pool::raii_contract"
}
```

### LifecycleEvent

Schema file: [schemas/v0.1/event.json](../schemas/v0.1/event.json) (`https://git-cortex.dev/schemas/v0.1/event.json`)

| Field | Type | Required | Constraints / Notes |
| --- | --- | --- | --- |
| `$schema` | `string` | Yes | Set to `https://git-cortex.dev/schemas/v0.1/event.json`. |
| `event_id` | `string` | Yes | Pattern `^evt_[0-9A-HJKMNP-TV-Z]{26}$`. |
| `target_id` | `string` | Yes | Target `tmb_...` or `inv_...` record ID (no regex in schema). |
| `event_type` | `string` | Yes | Enum: `"REVOKE"`, `"REAFFIRM"`, `"SUPERSEDE"`. |
| `reason` | `string` | Yes | Human or agent justification for the lifecycle transition. |
| `created_at` | `string` | Yes | RFC 3339 `date-time` (`DateTime<Utc>`). |
| `author` | `string` | Yes | Actor string (for example `"cortex-cli"` or `"git-cortex/pre-commit"`). |

Example `LifecycleEvent`:

```json
{
  "$schema": "https://git-cortex.dev/schemas/v0.1/event.json",
  "event_id": "evt_01JCF8Q3C6ABCDEFGHJKMNPQRW",
  "target_id": "inv_01JCF8P2B5ABCDEFGHJKMNPQRV",
  "event_type": "REAFFIRM",
  "reason": "Auto-reaffirmed via passing behavioral verification test",
  "created_at": "2026-11-13T12:10:00Z",
  "author": "git-cortex/pre-commit"
}
```

### CheckpointRecord

Schema file: [schemas/v0.1/checkpoint.json](../schemas/v0.1/checkpoint.json) (`https://git-cortex.dev/schemas/v0.1/checkpoint.json`)

| Field | Type | Required | Constraints / Notes |
| --- | --- | --- | --- |
| `$schema` | `string` | Yes | Schema URI (`https://git-cortex.dev/schemas/v0.1/checkpoint.json`). |
| `checkpoint_id` | `string` | Yes | Pattern `^chk_[0-9A-HJKMNP-TV-Z]{26}$`. |
| `branch` | `string` | Yes | Branch name when checkpoint was created. |
| `snapshot_ref` | `string` | Yes | Schema pattern `^refs/cortex/snapshots/checkpoints/[0-9a-f]{40}$` (see alignment notes below). |
| `objective` | `string` | Yes | Task objective string. |
| `state` | `object` | Yes | Required `edge_count` (`integer >= 0`) and `hypothesis_count` (`integer >= 0`); `additionalProperties: true`. |
| `created_at` | `string` | Yes | RFC 3339 `date-time`. |

Example `CheckpointRecord` conforming to `checkpoint.json`:

```json
{
  "$schema": "https://git-cortex.dev/schemas/v0.1/checkpoint.json",
  "checkpoint_id": "chk_01JCF8R4D7ABCDEFGHJKMNPQRX",
  "branch": "main",
  "snapshot_ref": "refs/cortex/snapshots/checkpoints/0123456789abcdef0123456789abcdef01234567",
  "objective": "Verify retry backoff before refactor",
  "state": {
    "edge_count": 2,
    "hypothesis_count": 1
  },
  "created_at": "2026-11-13T12:20:00Z"
}
```

## SQLite runtime tables and core types

`CortexDb::open` opens `<common_git_dir>/cortex/state.db` with `PRAGMA journal_mode = WAL`, `PRAGMA busy_timeout = 5000`, and `PRAGMA foreign_keys = ON` (`PRAGMA synchronous = NORMAL` is not set). When `PRAGMA user_version < 1`, [migrations.rs](../crates/cortex-storage/src/migrations.rs) runs one `Immediate` transaction creating 9 tables, 6 indexes, and setting `user_version = 1`.

| Table | Columns and keys | Role |
| --- | --- | --- |
| `records` | `id TEXT PRIMARY KEY`, `kind TEXT`, `json TEXT`, `origin TEXT`, `promoted INTEGER DEFAULT 0`, `branch TEXT`, `created_at TEXT`, `content_hash TEXT` | Stores `TOMBSTONE`, `INVARIANT`, and `EVENT` records (`origin` is `"LOCAL"` or `"LEDGER"`). |
| `record_symbols` | `record_id TEXT REFERENCES records(id) ON DELETE CASCADE`, `symbol_uri TEXT`, `path TEXT`, `ast_hash TEXT`, `role TEXT` (no primary key) | Inverted index mapping records to `TARGET`, `GUARDED`, and `UPSTREAM_DEP` symbols. |
| `symbol_cache` | `path TEXT`, `blob_oid TEXT`, `symbols_json TEXT`, `PRIMARY KEY(path, blob_oid)` | Defined with DB accessors in `db.rs`; runtime code uses in-memory `SymbolCache` instead. |
| `execs` | `id TEXT PRIMARY KEY`, `cmd TEXT`, `exit_code INTEGER`, `signature TEXT`, `output_tail TEXT`, `tree_hash TEXT`, `started_at TEXT`, `duration_ms INTEGER` | Stores `cortex exec` runs (`id` uses `chk_`; `output_tail` stores up to 4,096 UTF-8 bytes). |
| `candidates` | `id TEXT PRIMARY KEY`, `state TEXT`, `base_commit TEXT`, `symbols_json TEXT`, `diff TEXT`, `exec_ids TEXT`, `created_at TEXT` | Passive-capture `FAILED` and `PROMOTED` revert candidates (`cand_`). |
| `checkpoints` | `id TEXT PRIMARY KEY`, `branch TEXT`, `snapshot_ref TEXT`, `objective TEXT`, `state_json TEXT`, `created_at TEXT` | Stores checkpoint metadata from `checkpoint_state` (`snapshot_ref` holds `refs/cortex/checkpoints/chk_<ULID>`). |
| `file_reads` | `path TEXT`, `blob_oid TEXT`, `branch TEXT`, `read_at TEXT`, `PRIMARY KEY(path, branch)` | Tracks `sha256:<64hex>` digests for `handoff` `do_not_re_read_unchanged`. |
| `call_graph_edges` | `caller TEXT`, `callee TEXT`, `relationship TEXT`, `verified_ast_hash TEXT`, `branch TEXT`, `PRIMARY KEY(caller, callee, branch)` | Branch-scoped verified call-graph edges populated by `checkpoint` or `handoff import`. |
| `hypotheses` | `hypothesis_id TEXT`, `statement TEXT`, `confidence REAL`, `status TEXT`, `next_verification_step TEXT`, `branch TEXT`, `PRIMARY KEY(hypothesis_id, branch)` | Branch-scoped active hypotheses populated by `checkpoint` or `handoff import`. |

Indexes created in version `1`: `idx_rs_path` (`record_symbols(path)`), `idx_rs_symbol_uri` (`record_symbols(symbol_uri)`), `idx_rs_ast_hash` (`record_symbols(ast_hash)`), `idx_records_kind_branch` (`records(kind, branch)`), `idx_record_symbols_record_id` (`record_symbols(record_id)`), and `idx_record_symbols_path_role` (`record_symbols(path, role)`).

Core runtime types outside the 5 JSON Schemas:

- `StoredRecord` ([db.rs](../crates/cortex-storage/src/db.rs#L13-L22)): row wrapper serialized by `tombstone list --json` and `invariant list --json` (`id`, `kind`, `json` string, `origin`, `promoted` boolean, `branch`, `created_at`, `content_hash`).
- `DerivedStatus` ([schema.rs](../crates/cortex-core/src/schema.rs#L307-L327)): `Active` (`"ACTIVE"`), `Stale { changed_symbols }` (`"STALE"`), `Orphaned { missing_symbols }` (`"ORPHANED"`), and `Revoked { reason }` (`"REVOKED"`).

## Schema and implementation alignment notes

1. `tombstone.json` `invalidation_policy.status` vs query-time evaluation: `tombstone.json` permits an optional `invalidation_policy.status` string, and `TombstoneRecord::new` writes `Some("ACTIVE")`. At query time, `inspect_symbol` ignores `invalidation_policy.status`, evaluates live AST hashes and `REVOKE` events dynamically, and classifies missing symbols under `stale_tombstones` and `stale_invariants` without emitting `DerivedStatus::Orphaned`.
2. `event.json` and `LifecycleEvent` differences: `event.json` and `LifecycleEvent` omit `schema_version`, whereas `tombstone.json`, `handoff.json`, and `invariant.json` require it. `event.json` defines no regex on `target_id`. Both `event.json` and `EventType` define `"SUPERSEDE"`, which has no producer or consumer in `crates/cortex-engine`.
3. `handoff.json` regex gaps and confidence checks: `handoff.json` omits regex patterns for `verified_call_graph[].verified_ast_hash` (`^ast:sha256:[0-9a-f]{16}$`), `working_tree_dirty_hash`, and `active_tombstone_ids[]`. Both `handoff.json` and Rust (`ActiveHypothesis::validate` and `deserialize_confidence`) enforce `confidence` in `[0.0, 1.0]`, and Rust also rejects `NaN` and `Infinity`. `EpistemicHandoff` records are transient and never stored in SQLite `records` or `refs/cortex/ledger`.
4. `checkpoint.json` `snapshot_ref` mismatch and storage format: `checkpoint.json` enforces `^refs/cortex/snapshots/checkpoints/[0-9a-f]{40}$`, while `CortexSnapshot::create_snapshot` in [snapshot.rs](../crates/cortex-storage/src/snapshot.rs#L18) writes `refs/cortex/checkpoints/chk_<26-ULID>` (and `refs/cortex/backups/...`, `refs/cortex/untracked/...`). [checkpoint.rs](../crates/cortex-engine/src/checkpoint.rs#L48-L53) writes `{"edge_count": ..., "hypothesis_count": ...}` into SQLite `checkpoints.state_json` and never emits a `CheckpointRecord` JSON object with `$schema`.
5. `execs.id` prefix reuse: `execute_command` in [exec.rs](../crates/cortex-capture/src/exec.rs#L160) generates `execs.id` with `IdPrefix::Checkpoint` (`chk_`); no dedicated execution prefix exists.
6. `PrefixedId` vs `String` fields in `schema.rs`: `TombstoneRecord`, `EpistemicHandoff`, `InvariantContract`, and `LifecycleEvent` store ID fields as `String`, so JSON deserialization of those structs accepts any string unless validated by `PrefixedId::parse` or an external Draft 2020-12 validator.
