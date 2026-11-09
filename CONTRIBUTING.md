# Contributing to git-cortex

Contributions to `git-cortex` are welcome. This guide details local development setup, testing standards, and contribution policies.

---

## 1. Prerequisites

- **Rust**: Version 1.80.0 or later (MSRV). Recommended: stable Rust toolchain via `rustup`.
- **Git**: Version 2.30 or later.
- **C Compiler**: Clang, GCC, or MSVC (required by Tree-sitter C grammar runtime).
- **Node.js** (Optional): Version 18+ if testing npm packaging.

---

## 2. Workspace Structure

The project is structured as a Cargo workspace:

| Crate | Purpose |
|---|---|
| `crates/cortex-core` | Data models, ULID IDs, RFC 8785 canonical JSON, redactor, injection defense |
| `crates/cortex-ast` | Tree-sitter parsers, symbol hierarchy, normalized AST hashing |
| `crates/cortex-storage` | SQLite WAL state DB, `refs/cortex/ledger` manager, checkpoints |
| `crates/cortex-engine` | Business logic, dynamic invalidation, handoff packing, invariant checks |
| `crates/cortex-capture` | Command executor, error signatures, dirty-tree candidate reconciliation |
| `crates/cortex-mcp` | Fast stdio JSON-RPC 2.0 MCP server for AI agents |
| `crates/cortex-cli` | CLI binary (`cortex` / `git-cortex`) and git subcommands |

---

## 3. Development Workflow

### Build
```bash
cargo build --workspace
```

### Run Tests
```bash
cargo test --workspace
```

### Formatting & Linting
All pull requests must pass formatting and zero-warning Clippy checks:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 4. Conventional Commits

Commits must follow the [Conventional Commits](https://www.conventionalcommits.org/) format:
- `feat(core): add new symbol extraction metric`
- `fix(storage): resolve worktree common dir edge case`
- `docs: update ECP v0.1 specification`
- `test(engine): add tombstone invalidation regression test`

---

## 5. Submitting Pull Requests

1. Fork repository and create a feature branch (`git checkout -b feat/my-feature`).
2. Implement changes with corresponding unit and integration tests.
3. Verify that `cargo test --workspace`, `cargo fmt --all --check`, and `cargo clippy --workspace --all-targets -- -D warnings` pass.
4. Push your branch and open a PR with a description referencing any related issues.
