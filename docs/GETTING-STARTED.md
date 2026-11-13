# Getting started with git-cortex

## Prerequisites

Install these baseline tools before setting up `git-cortex` in a repository:

| Requirement | Minimum version | Needed for |
|---|---|---|
| Git | 2.30+ | Repository discovery, linked worktrees, and `git cortex` subcommand dispatch |
| Rust toolchain and C compiler | Stable Rust (newer than 1.80 for `Cargo.lock`) | Building `crates/cortex-cli` from source (compiles bundled SQLite, `libgit2`, and Tree-sitter) |
| Node.js | 16.0.0+ | Running the `npx git-cortex` wrapper without building from source |

`Cargo.toml` sets `rust-version = "1.80"`, while `Cargo.lock` locks transitive dependencies including `clap 4.6.7` and `tempfile 3.27.0` that require a newer Rust toolchain when built with the lockfile. Run `rustup update stable` if Cargo rejects a dependency version. Source builds on Linux and macOS need `cc` (GCC or Clang); Windows builds need MSVC Build Tools or an LLVM-MinGW toolchain.

## Installing the binaries

`crates/cortex-cli` compiles two identical executables, `git-cortex` and `cortex`, which both run `cortex_cli::run()` and strip a leading `cortex` argument when invoked via `git cortex <command>`.

### Option 1: Build with Cargo

Install both `git-cortex` and `cortex` directly into `~/.cargo/bin` (or `%USERPROFILE%\.cargo\bin` on Windows):

```bash
# From a cloned repository
cargo install --path crates/cortex-cli

# Or directly from GitHub
cargo install --git https://github.com/augastinklazar/git-cortex cortex-cli
```

Ensure Cargo's `bin` directory is on your `PATH`. Git automatically routes `git cortex <subcommand>` to `git-cortex`.

### Option 2: Run via npx

```bash
npx git-cortex doctor
```

[`npm/bin/git-cortex.js`](../npm/bin/git-cortex.js) resolves the native executable in six steps and stops at the first match:

1. `GIT_CORTEX_BIN` environment variable, if set to an existing file path.
2. Bundled `npm/binaries/<target>/<binaryName>` inside the npm package.
3. Cached user binary at `~/.git-cortex/bin/<binaryName>`.
4. Workspace release build at `target/release/<binaryName>`.
5. First matching binary found on `PATH` (skipping the wrapper itself via `fs.realpathSync`).
6. Automated HTTPS download from GitHub Releases (`SHA256SUMS.txt` verified) into `~/.git-cortex/bin/`.

You can set `GIT_CORTEX_BIN` to pin a locally built debug or release binary when testing wrapper invocations:

```bash
export GIT_CORTEX_BIN="$PWD/target/release/git-cortex"
```

### Option 3: Prebuilt GitHub release archives

[`.github/workflows/release.yml`](../.github/workflows/release.yml) publishes release archives and `SHA256SUMS.txt` on tagged releases (`v*.*.*`) for five target triples:

| Platform | Target triple | Archive |
|---|---|---|
| Linux x64 | `x86_64-unknown-linux-gnu` | `git-cortex-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `git-cortex-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x64 | `x86_64-apple-darwin` | `git-cortex-x86_64-apple-darwin.tar.gz` |
| macOS ARM64 | `aarch64-apple-darwin` | `git-cortex-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `git-cortex-x86_64-pc-windows-msvc.zip` |

Two packaging caveats apply to `v0.1.0`:

- `release.yml` compiles `--bin git-cortex` only, so release archives omit the separate `cortex` binary. Invoke `git cortex <command>`, or copy `git-cortex` to `cortex` in your `PATH` if you want the standalone alias.
- Windows ARM64 (`aarch64-pc-windows-msvc`) is not built in `release.yml`, and `npm/bin/git-cortex.js` maps all `win32` platforms to `x86_64-pc-windows-msvc`. On Windows ARM64 machines, rely on x64 emulation or compile natively from source with Cargo.

### Option 4: Homebrew formula

[`packaging/homebrew/git-cortex.rb`](../packaging/homebrew/git-cortex.rb) defines a source formula (`depends_on "rust" => :build`, `depends_on "git"`) that builds `crates/cortex-cli` and installs both `git-cortex` and `cortex`. In `v0.1.0`, its `sha256` field still holds the placeholder `REPLACE_WITH_RELEASE_TARBALL_SHA256`. Installing that versioned tarball fails checksum verification until the release tarball SHA-256 is populated; install from Cargo or build from `HEAD` until a release tap updates the digest.

## Windows and cloud-sync drive build setup

Developing inside Google Drive, OneDrive, or Dropbox often causes slow builds or file-lock errors because real-time sync agents lock object files in `target/`. In addition, LLVM-MinGW toolchains (`x86_64-pc-windows-gnullvm`) pass `--target=x86_64-pc-windows-gnullvm` to C build scripts in `tree-sitter`, `libgit2-sys`, and `libsqlite3-sys`, which some `clang.exe` distributions reject.

Copy [`.cargo/config.toml.example`](../.cargo/config.toml.example) to `.cargo/config.toml` (gitignored in `.gitignore`) and uncomment the local overrides you need:

```toml
[build]
# Redirect target directory outside sync drives (e.g. Google Drive, OneDrive)
target-dir = "C:/Users/<username>/AppData/Local/cargo-target/git-cortex"

