# Frequently asked questions

## How does git-cortex compare to AGENTS.md and code comments?

`AGENTS.md` files and inline code comments are static text checked into tracked files. They stay unchanged after code edits and drop out of context when a long session compacts.

`git-cortex` stores negative-knowledge tombstones and causal invariant contracts in `.git/cortex/state.db` and `refs/cortex/ledger`, keyed to normalized Tree-sitter AST digests (`ast:sha256:<16hex>`). When a target symbol changes structurally, `cortex_inspect_symbol` detects the hash mismatch on demand and marks the record stale. When a `BLOCKING` invariant's upstream dependency drifts, `verify-invariants` flags the contract and the `pre-commit` hook checks whether the contract's verification test passes before allowing the commit.

## Does git-cortex modify tracked source files?

Normal inspection, command execution (`exec`), tombstone and invariant creation, checkpoints, promotion, and ledger sync never edit tracked source code files. Runtime records live in `.git/cortex/state.db` and under `refs/cortex/*` inside the Git object database.

Two commands write to the working tree:

- `git cortex init` writes or appends sections to `AGENTS.md`, `CLAUDE.md`, and `.mcp.json` in the repository root. On the `v0.1.0` CLI, `--hooks`, `--agents-md`, and `--mcp` default to `true` with no `--no-*` opt-out flags, so delete or revert those files if you only wanted local `.git/cortex/` initialization.
- `git cortex rewind <checkpoint_id>` snapshots dirty tracked files to `refs/cortex/backups/<new_chk_id>` and untracked files to `refs/cortex/untracked/<checkpoint_id>`, then performs a forced checkout (`remove_untracked(true)`) back to the checkpoint tree.

See [Getting started](GETTING-STARTED.md) and [CLI reference](CLI-REFERENCE.md) for command details.

## Which languages does git-cortex support, and what happens for other files?

`crates/cortex-ast` parses six language families with Tree-sitter:

- Rust (`.rs`, scope separator `::`)
- TypeScript (`.ts`, `.mts`, `.cts`, scope separator `.`)
- TSX (`.tsx`, scope separator `.`)
- JavaScript (`.js`, `.mjs`, `.cjs`, `.jsx`, scope separator `.`)
- Python (`.py`, `.pyi`, scope separator `.`)
- Go (`.go`, scope separator `.`)

Files with unsupported extensions, or supported files with zero extracted declarations, fall back to a whole-file symbol `<path>#<file>` (`SymbolKind::File`) hashed over trimmed non-empty lines joined by `\n`. Files larger than 2 MiB (`2 * 1024 * 1024` bytes) or containing a NUL byte (`0x00`) in the first 1,024 bytes skip Tree-sitter and receive `<path>#<binary>` hashed over raw file bytes.

## How big do .git/cortex/state.db and refs/cortex/ledger get?

Tier 1 (`.git/cortex/state.db`, SQLite WAL mode) stays local to the clone and is ignored by `.gitignore`. Command runs in `execs` keep at most the trailing 4,096 UTF-8 bytes (`output_tail`) alongside error signatures, candidates, checkpoints metadata, `file_reads`, `call_graph_edges`, `hypotheses`, and indexed records.

Tier 2 (`refs/cortex/ledger`) stores only promoted JSON records for `TOMBSTONE`, `INVARIANT`, and `EVENT` items under `v1/<kind>/<shard>/<id>.json`, distributed across 1,024 two-character shards (`00` through `zz` from the trailing ULID characters). Command logs (`execs`), `candidates`, working-tree snapshots (`refs/cortex/checkpoints/*`), and `EpistemicHandoff` envelopes are never committed to `refs/cortex/ledger`.

## How do linked git worktree checkouts share state?

`CortexRepo::resolve_commondir` reads `<gitdir>/commondir` when present, resolves the primary `.git` common directory, and opens `<commondir>/cortex/state.db`. Linked worktrees therefore share the same SQLite state database and Git reference namespace (`refs/cortex/ledger`, `refs/cortex/checkpoints/*`) as the main working tree.

One `v0.1.0` detail to keep in mind: `CortexSnapshot::create_snapshot` creates its temporary index file (`tmp_index_<id>`) under `repo.path().join("cortex")` (the worktree-specific Git admin directory) and deletes it once the snapshot tree is written.

## Why does cortex_handoff_pack(mode="IMPORT") return an error without an envelope?

`git cortex init` writes a prompt block into `AGENTS.md` and `CLAUDE.md` telling agents to call `cortex_handoff_pack(mode="IMPORT")` at session start. However, `crates/cortex-mcp/src/server.rs` requires an `"envelope"` JSON argument whenever `mode` is `"IMPORT"` and returns `Missing 'envelope' payload for IMPORT mode` if none is provided.

