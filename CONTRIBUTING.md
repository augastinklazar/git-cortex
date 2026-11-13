# Contributing to git-cortex

## Prerequisites

Building and testing `git-cortex` requires three tools, plus Node.js when testing the npm wrapper:

| Tool | Requirement | Notes |
|---|---|---|
| Rust toolchain | Stable via `rustup` (`1.88+` with `Cargo.lock`) | `Cargo.toml` sets `rust-version = "1.80"`, while `Cargo.lock` pins transitive crates requiring newer toolchains (`clap 4.6.7` requires `1.85`, `wasip2 1.0.4+wasi-0.2.12` requires `1.87.0`, and `icu_* 2.4.0` requires `1.88`). [CI](.github/workflows/ci.yml) runs `dtolnay/rust-toolchain@stable` with `rustfmt` and `clippy`. |
| Git | `2.30` or newer | Used for refspec configuration, linked worktrees, and CLI hook tests. |
| C compiler | Clang, GCC, or MSVC | Compiles Tree-sitter C grammars, bundled `rusqlite`, and `libgit2-sys`. On Windows, `crates/cortex-storage/build.rs` links `advapi32`. |
| Node.js | `18+` recommended (`>=16.0.0` in `npm/package.json`) | Required only when testing or modifying the wrapper under `npm/`. |

## Windows and cloud-sync drive builds

Developing inside a cloud-synced directory such as Google Drive or OneDrive can trigger file-locking errors or slow incremental Cargo builds inside `target/`. Copy [`.cargo/config.toml.example`](.cargo/config.toml.example) to `.cargo/config.toml` (which is ignored by [`.gitignore`](.gitignore)) and uncomment `target-dir` to place build artifacts on a local drive:

```toml
[build]
target-dir = "C:/Users/<username>/AppData/Local/cargo-target/git-cortex"
```

If you build on Windows with an LLVM-MinGW toolchain (`x86_64-pc-windows-gnullvm`), uncomment the compiler override in `.cargo/config.toml`:

```toml
[env]
CC_x86_64_pc_windows_gnullvm = { value = "scripts/cortex-clang.cmd", relative = true }
```

[`scripts/cortex-clang.cmd`](scripts/cortex-clang.cmd) rewrites `--target=x86_64-pc-windows-gnullvm` to `--target=x86_64-w64-windows-gnu` before invoking `clang.exe`. Keep `.cargo/config.toml` untracked; never force-add local toolchain overrides to a commit.

[`.gitattributes`](.gitattributes) sets `* text=auto eol=lf`. Configure Git on Windows so tracked text files keep LF line endings.

## Workspace layout

`git-cortex` is a seven-crate Rust workspace (`edition = "2021"`):

| Crate | Path | Responsibilities |
|---|---|---|
| `cortex-core` | `crates/cortex-core` | Data models (`TombstoneRecord`, `EpistemicHandoff`, `InvariantContract`, `LifecycleEvent`), prefixed monotonic ULIDs, RFC 8785 canonical JSON, secret redaction, prompt injection fencing |
| `cortex-ast` | `crates/cortex-ast` | Tree-sitter parsers (Rust, TypeScript, TSX, JavaScript, Python, Go), line-based fallback, symbol URI normalization, normalized AST hashing |
| `crates/cortex-storage` | `crates/cortex-storage` | SQLite WAL state database (`.git/cortex/state.db`), `refs/cortex/ledger` tree operations, isolated snapshot index commits, linked worktree `commondir` discovery |
| `crates/cortex-engine` | `crates/cortex-engine` | Dynamic invalidation (`inspect_symbol`), tombstone and invariant lifecycle, checkpoint and rewind, 4,096-byte handoff packing, ledger promote and sync |
| `crates/cortex-capture` | `crates/cortex-capture` | `cortex exec` child runner with process-group/tree timeout, error signature normalization, `Edit -> Fail -> Revert` candidate reconciliation |
| `cortex-mcp` | `crates/cortex-mcp` | Dual-framed (`Content-Length` and NDJSON) stdio JSON-RPC 2.0 MCP server exposing the 6 `cortex_*` tools |
| `cortex-cli` | `crates/cortex-cli` | Dual binaries (`git-cortex` and `cortex`) implementing the 17 CLI subcommands, `init`, `doctor`, and Git hook handlers |

