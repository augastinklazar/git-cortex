# Epistemic Context Protocol (ECP v0.1)

Normative specification for Git-native epistemic working memory, negative-knowledge tombstones, and causal invariant locks in multi-agent software engineering.

## 1. Specification attributes and versioning

This document defines ECP v0.1 (`schema_version = "0.1.0"`). Its companion JSON Schema files live under `schemas/v0.1/`, and the reference implementation is `git-cortex` `0.1.0`.

| Attribute | Value |
|---|---|
| Protocol version | `ECP v0.1` (`schema_version: "0.1.0"`) |
| Reference binaries | `git-cortex` and `cortex` (v0.1.0) |
| Target interoperability | Model Context Protocol (`2024-11-05`), Google Agent2Agent (`tasks/send` extension), `AGENTS.md`, `CLAUDE.md` |
| Storage architecture | Tier 1: SQLite WAL index (`state.db`); Tier 2: orphan Git commit tree (`refs/cortex/ledger`) |
| AST addressing | Tree-sitter symbol URIs paired with 64-bit truncated SHA-256 subtree digests (`ast:sha256:<16hex>`) |
| Canonical serialization | RFC 8785 JSON Canonicalization Scheme (JCS) with `sha256:<64hex>` content digests |

## 2. Architectural scope

Autonomous coding agents encounter three recurring failure modes in multi-step repository work:

1. Context compaction drops negative knowledge first. When session history is summarized, failed refactorings and deadlock causes vanish, and the agent repeats the same dead end.
2. Sub-agent delegation and harness switches incur re-reading overhead. Incoming agents spend substantial token budgets reconstructing verified call graphs and open hypotheses from scratch.
3. Line-based Git merges miss semantic drift. When an upstream dependency changes runtime preconditions while keeping syntax valid, Git merges cleanly while introducing silent bugs downstream.

ECP v0.1 preserves three corresponding artifacts outside tracked source files: `TombstoneRecord` (disproved hypotheses and failed edits), `EpistemicHandoff` (bounded working-state transfers), and `InvariantContract` (cross-file behavioral preconditions anchored to AST hashes).

## 3. Two-tier storage architecture

ECP v0.1 separates high-frequency local working state from immutable repository history:

```text
+-------------------------------------------------------------------+
| Tier 1: Local SQLite WAL State (.git/cortex/state.db)             |
| - Branch-scoped cache, candidates, execs, edges, hypotheses       |
| - Shared via <commondir>/cortex/state.db across linked worktrees  |
| - Rebuilt from Tier 2 via `git cortex reindex`                    |
+---------------------------------+---------------------------------+
                                  | promote / post-commit / sync
                                  v
+-------------------------------------------------------------------+
| Tier 2: Distributed Ledger (refs/cortex/ledger)                   |
| - Orphan Git commit graph holding canonical RFC 8785 JSON         |
| - Tree path: v1/<kind>/<shard>/<id>.json                          |
| - Three-way set-union merge aborts on conflicting blob OIDs       |
+-------------------------------------------------------------------+
```

### 3.1 Ledger reference design (`refs/notes/cortex` to `refs/cortex/ledger`)

Early protocol drafts proposed storing records under `refs/notes/cortex` alongside sub-namespaces such as `refs/notes/cortex/tombstones`. Two properties of Git make `refs/notes/*` unsuitable:

1. Loose ref directory/file conflicts: Git stores unpacked refs as paths under `.git/refs/`, so `refs/notes/cortex` cannot coexist as a file alongside `refs/notes/cortex/tombstones`.
2. Commit-keyed indexing: Git notes map commit SHAs to blobs, which forces O(N) scans across commits to look up records by symbol URI.

ECP v0.1 resolves both constraints by storing distributed records in an independent orphan commit history at `refs/cortex/ledger`. Remote synchronization uses the fetch refspec `+refs/cortex/ledger:refs/remotes/origin/cortex/ledger`.