To import working memory, pass the JSON payload returned by an earlier `cortex_handoff_pack(mode="EXPORT")`, `git cortex handoff export`, or parent orchestrator handoff under the `envelope` parameter. When starting a fresh session with no prior envelope, call `cortex_inspect_symbol` on target files and `git cortex status --json` to read existing tombstones, invariants, and branch context. See [Agent guide](AGENT-GUIDE.md) and [MCP and integration](MCP-AND-INTEGRATION.md).

## How does the pre-commit hook run invariant tests, and how do I inspect or disable it?

`git cortex verify-invariants` and the MCP `cortex_verify_invariants` tool check Tree-sitter AST hashes statically and never execute `verification_test_cmd`. By contrast, the `pre-commit` hook installed by `git cortex init` runs `git-cortex hook pre-commit`, which calls `verify_and_reaffirm_invariants(repo, db, DiffScope::Staged, true)`.

When a staged commit causes a `BLOCKING` invariant to drift and that contract defines a non-empty `verification_test_cmd`, the hook runs the command via `sh -c` (Unix) or `cmd /C` (Windows) inside `repo.workdir` with a 30-second timeout. Exit code `0` triggers automatic reaffirmation against the updated AST hash; any other exit code blocks the commit.

Before committing in an unfamiliar repository:

1. Run `git config core.hooksPath` to check whether a custom hooks directory is active; otherwise inspect `.git/hooks/pre-commit`.
2. Bypass hooks for a single commit with `git commit --no-verify`.
3. Remove the `# --- git-cortex:begin ---` through `# --- git-cortex:end ---` block from `.git/hooks/{pre-commit,post-commit,post-checkout,post-merge}` to disable all `git-cortex` hooks permanently.

See [Security policy](../SECURITY.md) for command execution boundaries and untrusted repository guidance.

## Why can reaffirming an invariant on two branches trigger a tamper alert in v0.1.0?

`reaffirm_invariant` (`crates/cortex-engine/src/lifecycle.rs`) updates the existing `INVARIANT` JSON record in SQLite (`promoted = false`, updated `commit_sha` and `ast_hash`) and emits a `REAFFIRM` lifecycle event. Promoting that record writes a modified JSON blob to the same `v1/invariants/<shard>/<id>.json` ledger path.

When `git cortex sync` runs `CortexLedger::merge_commits`, it performs a set-union tree merge and aborts with `Tamper alert: conflicting content detected for record path '...'` if local and remote ledger commits hold different blob OIDs at the same path. If two branches independently reaffirm the same `inv_...` ID to different hashes and push both to `refs/cortex/ledger`, the merge rejects the conflict.

In `v0.1.0`, either reaffirm contracts on a single integration branch or revoke the old invariant (`git cortex invariant revoke <id>`) and create a new invariant contract (`git cortex invariant add ...`) with a fresh ULID when diverging on separate branches. See [Concepts and storage](CONCEPTS-AND-STORAGE.md).

## How do I rebuild the Tier 1 SQLite cache?

Run `git cortex reindex` inside the repository:

```bash
git cortex reindex
```

`reindex` reads every record in `refs/cortex/ledger` and inserts or updates rows in `.git/cortex/state.db` in 500-record `IMMEDIATE` transactions while preserving `branch_context` and `created_at`. Run `reindex` after deleting `.git/cortex/state.db`, after manual `git fetch` of `refs/cortex/ledger`, or after `post-merge` recovery. Use `git cortex sync` when you also want to fetch, merge, reindex, and push `refs/cortex/ledger` with a remote.

## What does git-cortex vs. cortex release packaging currently provide?

Source builds (`crates/cortex-cli/Cargo.toml`) produce two binaries: `git-cortex` (which Git invokes via `git cortex <subcommand>`) and `cortex`. The `npm` package (`npm/package.json`) exposes both `git-cortex` and `cortex` CLI bins through [`npm/bin/git-cortex.js`](../npm/bin/git-cortex.js), which resolves `GIT_CORTEX_BIN`, `npm/binaries/`, `~/.git-cortex/bin`, `target/release`, `PATH`, or a SHA-256 verified GitHub release download (`SHA256SUMS.txt`).

Three `v0.1.0` packaging constraints apply:

- [`.github/workflows/release.yml`](../.github/workflows/release.yml) builds `--bin git-cortex` only across five targets (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`). Prebuilt release archives do not ship the standalone `cortex` binary or a `win32` ARM64 build.
- `npm/bin/git-cortex.js` maps all `win32` architectures to `x86_64-pc-windows-msvc`.
- [`packaging/homebrew/git-cortex.rb`](../packaging/homebrew/git-cortex.rb) compiles `crates/cortex-cli` from source via `cargo install`, and its `sha256` field in `v0.1.0` still holds the placeholder `REPLACE_WITH_RELEASE_TARBALL_SHA256`.

Build from source with `cargo install --path crates/cortex-cli` when you need both `git-cortex` and `cortex` installed natively or run on Windows ARM64.