## Building, testing, and linting

Run the full workspace build, formatter, linter, and test suite locally before opening a pull request. [`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs formatting, Clippy, and `cargo test --workspace --verbose` on `ubuntu-latest`, `macos-latest`, and `windows-latest`.

```bash
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

On commit `b143df9`, `cargo test --workspace` executes 94 tests across 25 suites (18 unit and integration test binaries plus 7 doc-test suites) with zero ignored tests. The integration suites cover:

| Suite path | Coverage |
|---|---|
| `crates/cortex-ast/tests/golden_vectors.rs` | Pinned AST hashes and normalization invariants across supported languages |
| `crates/cortex-ast/tests/disambiguation_test.rs` | Duplicate scope disambiguation (`[2]`, `[3]`) and overload numbering |
| `crates/cortex-ast/tests/bench_hashing.rs` | Release-mode latency assertion (`<= 35 ms` on ~1,100 to 1,200 LOC files) and 10,002-line stress test |
| `crates/cortex-storage/tests/storage_test.rs` | SQLite WAL persistence, sharded ledger commits, set-union merges, tamper detection |
| `crates/cortex-engine/tests/engine_test.rs` | Invalidation, handoff byte budget compaction, invariant drift, rewind |
| `crates/cortex-capture/tests/capture_test.rs` | Command capture, timeout behavior, baseline failure filtering, partial revert promotion |
| `crates/cortex-mcp/tests/mcp_test.rs` | JSON-RPC framing, `tools/list`, `tools/call`, error responses |
| `crates/cortex-cli/tests/cli_test.rs` | CLI subcommand execution, `init`, `doctor`, hook configuration |
| `crates/cortex-cli/tests/e2e_scenarios.rs` | End-to-end repository workflows across CLI and Git |

Run the release AST benchmark directly when changing `cortex-ast` parsing or hashing:

```bash
cargo test -p cortex-ast --test bench_hashing --release -- --nocapture
```

If you modify files under `npm/`, run the Node checksum tests:

```bash
cd npm
npm test
```

## Conventional commits

Format commit messages according to [Conventional Commits](https://www.conventionalcommits.org/):

- `feat(ast): add Ruby grammar extraction`
- `fix(storage): resolve linked worktree temp index path`
- `docs(security): clarify pre-commit hook execution scope`
- `test(engine): cover multi-branch reaffirm conflict`
- `chore(ci): update release target matrix`

## Pull request workflow

1. Create a branch from `main` (for example, `feat/my-change` or `fix/issue-name`).
2. Add or update unit and integration tests for any behavioral change or bug fix.
3. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `npm test` (if `npm/` changed).
4. Verify `git status` contains no untracked runtime artifacts (`.cargo/config.toml`, `.git/cortex/` state, or temporary logs).
5. Open a pull request against `main` summarizing the change, test results, and any schema or behavior impacts.

## Documentation maintenance

Update documentation in the same pull request whenever code, schemas, CLI flags, MCP tools, or workflows change:

| Change area | Documentation to update |
|---|---|
| CLI subcommands, flags, or exit codes | [`docs/CLI-REFERENCE.md`](docs/CLI-REFERENCE.md), [`README.md`](README.md), [`docs/GETTING-STARTED.md`](docs/GETTING-STARTED.md) |
| MCP server framing, tools, or `AGENTS.md` snippets | [`docs/MCP-AND-INTEGRATION.md`](docs/MCP-AND-INTEGRATION.md), [`docs/AGENT-GUIDE.md`](docs/AGENT-GUIDE.md), [`README.md`](README.md) |
| Record schemas, ULIDs, or canonical hashing | [`docs/SCHEMAS.md`](docs/SCHEMAS.md), [`docs/spec/ECP-v0.1.md`](docs/spec/ECP-v0.1.md) |
| SQLite tables, ledger layout, AST rules, or handoff compaction | [`docs/CONCEPTS-AND-STORAGE.md`](docs/CONCEPTS-AND-STORAGE.md), [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) |
| Hooks, `exec`, redaction, fencing, or path checks | [`SECURITY.md`](SECURITY.md), [`docs/FAQ.md`](docs/FAQ.md) |
| Release artifacts, version bumps, or user-visible fixes | [`CHANGELOG.md`](CHANGELOG.md), [`docs/FAQ.md`](docs/FAQ.md) |