Records inside the ledger tree use the path layout `v1/<kind>/<shard>/<id>.json`, where `<kind>` is `tombstones`, `invariants`, or `events`, and `<shard>` is the trailing two lowercase characters of `<id>` (across 1,024 Crockford Base32 shards; `"00"` if shorter than two characters). Because every record ID is a unique prefixed ULID, concurrent merges perform a set-union across tree paths without text merge conflicts. If the same tree path maps to different Git blob OIDs on local and remote ledgers, merge aborts with a tamper alert error.

### 3.2 Implementation notes (v0.1.0): storage

- Tier 1 pragmas in `crates/cortex-storage/src/migrations.rs` set `journal_mode = WAL`, `busy_timeout = 5000`, and `foreign_keys = ON` inside a `TransactionBehavior::Immediate` schema migration (`PRAGMA user_version = 1`). `PRAGMA synchronous = NORMAL` is omitted in v0.1.0, leaving SQLite WAL at its default synchronous mode.
- SQLite contains 9 tables (`records`, `record_symbols`, `symbol_cache`, `execs`, `candidates`, `checkpoints`, `file_reads`, `call_graph_edges`, `hypotheses`) and 6 indexes. `symbol_cache` has DB read/write helpers while runtime extraction uses the in-memory `SymbolCache`.
- `EpistemicHandoff` payloads are never written to `records` or `refs/cortex/ledger`; they are exported on demand.
- `CortexLedger::append_records` uses a 5-attempt compare-and-swap retry loop (`10 * (attempt + 1)` ms backoff). `sync` and `sync --rebase` retry remote pushes up to `MAX_SYNC_RETRIES = 5` with `50 * (1 << attempt)` ms backoff. `CortexLedger::reindex_into_db` writes 500-record `Immediate` batches and splits symbol URIs on `.split('#').next()` in v0.1.0.
- `reaffirm_invariant` overwrites the existing `INVARIANT` row in SQLite with `promoted = false` before emitting an `EVENT` record. Independently reaffirming and promoting the same `inv_...` record on two branches creates conflicting blob OIDs at the same ledger path and triggers a tamper alert during `git cortex sync`.
- Working-tree snapshots write under `refs/cortex/checkpoints/<id>`, `refs/cortex/backups/<id>`, and `refs/cortex/untracked/<id>` using an isolated temporary index `tmp_index_<id>` placed in `<repo.path()>/cortex/` (in v0.1.0, `snapshot.rs` joins `repo.path().join("cortex")` directly while `state.db` uses `resolve_commondir`).
- `execute_command` mints `execs.id` using `IdPrefix::Checkpoint` (`chk_`) in v0.1.0.

## 4. Deterministic AST symbol addressing (`ast_hash`)

Because line offsets move on unrelated edits, ECP v0.1 binds records to canonical symbol URIs and structural AST hashes.

### 4.1 Symbol URI grammar

Each symbol URI has the form `<relative_path>#<scope_chain>`:

1. Path normalization (`normalize_rel_path`) converts `\` to `/`, strips Windows drive prefixes (`C:` or `c:`), drops empty and `.` segments, and pops `..` segments. Parsers split URIs at the rightmost `#` (`rsplit_once('#')`).
2. Scope separators follow language conventions: `::` for Rust (`src/pool/connection.rs#ConnectionPool::acquire`) and `.` for TypeScript, TSX, JavaScript, Python, and Go (`src/auth/token_manager.ts#TokenManager.refreshSession`, `app/worker.py#Worker.process`, `pkg/queue/worker.go#Worker.Dispatch`).
3. Language-specific scoping rules cover Rust `mod_item`, `struct_item`, `enum_item`, `trait_item`, and `impl_item` (stripping generic `<...>` arguments and formatting trait impls as `Base<as Trait>`), Go pointer receiver stripping (`*ConnectionPool` becomes `ConnectionPool`), and ordinal overload suffixes `[2]`, `[3]` for duplicate scopes in one file.
4. Fallbacks handle non-AST inputs: unsupported extensions or supported files with zero extracted symbols emit `<relative_path>#<file>`, and files exceeding 2 MiB (`2 * 1024 * 1024` bytes) or containing `\0` in the first 1,024 bytes emit `<relative_path>#<binary>`.

