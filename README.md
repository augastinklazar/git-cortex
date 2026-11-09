# git-cortex: The Epistemic Context Protocol (ECP v0.1)

> Git-Native Working Memory, Negative-Knowledge Tombstones, and Causal Invariant Locks for Multi-Agent Software Engineering.

[![CI](https://github.com/augastinklazar/git-cortex/actions/workflows/ci.yml/badge.svg)](https://github.com/augastinklazar/git-cortex/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

---

## 1. Problem Statement

Autonomous coding agents routinely fail during complex tasks due to three missing primitives:
1. **The Context Compaction Death Spiral**: When context window compaction prunes conversation history, negative knowledge ("why refactoring symbol X broke invariant Y") is lost first. Minutes later, the agent re-attempts the exact same failing approach.
2. **The Cross-Harness & Sub-Agent Handoff Tax**: Switching between coding harnesses (Claude Code, Gemini CLI, Codex CLI, OpenCode) burns up to 70% of initial tokens re-reading files and re-establishing call graphs.
3. **Silent Semantic Merge Conflicts**: Git tracks textual line diffs rather than causal assumptions. Merges can succeed cleanly while silently violating behavioral preconditions.

## 2. Core Architecture

`git-cortex` provides an epistemic memory layer using a two-tier storage engine and normalized AST hashing:
- **Tier 1 (Ephemeral Local WAL)**: `.git/cortex/state.db` SQLite database tracking live edits, candidate failures, and hypotheses.
- **Tier 2 (Distributed Ledger)**: `refs/cortex/ledger` append-only commit tree storing immutable RFC 8785 canonical JSON records that merge without conflicts.
- **AST Symbol Addressing (`ast_hash`)**: Strips non-semantic comments and whitespace, serializing AST subtrees into deterministic `ast:sha256:...` digests via Tree-sitter.

## 3. Key Capabilities

- **Negative-Knowledge Tombstones**: Fast indexed checks before an agent touches a symbol.
- **Epistemic Handoff Envelopes**: Deterministic ≤4 KB envelopes enabling instant context transfer between harnesses or Google A2A tasks.
- **Causal Invariant Locks**: Pre-commit verification preventing upstream dependency drift from silently breaking downstream expectations.
- **Passive Capture Engine**: Automatic *Edit → Fail → Revert* detection via `git cortex exec -- <cmd>` without requiring voluntary agent logging.
- **MCP Server**: Zero-latency stdio Model Context Protocol server exposing `cortex_inspect_symbol`, `cortex_record_tombstone`, `cortex_checkpoint_state`, `cortex_handoff_pack`, and `cortex_verify_invariants`.

## 4. Quickstart

### Installation

```bash
# Via Cargo (installs both 'git-cortex' and 'cortex' binaries)
cargo install --path crates/cortex-cli

# Or via Homebrew
brew tap augastinklazar/tap
brew install git-cortex

# Or run instantly via npx
npx git-cortex init
```

Both `git cortex <cmd>` (as a native Git subcommand) and `cortex <cmd>` (standalone binary) are supported interchangeably.

### Initialize in a Repository

```bash
git cortex init
# or: cortex init
```

This sets up:
- `.git/cortex/` local SQLite state cache
- Chained Git hooks (`pre-commit`, `post-commit`, `post-checkout`, `post-merge`)
- Normative instructions in `AGENTS.md` and `CLAUDE.md`
- Harness MCP server configuration merged into `.mcp.json`

### CLI Commands Reference

| Command | Description |
|---|---|
| `git cortex init` | Initialize local SQLite DB, git hooks, `AGENTS.md`, and `.mcp.json` |
| `git cortex status` | Show current branch, dirty hash, and epistemic count overview |
| `git cortex inspect <file>` | Inspect a file or symbol for active tombstones and invariant locks |
| `git cortex tombstone` | List, show, or add negative-knowledge tombstones |
| `git cortex invariant` | List, check, or record causal invariant contracts |
| `git cortex checkpoint` | Save working tree snapshot and active hypotheses |
| `git cortex rewind <id>` | Safely revert to checkpoint and synthesize failure tombstones |
| `git cortex handoff` | Export/import ≤4 KB sub-agent handoff envelopes |
| `git cortex verify-invariants` | Verify invariants against staged, working tree, or branch diffs |
| `git cortex promote` | Promote local records to the distributed `refs/cortex/ledger` |
| `git cortex sync` | Push and fetch distributed ledger records with origin remote |
| `git cortex doctor` | Run configuration and git environment diagnostics |
| `git cortex mcp-serve` | Run stdio Model Context Protocol (MCP) server for agent harnesses |

## 5. Specification

See [docs/spec/ECP-v0.1.md](docs/spec/ECP-v0.1.md) for the normative protocol specification.
Original architecture blueprint: [docs/spec/git-cortex Architecture & MVP Specification.pdf](docs/spec/git-cortex%20Architecture%20&%20MVP%20Specification.pdf).

## 6. License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