[env]
# Rewrite gnullvm target flag for clang.exe on LLVM-MinGW toolchains
CC_x86_64_pc_windows_gnullvm = { value = "scripts/cortex-clang.cmd", relative = true }
```

[`scripts/cortex-clang.cmd`](../scripts/cortex-clang.cmd) replaces `--target=x86_64-pc-windows-gnullvm` with `--target=x86_64-w64-windows-gnu` and forwards the arguments to `clang.exe`. Keep `.cargo/config.toml` untracked so machine-specific paths do not affect CI or other developers.

## First session walkthrough

Every command below assumes you are inside a non-bare Git repository with at least one commit (`HEAD` must exist for branch, diff, and snapshot commands).

### 1. Initialize the repository

```bash
git cortex init
```

`init` performs five setup actions:

1. Opens and migrates `.git/cortex/state.db` (or `<commondir>/cortex/state.db` in linked worktrees). Migration sets `PRAGMA journal_mode = WAL`, `PRAGMA busy_timeout = 5000`, `PRAGMA foreign_keys = ON`, and `PRAGMA user_version = 1`. It creates nine tables (`records`, `record_symbols`, `symbol_cache`, `execs`, `candidates`, `checkpoints`, `file_reads`, `call_graph_edges`, `hypotheses`) and six indexes.
2. Configures `remote.origin.fetch` in `.git/config` with `+refs/cortex/ledger:refs/remotes/origin/cortex/ledger`. If the repository previously had `+refs/cortex/ledger:refs/cortex/ledger`, `init` replaces it with the remote-tracking mapping and ensures `+refs/heads/*:refs/remotes/origin/*` remains present.
3. Installs four chained Git hooks (`pre-commit`, `post-commit`, `post-checkout`, `post-merge`) inside `core.hooksPath` if configured, or `.git/hooks/` otherwise. Each hook appends a block delimited by `# --- git-cortex:begin ---` and `# --- git-cortex:end ---` that runs `git-cortex hook <name> || exit $?` (falling back to `cortex hook <name> || exit $?`). Existing hook commands stay intact, and Unix hooks get mode `0o755`.
4. Appends the `<!-- git-cortex:begin -->` instruction block to `AGENTS.md` and `CLAUDE.md` when absent.
5. Creates `.mcp.json` or merges `mcpServers["git-cortex"]` (`{"command": "git", "args": ["cortex", "mcp-serve"]}`) into an existing `.mcp.json`.

`--hooks`, `--agents-md`, and `--mcp` on `git cortex init` are `bool` flags with `default_value_t = true`, and the CLI parser currently defines no `--no-hooks`, `--no-agents-md`, or `--no-mcp` opt-out flags. If you need to disable hooks or agent config files after initializing, edit or remove the `git-cortex` blocks in `.git/hooks/`, `AGENTS.md`, `CLAUDE.md`, or `.mcp.json`.

### 2. Run diagnostics and check status

```bash
git cortex doctor
git cortex status
```

`doctor` runs before opening `state.db`, so it checks an uninitialized repository without creating `.git/cortex/state.db`. It prints the repository root, Git common dir, current branch and short `HEAD`, `state.db` record counts, `refs/cortex/ledger` head commit, installed hooks, `AGENTS.md`/`CLAUDE.md` status, and `.mcp.json` status.

`git cortex status` (or `git cortex status --json`) opens `state.db` and reports the branch, `HEAD` commit, working-tree dirty hash (`wt:clean` or `wt:sha256:<16hex>`), and counts for tombstones, invariants, call-graph edges, and hypotheses.

### 3. List AST symbols and inspect constraints

Check how `git-cortex` names and hashes symbols in a file before creating records:

```bash
git cortex symbols src/lib.rs
git cortex inspect src/lib.rs
```

`symbols` extracts Tree-sitter symbols for `.rs`, `.ts`, `.mts`, `.cts`, `.tsx`, `.js`, `.mjs`, `.cjs`, `.jsx`, `.py`, `.pyi`, and `.go`. Unsupported text files emit a whole-file `<path>#<file>` entry hashed across trimmed non-empty lines, while files over 2 MiB or containing a NUL byte in the first 1,024 bytes emit `<path>#<binary>`. Note that `symbols` currently opens `state.db` on startup, so run it inside an initialized or writable Git directory.

`inspect <filepath> [symbol_name]` evaluates recorded tombstones and invariants against the current file AST. Active constraints are returned inside provenance-tagged HTML comments (`<!-- CORTEX_TOMBSTONE_BEGIN ... -->` and `<!-- CORTEX_INVARIANT_BEGIN ... -->`), while changed or deleted targets appear as `STALE` and revoked records appear as `REVOKED`. Add `--json` for structured output.

### 4. Capture failures passively with exec

Wrap build or test commands with `git cortex exec`:

```bash
git cortex exec -- cargo test
git cortex exec -t 60 -- npm test
```

Pass `-t` / `--timeout` in seconds (`0` disables the timeout). When a command exits non-zero, `exec` records the normalized error signature and up to 4,096 UTF-8 tail bytes in `execs`, and stores a `FAILED` candidate if dirty symbols exist and the same `(cmd, signature)` did not already fail on a clean tree (`wt:clean`). After you revert any modified symbol back to its `HEAD` AST hash, the next `exec`, `post-checkout` hook, `post-merge` hook, or MCP tool call runs `reconcile_candidates` and promotes the reverted symbols into an `AUTO_REVERT_DETECTED` tombstone.

You can also add tombstones and invariant contracts directly:

```bash
git cortex tombstone add \
  --symbols "src/auth.rs#verify_jwt" \
  --approach "Inline blocking mutex around token refresh" \
  --reason "Deadlocks async runtime workers under concurrent refresh" \
  --cmd "cargo test auth::" \
  --sig "test auth::refresh_concurrent ... FAILED"

git cortex invariant add \
  --guarded "src/session.rs#Session::rotate" \
  --assertion "Must validate token signature via TokenMgr::verify before rotating" \
  --upstream "src/auth.rs#TokenMgr::verify" \
  --test "cargo test session::" \
  --severity BLOCKING
```

Check invariants manually without running shell commands via `git cortex verify-invariants` (checks `--working-tree` by default, or pass `--staged` or `--branch`). Unlike the manual command, the `pre-commit` hook runs `verify_and_reaffirm_invariants` with auto-reaffirmation enabled. Whenever a staged change drifts on a `BLOCKING` invariant, `pre-commit` executes `verification_test_cmd` via `sh -c` or `cmd /C` with a 30-second timeout. Exit `0` reaffirms the contract automatically; any other exit code rejects the commit.

### 5. Save checkpoints and rewind safely

```bash
git cortex checkpoint -m "Working baseline before token refactor"
```

`checkpoint` captures the working tree (tracked and untracked non-ignored files) under `refs/cortex/checkpoints/chk_<ULID>` using an isolated temporary Git index so your live `.git/index` stays untouched. It also records summary metadata in the `checkpoints` SQLite table.

If an exploration breaks down, roll back using the checkpoint ID printed by `checkpoint`:

```bash
git cortex rewind chk_01JBYN... -r "Deadlock on concurrent refresh"
```

`rewind` backs up the current dirty state under `refs/cortex/backups/chk_<ULID>`, stashes untracked files under `refs/cortex/untracked/<checkpoint_id>`, force-restores the working tree and index to the checkpoint, and records a `REWIND_SYNTHESIZED` tombstone on modified symbols. Two v0.1.0 caveats apply: `rewind` leaves existing SQLite `call_graph_edges` and `hypotheses` untouched, and its diff scan skips files or symbols that were deleted relative to the checkpoint.

### 6. Export and import handoff envelopes

```bash
# Standard EpistemicHandoff JSON (<= 4,096 bytes)
git cortex handoff export --out handoff.json

# Google A2A JSON-RPC tasks/send wrapper
git cortex handoff export --a2a --out a2a-handoff.json

# Load into another session or branch
git cortex handoff import --file handoff.json
```

`handoff export` packs branch state, salient active/stale tombstones, verified call-graph edges, active hypotheses, and unchanged file markers inside a 7-tier 4,096-byte budget. `handoff import` also reads from stdin when `--file` is omitted. `EpistemicHandoff` payloads live only for transfer (`import` updates `call_graph_edges`, `hypotheses`, and `file_reads` in `state.db`) and are never written to `records` or `refs/cortex/ledger`.

If you integrate via MCP, pass the exported JSON object as `envelope` when calling `cortex_handoff_pack(mode="IMPORT")`. Calling `IMPORT` without an `envelope` returns an error (`Missing 'envelope' payload for IMPORT mode`); see [AGENT-GUIDE.md](AGENT-GUIDE.md) for the exact startup sequence.

### 7. Promote, sync, and reindex the distributed ledger

Local tombstones, invariants, and lifecycle events start in `state.db` with `promoted = 0`. Promote them to the orphan commit tree `refs/cortex/ledger` (sharded under `v1/<kind>/<shard>/<id>.json`):

```bash
git cortex promote
git cortex sync
```

`post-commit` hooks run `promote` automatically after every commit. `sync` fetches `refs/cortex/ledger` from `origin` (or a specified remote) into `refs/remotes/<remote>/cortex/ledger`, performs a set-union tree merge, reindexes into SQLite, and pushes `refs/cortex/ledger` with up to five backoff retries (pass `--rebase` to force retry branches on push rejection). To rebuild your local SQLite cache from `refs/cortex/ledger` at any time, run:

```bash
git cortex reindex
```

## Linked Git worktrees

`git-cortex` discovers repositories through `CortexRepo::resolve_commondir`, reading `<git_dir>/commondir` when inside a linked `git worktree` and canonicalizing the primary repository `.git` path. Because `state.db` and `refs/cortex/ledger` live under the common directory, all linked worktrees share one epistemic database and ledger ref while tracking per-branch records, hypotheses, call-graph edges, and file reads by branch name. In v0.1.0, `CortexSnapshot::create_snapshot` writes its transient `tmp_index_<id>` file inside the per-worktree gitdir (`repo.path().join("cortex")`) and deletes it immediately after writing the snapshot tree.

## Troubleshooting

### git prints `'cortex' is not a git command`

`git-cortex` is not in your `PATH`. Confirm where `cargo install` placed `git-cortex` (`~/.cargo/bin` or `%USERPROFILE%\.cargo\bin`) and add that directory to `PATH`, or run via `npx git-cortex <command>`.

### `npx git-cortex` fails to download or find a binary

Check your network access to GitHub Releases or set `GIT_CORTEX_BIN` to a local `git-cortex` executable. On Windows ARM64, `npm/bin/git-cortex.js` attempts to fetch `x86_64-pc-windows-msvc`; if x64 binaries cannot run on your machine, build natively with `cargo install --path crates/cortex-cli`.

### Slow or locked Cargo builds on cloud-synced drives or Windows LLVM-MinGW

Move Cargo's build directory outside Google Drive, OneDrive, or Dropbox using `target-dir` in `.cargo/config.toml` (copied from [`.cargo/config.toml.example`](../.cargo/config.toml.example)). If `clang.exe` rejects `--target=x86_64-pc-windows-gnullvm`, enable `CC_x86_64_pc_windows_gnullvm = { value = "scripts/cortex-clang.cmd", relative = true }`.

### Cargo fails compiling `clap` or `tempfile` on Rust 1.80

Although `Cargo.toml` sets `rust-version = "1.80"`, the workspace `Cargo.lock` pins `clap 4.6.7` and `tempfile 3.27.0` which expect a newer Rust release. Update with `rustup update stable` and rebuild.

### `cortex_handoff_pack` returns `Missing 'envelope' payload for IMPORT mode`

`IMPORT` mode requires an `"envelope"` JSON object produced by `cortex_handoff_pack(mode="EXPORT")` or `git cortex handoff export`. When starting a fresh session without a prior envelope, inspect files directly with `cortex_inspect_symbol` and `git cortex status --json` instead.

### `pre-commit` blocks a commit or hits the 30-second timeout

Run `git cortex verify-invariants --staged` to inspect which guarded symbol or upstream dependency drifted and which `verification_test_cmd` is attached. After an intentional change where tests pass, run `git cortex invariant reaffirm <invariant_id>`, or revoke obsolete contracts with `git cortex invariant revoke <invariant_id>`. See [SECURITY.md](../SECURITY.md) before running `git commit` in untrusted repositories with pre-existing invariant contracts.

### `Tamper alert: conflicting content detected` during `git cortex sync`

When two branches independently run `reaffirm` on the same `inv_...` contract and promote it, `reaffirm_invariant` mutates the existing invariant JSON in place, producing two different blob OIDs at the same `v1/invariants/<shard>/<id>.json` ledger path. Set-union merge detects the collision and aborts to protect ledger integrity. Revoke the conflicted invariant or create a new invariant contract with a fresh `inv_...` ID; see [FAQ.md](FAQ.md) for details.

## Next steps

Read [AGENT-GUIDE.md](AGENT-GUIDE.md) to configure coding agents and MCP workflows, [CLI-REFERENCE.md](CLI-REFERENCE.md) for complete command flags, or [CONCEPTS-AND-STORAGE.md](CONCEPTS-AND-STORAGE.md) for storage and AST hashing internals.