### 4.2 Normalized subtree hash calculation

For each extracted symbol subtree, `compute_ast_hash_for_lang` builds a SHA-256 digest truncated to 64 bits (`ast:sha256:<16-hex-chars>`):

1. Skip comment nodes (`comment`, `line_comment`, `block_comment`, `doc_comment`, `inner_doc_comment`, `outer_doc_comment`).
2. For Python symbols below the root node, skip leading docstrings (`expression_statement` whose first named child is `string` or `concatenated_string` and which is the first non-comment named child of `block` or `module`).
3. Visit every child `0..node.child_count()`, encoding internal nodes as `(<kind>:...children...)`.
4. For leaf nodes, trim whitespace, normalize `\r\n` to `\n`, skip empty strings, and hash `<kind>:<len>:<norm>;` (named leaves) or `tok:<len>:<norm>;` (unnamed tokens such as operators and delimiters).

Because comments, Python docstrings, indentation, blank lines, and CRLF/LF line endings are omitted or normalized, those edits keep `ast_hash` unchanged. Because named leaves include identifier tokens verbatim, renaming variables, functions, or parameters changes `ast_hash`.

For `#<file>` fallbacks, `compute_text_fallback_hash` trims every line, discards empty lines, joins with `\n`, and truncates SHA-256 to 16 hex chars. For `#<binary>` fallbacks, `extract_symbols` truncates the SHA-256 digest of the raw file bytes to 16 hex chars.

### 4.3 Query-time status evaluation

ECP v0.1 computes record status at query time by comparing stored AST hashes against live symbol extraction and reading `EVENT` records:

- `ACTIVE`: all bound symbols exist at their URIs with matching `ast_hash`, and no `REVOKE` event targets the record ID.
- `STALE`: one or more bound symbols changed `ast_hash` or are missing from the current file.
- `REVOKED`: an append-only `REVOKE` event targets the record ID.

### 4.4 Implementation notes (v0.1.0): AST and status

- `DerivedStatus::Orphaned` (`"ORPHANED"`) is defined in `crates/cortex-core/src/schema.rs`, though `inspect_symbol` never emits it. Missing files and deleted symbols appear under `stale_tombstones` and `stale_invariants` (`<!-- STALE ... -->`).
- `EventType::Supersede` (`"SUPERSEDE"`) exists in `schema.rs` and `schemas/v0.1/event.json`, though no v0.1.0 code writes or evaluates `SUPERSEDE` events.
- Rust `function_signature_item` nodes (trait methods without bodies) are not extracted, and function bodies do not push onto `scope_stack`.

## 5. Canonical serialization, identifiers, and schemas

### 5.1 Prefixed ULIDs and RFC 8785 JCS

Every ECP record identifier combines an entity prefix with a 26-character monotonic ULID encoded in uppercase Crockford Base32 (`0-9 A-H J-K M-N P-T V-Z`):

| Prefix | Record type |
|---|---|
| `tmb_` | `TombstoneRecord` |
| `hnd_` | `EpistemicHandoff` |
| `inv_` | `InvariantContract` |
| `chk_` | Working-tree checkpoint (and `execs.id` in v0.1.0) |
| `cand_` | Passive-capture candidate failure |
| `evt_` | `LifecycleEvent` |

All persistent records serialize via RFC 8785 JSON Canonicalization Scheme (`serde_jcs::to_string`) before SHA-256 content hashing (`sha256:<64-hex-chars>`).

### 5.2 Normative data schemas

Five JSON Schema (Draft 2020-12) documents define the v0.1 data structures under `https://git-cortex.dev/schemas/v0.1/`:

| Record type | Schema ID | Repository file |
|---|---|---|
| `TombstoneRecord` | `https://git-cortex.dev/schemas/v0.1/tombstone.json` | [tombstone.json](../../schemas/v0.1/tombstone.json) |
| `EpistemicHandoff` | `https://git-cortex.dev/schemas/v0.1/handoff.json` | [handoff.json](../../schemas/v0.1/handoff.json) |
| `InvariantContract` | `https://git-cortex.dev/schemas/v0.1/invariant.json` | [invariant.json](../../schemas/v0.1/invariant.json) |
| `LifecycleEvent` | `https://git-cortex.dev/schemas/v0.1/event.json` | [event.json](../../schemas/v0.1/event.json) |
| `CheckpointRecord` | `https://git-cortex.dev/schemas/v0.1/checkpoint.json` | [checkpoint.json](../../schemas/v0.1/checkpoint.json) |

### 5.3 Implementation notes (v0.1.0): schemas

- `TombstoneRecord::new` sets `invalidation_policy.status = Some("ACTIVE")`, and `tombstone.json` retains an optional `status` string field even though actual status is evaluated dynamically at query time.
- `LifecycleEvent` in `schema.rs` and `schemas/v0.1/event.json` omit `schema_version`, and `event.json` has no regex constraint on `target_id`.
- `schemas/v0.1/handoff.json` omits a regex on `verified_ast_hash` (`ast:sha256:[0-9a-f]{16}`), while `schema.rs` enforces finite `confidence` within `[0.0, 1.0]` in `ActiveHypothesis::new`, `validate`, and Serde deserialization.
- `schemas/v0.1/checkpoint.json` specifies `"snapshot_ref": { "pattern": "^refs/cortex/snapshots/checkpoints/[0-9a-f]{40}$" }`. In v0.1.0, `checkpoint_state` writes `refs/cortex/checkpoints/chk_<26-ULID>` and persists `{edge_count, hypothesis_count}` in SQLite `checkpoints.state_json` without emitting `CheckpointRecord` JSON or `$schema`.

## 6. Model Context Protocol (MCP) interface

`git-cortex mcp-serve` implements JSON-RPC 2.0 over `stdio` (`protocolVersion: "2024-11-05"`, `capabilities: {"tools":{}}`, `serverInfo: {"name":"git-cortex","version":"0.1.0"}`). Transport framing supports both `Content-Length` headers and newline-delimited JSON per request. Notifications without an `id` receive no response.

The protocol exposes six normative tools:

| Tool | Required parameters | Optional parameters and behavior |
|---|---|---|
| `cortex_inspect_symbol` | `filepath` | `symbol_name`. Returns sanitized fenced markdown for active records plus stale/revoked comments. |
| `cortex_record_tombstone` | `target_symbols`, `attempted_approach`, `failure_reason` | `test_command` (`""`), `error_signature` (`""`). Scrubs secrets and validates symbol URIs in existing files. |
| `cortex_checkpoint_state` | `objective` | `verified_edges`, `active_hypotheses`. Writes `refs/cortex/checkpoints/<id>` and saves branch state in SQLite. |
| `cortex_handoff_pack` | `mode` (`IMPORT` or `EXPORT`) | `branch`, `envelope`. `EXPORT` compacts working state to <= 4,096 bytes. `IMPORT` ingests edges and hypotheses. |
| `cortex_verify_invariants` | None | `diff_scope` (`STAGED`, `WORKING_TREE` default, `BRANCH`). Performs static AST checks. |
| `cortex_record_invariant` | `guarded_symbol`, `assertion`, `verification_test_cmd` | `upstream_dependencies`, `severity` (`BLOCKING` default, `WARNING`). |

`EpistemicHandoff` compaction enforces the 4,096-byte JSON ceiling (`MAX_HANDOFF_BYTES`) by ranking tombstones with `0.50 * prox_score + 0.35 * bm25_score + 0.15 * decay_score` (`prox_score` in `{1.0, 0.7, 0.3, 0.1}`; `decay_score = 1.0` in v0.1.0) and shedding lower-priority fields across seven ordered tiers (`do_not_re_read_unchanged`, `confidence < 0.5` hypotheses, oldest `verified_call_graph` edges, `confidence < 0.8` hypotheses, lowest-salience `active_tombstone_ids`, remaining hypotheses, and UTF-8 `objective` truncation).

For Google A2A transport (`git cortex handoff export --a2a`), `format_a2a_task_envelope` emits JSON-RPC 2.0 `method: "tasks/send"` with `handoff_id` and `epistemic_handoff` placed under `params.metadata["https://git-cortex.dev/a2a-extension/v0.1"]`.

### 6.1 Implementation notes (v0.1.0): MCP

- Calling `cortex_handoff_pack` with `mode = "IMPORT"` without an `envelope` object returns a tool error (`"Missing 'envelope' payload for IMPORT mode"`).
- Every `tools/call` dispatch invokes `reconcile_candidates` synchronously before running the requested tool.
- `verify_invariants` with `DiffScope::Branch` reads files from `repo.head()` tree without computing a base-branch diff, and `read_file_for_scope(_, true)` falls back to working-tree bytes if a path is absent from the Git index.

## 7. Passive capture (`git cortex exec`)

`git cortex exec [--timeout <secs>] -- <command...>` observes failing edits without requiring explicit agent logging:

1. Before spawning the command, `find_dirty_symbols` compares tracked modified files against `HEAD` to record touched symbol URIs and AST hashes.
2. Output is drained concurrently from stdout and stderr, printed to the terminal, and summarized by `extract_error_signature` (reverse line scan for `"error:"`, `"fail"`, `"assertion"`, `"panic"`, `"timeout"`, `"exception"`, `"traceback"`, normalizing `0x[0-9a-fA-F]+` to `0x<HEX>` and `\d{1,2}:\d{2}(:\d{2})?` to `<TIME>`, capped at 300 UTF-8 bytes).
3. Up to 4,096 UTF-8 bytes of trailing output are saved to `execs.output_tail`.
4. If the command exits non-zero while tracked symbols are dirty and no clean-tree (`wt:clean`) execution shares the same `(cmd, signature)`, a `FAILED` row is saved to `candidates`.
5. When `reconcile_candidates` detects that touched symbols reverted to `base_ast_hash`, those reverted symbols promote into an `AUTO_REVERT_DETECTED` tombstone; partial reverts keep the remaining unreverted symbols in the `FAILED` candidate.

### 7.1 Implementation notes (v0.1.0): passive capture

- Concurrent pipe readers call `read_to_end` into unbounded memory before `safe_tail_utf8` extracts the 4,096-byte `output_tail` persisted to SQLite.
- Timeout enforcement (`timeout_secs > 0`) sends `SIGKILL` to the child process group on Unix or runs `taskkill /F /T /PID` on Windows and returns exit code `124`.

## 8. Checkpoints and split-brain safe rollback (`git cortex rewind`)

Running `git cortex rewind <checkpoint_id> [-r <reason>]` performs five steps:

1. Back up dirty working-tree state to `refs/cortex/backups/<new_chk_id>`.
2. Diff `<checkpoint_id>` tree against working tree (`diff_tree_to_workdir_with_index`) to collect modified and newly added symbol URIs.
3. Stash untracked files under `refs/cortex/untracked/<checkpoint_id>`.
4. Restore working tree and index to `<checkpoint_id>` with `force().remove_untracked(true)`.
5. Record a `CaptureMode::RewindSynthesized` tombstone (`command = "cortex rewind"`, `error_signature = "ABORTED_WORKTREE_REWIND"`) bound to the modified symbols.

### 8.1 Implementation notes (v0.1.0): rewind

- In v0.1.0, `rewind` inserts the synthesized tombstone into SQLite `state.db` without attaching metadata to the checkpoint commit object.
- `find_modified_symbols_against_checkpoint` skips deleted files and deleted symbols (`if !full_path.exists() { continue; }`).
- `rewind_to_checkpoint` restores files and indexes without rolling back rows in SQLite `call_graph_edges` or `hypotheses`.

## 9. Security defenses and execution boundaries

### 9.1 Verification command execution boundary

- Read-only queries (`cortex_inspect_symbol`, `cortex_verify_invariants`, `git cortex verify-invariants`) never execute `verification_test_cmd` or `reproduction_cmd`.
- Implementation note (v0.1.0): the `pre-commit` Git hook installed by `git cortex init` calls `verify_and_reaffirm_invariants(repo, db, DiffScope::Staged, true)`. When a `BLOCKING` invariant drifts and specifies a non-empty `verification_test_cmd`, the hook executes that command via `sh -c` (Unix) or `cmd /C` (Windows) with a 30-second timeout; exit code `0` triggers automatic `reaffirm_invariant`. Always inspect `.git/hooks/pre-commit` and ledger invariants before committing inside untrusted repositories.

### 9.2 Prompt injection fencing (`cortex_core::fence`)

Before rendering records to LLM context, `fence_tombstone` and `fence_invariant` call `sanitize_text` and enforce character caps:

- Stripped sequences: 7-bit (`\x1B[...`) and 8-bit (`\u{009B}`) CSI, OSC/DCS/APC/PM/SOS (`\x1B]`, `P`, `_`, `^`, `X` up to BEL/`\x07`, `\u{009C}`, or ST `\x1B\\`), single-character ESC sequences, `\0`, `\r`, C0/C1 control characters other than `\n` and `\t`, Bidi controls (`\u{061C}`, `\u{200B}..=\u{200F}`, `\u{202A}..=\u{202E}`, `\u{2060}..=\u{2069}`, `\u{FEFF}`), and Unicode Tags (`\u{E0000}..=\u{E007F}`).
- Neutralized delimiters: `<!--` becomes `< !--`, `-->` becomes `-- >`, `CORTEX_TOMBSTONE_` becomes `CORTEX_TMB_`, and `CORTEX_INVARIANT_` becomes `CORTEX_INV_`.
- Length caps: `author_harness` 100 chars, `error_signature` and `verification_test_cmd` 300 chars, `attempted_approach` 600 chars, `failure_reason` and `assertion` 800 chars.
- Fencing format: `<!-- CORTEX_TOMBSTONE_BEGIN id="..." provenance="..." harness="..." --> ... <!-- CORTEX_TOMBSTONE_END id="..." -->` and `<!-- CORTEX_INVARIANT_BEGIN id="..." severity="..." provenance="..." --> ... <!-- CORTEX_INVARIANT_END id="..." -->` using `Provenance` tags `LOCAL_WORKING_TREE`, `LOCAL_CHECKPOINT`, `LEDGER_COMMITTED`, or `REMOTE_UNTRUSTED`.

### 9.3 Secret redaction (`cortex_core::redact`)

`scrub` runs 13 compiled `REDACTION_RULES` (AWS `AKIA`/`ASIA` -> `[REDACTED:AWS_KEY]`, GitHub classic -> `[REDACTED:GITHUB_TOKEN]`, GitHub fine-grained 82-char PAT -> `[REDACTED:GITHUB_PAT]`, Bearer tokens -> `Bearer [REDACTED:BEARER_TOKEN]`, generic secret key/value -> `$1=[REDACTED:SECRET]`, PEM private keys -> `[REDACTED:PRIVATE_KEY]`, credentialed URLs -> `${1}[REDACTED_CREDS]@`, Slack -> `[REDACTED:SLACK_TOKEN]`, OpenAI/Anthropic/generic `sk-` -> `[REDACTED:API_KEY]`, Google API keys -> `[REDACTED:GOOGLE_API_KEY]`), followed by a high-entropy scanner (`[REDACTED:HIGH_ENTROPY]` for non-hex words >= 32 chars with uppercase, lowercase, digits, Shannon entropy > 3.5 bits/char, and `looks_like_camel_identifier` filtering).

Implementation note (v0.1.0): `REDACTION_RULES` has no dedicated `eyJ...` JWT rule yet.

### 9.4 Path containment and tamper detection

`normalize_rel_path` removes traversal segments, and `CortexError::PathOutOfBounds` covers out-of-root paths. Canonical JCS SHA-256 content hashes and Git tree OID checks detect modified ledger records during `merge_commits`.

## 10. Related documents

- [Concepts and storage internals](../CONCEPTS-AND-STORAGE.md)
- [System and crate architecture](../ARCHITECTURE.md)
- [Schemas reference](../SCHEMAS.md)
- [MCP server and integrations](../MCP-AND-INTEGRATION.md)
- [CLI reference](../CLI-REFERENCE.md)
- [Security policy](../../SECURITY.md)
