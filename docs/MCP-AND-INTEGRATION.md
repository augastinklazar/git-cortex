# MCP server and harness integration

`git cortex mcp-serve` exposes the repository's epistemic memory over stdio using JSON-RPC 2.0 and the Model Context Protocol (`2024-11-05`). Any MCP host or custom orchestrator can run it inside a Git repository or linked worktree.

## Stdio transport (`git cortex mcp-serve`)

You can start the server with `git cortex mcp-serve`, `git-cortex mcp-serve`, or `cortex mcp-serve`. When invoked as `git cortex ...`, the CLI strips the leading `"cortex"` argument, discovers the Git repository from the current working directory (following `commondir` for linked worktrees), opens `<commondir>/cortex/state.db`, and enters the stdio message loop.

### Dual framing

`read_next_rpc_message` in [server.rs](../crates/cortex-mcp/src/server.rs) detects framing per incoming message on stdin and replies in the matching format on stdout.

When a message begins with `Content-Length: <n>` (matched for both `Content-Length:` and `content-length:`), the server consumes any remaining header lines until a blank line, reads exact `<n>` raw bytes, and replies with `Content-Length: <len>\r\n\r\n<json>`. When a line starts directly with JSON (NDJSON), the server parses the trimmed line and replies with `<json>\n`.

Blank lines between messages are skipped. Clients may intermix both framing modes within the same stdio session.

### Protocol handshake and `clientInfo` mapping

| Method | Request type | Server response | Notes |
| :--- | :--- | :--- | :--- |
| `initialize` | Request (`id` present) | `protocolVersion: "2024-11-05"`, `capabilities: {"tools":{}}`, `serverInfo: {"name":"git-cortex","version":"0.1.0"}` | Reads `params.clientInfo.name` and `version` into `author_harness` |
| `notifications/initialized` | Notification or request | No output | Returns `None` regardless of `id` |
| `ping` | Request | `{}` | Liveness check |
| `tools/list` | Request | `{"tools": [...]}` | Returns all 6 MCP tool definitions |
| `tools/call` | Request | `ToolResult` (`content`, `isError`) | Runs `reconcile_candidates` before executing the tool |

During `initialize`, if `params.clientInfo` is present, the server sets `author_harness` to `"<name>/<version>"` using `"agent"` when `name` is missing and `"1.0"` when `version` is missing. When `clientInfo` is omitted, `author_harness` defaults to `"generic-mcp-client/1.0"`. Every tombstone written by `cortex_record_tombstone` stores this `author_harness` value.

Per JSON-RPC 2.0 section 4.1, any message with `id` omitted or `null` produces zero output on stdout.

### Passive capture side effect on `tools/call`

Before dispatching `tools/call` to any of the 6 tools, the server calls `reconcile_candidates(&self.repo, &mut self.db)`. Any `FAILED` candidate from a prior `git cortex exec` whose modified symbols have reverted to their `HEAD` AST hash is promoted to an `AUTO_REVERT_DETECTED` tombstone immediately, so subsequent `cortex_inspect_symbol` or `cortex_handoff_pack` calls see the new tombstone. Errors from `reconcile_candidates` are ignored so the requested tool still executes.

### Error responses

| Condition | Wire shape | Code or `isError` | Message format |
| :--- | :--- | :--- | :--- |
| Invalid JSON input | JSON-RPC `error` (`"id": null`) | `-32700` | `Parse error: <serde error>` |
| Unknown method with `id` | JSON-RPC `error` | `-32601` | `Method not found: <method>` |
| Unknown tool name in `tools/call` | JSON-RPC `result` | `isError: true` | `Unknown tool: <name>` |
| Tool validation or engine failure | JSON-RPC `result` | `isError: true` | Tool-specific diagnostic text |

When stdin reaches EOF, the server loop exits with code `0`. If reading stdin fails with an I/O error, the server prints `cortex-mcp: stdin error: <err>` to stderr and exits the loop.

## MCP tool reference

All 6 tools return a `ToolResult` with `content: [{"type": "text", "text": "..."}]` and `isError: true | false`.

### 1. `cortex_inspect_symbol`

Retrieve active tombstones, active invariant locks, and stale or revoked comments for a file or symbol before planning edits.

| Parameter | Type | Required | Default | Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `filepath` | `string` | Yes | None | Repository-relative path; normalized before lookup |
| `symbol_name` | `string` | No | `null` | Substring/suffix filter (`ends_with` or `contains` on tombstone URIs; `contains` on invariant guarded/upstream URIs) |

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "cortex_inspect_symbol",
    "arguments": {
      "filepath": "src/service.rs",
      "symbol_name": "Service::run"
    }
  }
}
```

Response (`isError: false`):

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "<!-- CORTEX_TOMBSTONE_BEGIN id=\"tmb_01J...\" provenance=\"LOCAL_WORKING_TREE\" harness=\"claude-code/1.4.2\" -->\n[TOMBSTONE: tmb_01J...]\n- Approach Attempted: Tried multithreading without Arc\n- Why It Failed: Lifetime borrowed data escapes function\n- Error Signature: E0373: closure may outlive the current function\n<!-- CORTEX_TOMBSTONE_END id=\"tmb_01J...\" -->"
      }
    ],
    "isError": false
  }
}
```

When no matching records exist, `text` contains `<!-- CORTEX_INSPECT: No active Tombstones or Invariants found for <filepath> -->`.

Error cases (`isError: true`):
- `Missing required parameter 'filepath'` when `filepath` is absent or non-string.
- `Inspection failed: <err>` when reading or parsing the target file fails.

### 2. `cortex_record_tombstone`

Record a disproved approach as an AST-bound `TombstoneRecord` in `.git/cortex/state.db` (`capture_mode: "MANUAL"`, `exit_code: 1`). Secrets in `attempted_approach`, `failure_reason`, `test_command`, and `error_signature` are scrubbed through `cortex_core::redact::scrub` before storage.

| Parameter | Type | Required | Default | Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `target_symbols` | `string[]` | Yes | None | List of `<rel_path>#<scope>` URIs; bare paths map to `<rel_path>#<file>` |
| `attempted_approach` | `string` | Yes | None | Technical summary of what was tried |
| `failure_reason` | `string` | Yes | None | Root cause of the failure |
| `test_command` | `string` | No | `""` | Command that demonstrated the failure |
| `error_signature` | `string` | No | `""` | Primary assertion or error output line |

Non-string items inside `target_symbols` are skipped by string filtering. If a referenced file exists on disk and the scope is not `<file>`, a missing scope returns `SymbolNotFound`. Missing files record `ast:sha256:0000000000000000`.

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "cortex_record_tombstone",
    "arguments": {
      "target_symbols": ["src/service.rs#Service::run"],
      "attempted_approach": "Tried multithreading without Arc",
      "failure_reason": "Lifetime borrowed data escapes function",
      "test_command": "cargo test",
      "error_signature": "E0373: closure may outlive the current function"
    }
  }
}
```

Response (`isError: false`):

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Recorded Tombstone 'tmb_01J...' bound to 1 symbol(s)."
      }
    ],
    "isError": false
  }
}
```

Error cases (`isError: true`):
- `Missing or invalid 'target_symbols'` when `target_symbols` is missing or not an array.
- `Missing required 'attempted_approach'` or `Missing required 'failure_reason'`.
- `Failed to record tombstone: <err>` on `SymbolNotFound`, Git `HEAD` discovery failure (such as an unborn `HEAD` commit), or SQLite write failure.

### 3. `cortex_checkpoint_state`

Create a Git working-tree snapshot under `refs/cortex/checkpoints/chk_<ULID>`, upsert verified call-graph edges and active hypotheses for the current branch in SQLite, and insert a checkpoint row into `checkpoints`.

| Parameter | Type | Required | Default | Schema and validation |
| :--- | :--- | :--- | :--- | :--- |
| `objective` | `string` | Yes | None | Current goal stored on the checkpoint and commit message |
| `verified_edges` | `object[]` | No | `[]` | Objects requiring `caller`, `callee`, `relationship`, `verified_ast_hash` (all strings) |
| `active_hypotheses` | `object[]` | No | `[]` | Objects requiring `hypothesis_id`, `statement`, `status`, `next_verification_step` (strings) and `confidence` (finite number in `0.0..=1.0`) |

Both `verified_edges` and `active_hypotheses` validate strictly when provided and non-null. Passing `confidence: 1.5`, `-0.1`, or `"NaN"` fails the entire checkpoint call; `git-cortex` rejects the array when any item fails validation.

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "tools/call",
  "params": {
    "name": "cortex_checkpoint_state",
    "arguments": {
      "objective": "Verify concurrency refactor",
      "verified_edges": [
        {
          "caller": "src/main.rs#main",
          "callee": "src/service.rs#Service::run",
          "relationship": "INVOKES",
          "verified_ast_hash": "ast:sha256:1111222233334444"
        }
      ],
      "active_hypotheses": [
        {
          "hypothesis_id": "hyp_1",
          "statement": "Wrapping state in Arc<Mutex<T>> satisfies Send + Sync",
          "confidence": 0.85,
          "status": "TESTING",
          "next_verification_step": "Run cargo test --workspace"
        }
      ]
    }
  }
}
```

Response (`isError: false`):

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Checkpoint 'chk_01J...' saved on branch 'main' (snapshot: refs/cortex/checkpoints/chk_01J...). Epistemic graph: 1 edges, 1 hypotheses."
      }
    ],
    "isError": false
  }
}
```

Error cases (`isError: true`):
- `Missing required 'objective'`
- `Invalid 'verified_edges': <serde error>`
- `Invalid 'active_hypotheses': <serde error>` (including confidence out of `[0.0, 1.0]`)
- `Checkpoint failed: <err>` (for example, on a repo without an initial `HEAD` commit or SQLite failure)

### 4. `cortex_handoff_pack`

Export a compacted `EpistemicHandoff` envelope (`<= 4,096` bytes) or import a provided envelope into SQLite for the active branch.

| Parameter | Type | Required | Default | Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `mode` | `string` | Yes | None | `"EXPORT"` or `"IMPORT"` (case-insensitive) |
| `branch` | `string` | No | Current branch | Overrides source branch during `EXPORT`; ignored during `IMPORT` |
| `envelope` | `object` | On `IMPORT` | None | `EpistemicHandoff` JSON object to write into SQLite on `IMPORT` |

Calling `cortex_handoff_pack` with `{"mode": "IMPORT"}` and no `envelope` fails with an error because `IMPORT` requires an explicit payload.

`EXPORT` request and response:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "tools/call",
  "params": {
    "name": "cortex_handoff_pack",
    "arguments": {
      "mode": "EXPORT"
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\n  \"$schema\": \"https://git-cortex.dev/schemas/v0.1/handoff.json\",\n  \"handoff_id\": \"hnd_01J...\",\n  \"schema_version\": \"0.1.0\",\n  \"head_commit_sha\": \"b143df9...\",\n  \"working_tree_dirty_hash\": \"wt:clean\",\n  \"objective\": \"Active development task\",\n  \"verified_call_graph\": [],\n  \"active_hypotheses\": [],\n  \"active_tombstone_ids\": [\"tmb_01J...\"],\n  \"do_not_re_read_unchanged\": []\n}"
      }
    ],
    "isError": false
  }
}
```

`IMPORT` request and response:

```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "tools/call",
  "params": {
    "name": "cortex_handoff_pack",
    "arguments": {
      "mode": "IMPORT",
      "envelope": {
        "$schema": "https://git-cortex.dev/schemas/v0.1/handoff.json",
        "handoff_id": "hnd_01J...",
        "schema_version": "0.1.0",
        "head_commit_sha": "b143df9...",
        "working_tree_dirty_hash": "wt:clean",
        "objective": "Verify concurrency refactor",
        "verified_call_graph": [],
        "active_hypotheses": [],
        "active_tombstone_ids": [],
        "do_not_re_read_unchanged": []
      }
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Epistemic handoff imported successfully."
      }
    ],
    "isError": false
  }
}
```

Error cases (`isError: true`):
- `Missing required 'mode' (IMPORT or EXPORT)`
- `Invalid mode: expected 'IMPORT' or 'EXPORT'`
- `Missing 'envelope' payload for IMPORT mode`
- `Invalid handoff envelope JSON: <serde error>`
- `Export failed: <err>` or `Import failed: <err>`

### 5. `cortex_verify_invariants`

Run a read-only static AST check of all non-`REVOKE`d `InvariantContract` records (evaluating both `guarded_symbol` and `upstream_dependencies`). Never executes `verification_test_cmd`.

| Parameter | Type | Required | Default | Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `diff_scope` | `string` | No | `"WORKING_TREE"` | One of `"STAGED"`, `"WORKING_TREE"`, or `"BRANCH"` (unknown values fall back to `"WORKING_TREE"`) |

| `diff_scope` value | Files read in `v0.1.0` |
| :--- | :--- |
| `WORKING_TREE` | Reads current files directly from disk |
| `STAGED` | Reads stage-0 Git index blobs, falling back to working-tree files if absent from the index, then `HEAD` |
| `BRANCH` | Reads the `HEAD` tree directly (does not diff against a base branch such as `main`) |

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "tools/call",
  "params": {
    "name": "cortex_verify_invariants",
    "arguments": {
      "diff_scope": "STAGED"
    }
  }
}
```

Passing response (`isError: false`, including when only `WARNING` invariants drifted):

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "All 2 invariant contracts verified successfully against Staged."
      }
    ],
    "isError": false
  }
}
```

Failing response (`isError: true` when one or more `BLOCKING` invariants drifted):

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "INVARIANT VIOLATION: 1 blocking failure(s) found in Staged:\n- Invariant 'inv_01J...': guarded symbol 'src/service.rs#Service::run' broken by upstream 'src/service.rs#Service' drift!\n  Assertion: Must return boolean without panicking\n  Run: cargo test\n"
      }
    ],
    "isError": true
  }
}
```

Error case (`isError: true`): `Invariant verification failed: <err>`.

### 6. `cortex_record_invariant`

Create a Causal Invariant Lock binding a guarded symbol URI to zero or more upstream dependency URIs using their current working-tree AST hashes.

| Parameter | Type | Required | Default | Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `guarded_symbol` | `string` | Yes | None | Symbol URI `<path>#<scope>` to protect |
| `assertion` | `string` | Yes | None | Contract precondition or requirement |
| `verification_test_cmd` | `string` | Yes | None | Shell command stored for the `pre-commit` hook and manual verification |
| `upstream_dependencies` | `string[]` | No | `[]` | Upstream symbol URIs; items lacking `#` are skipped |
| `severity` | `string` | No | `"BLOCKING"` | `"BLOCKING"` or `"WARNING"`; any string other than `"WARNING"` maps to `BLOCKING` |

If a target file or symbol does not exist on disk when recording, `record_invariant` stores `ast:sha256:0000000000000000` (which will report drift until the symbol matches or the invariant is reaffirmed/revoked).

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "tools/call",
  "params": {
    "name": "cortex_record_invariant",
    "arguments": {
      "guarded_symbol": "src/service.rs#Service::run",
      "assertion": "Must return boolean without panicking",
      "upstream_dependencies": ["src/service.rs#Service"],
      "verification_test_cmd": "cargo test",
      "severity": "BLOCKING"
    }
  }
}
```

Response (`isError: false`):

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Created InvariantContract 'inv_01J...' guarding 'src/service.rs#Service::run' against 1 upstream dependencies."
      }
    ],
    "isError": false
  }
}
```

Error cases (`isError: true`):
- `Missing 'guarded_symbol'`, `Missing 'assertion'`, or `Missing 'verification_test_cmd'`
- `Failed to record invariant: Invalid guarded symbol URI format` when `guarded_symbol` cannot parse as `<path>#<scope>`
- `Failed to record invariant: <err>` on Git `HEAD` lookup or SQLite failure

## Harness configuration recipes

MCP clients launch `git cortex mcp-serve` in the repository root so `CortexRepo::discover(None)` locates `.git` (or `<commondir>`). On macOS or Windows GUI editors whose environment does not inherit your shell `PATH`, point `"command"` to the full path of `git-cortex` (or `git`) with `"args": ["mcp-serve"]` (or `["cortex", "mcp-serve"]`).

### `.mcp.json` and `git cortex init` merge behavior

Running `git cortex init` writes or updates [.mcp.json](../.mcp.json) in the repository root:

```json
{
  "mcpServers": {
    "git-cortex": {
      "command": "git",
      "args": [
        "cortex",
        "mcp-serve"
      ]
    }
  }
}
```

When `.mcp.json` already exists, `setup_or_merge_mcp_config` parses it as JSON, preserves existing sibling keys under `mcpServers`, and inserts or updates `mcpServers["git-cortex"]`. If an existing `.mcp.json` contains invalid JSON or a non-object root, `init` resets it to `{}` before writing `mcpServers["git-cortex"]`. Notice that `git cortex init` flags (`--hooks`, `--agents-md`, `--mcp`) all default to `true` and accept no `--no-*` opt-out flags in `v0.1.0`.

### Claude Code

Claude Code reads project-scoped `.mcp.json` and [CLAUDE.md](../CLAUDE.md) automatically when initialized with `git cortex init`. To register it at user scope instead:

```sh
claude mcp add git-cortex -- git cortex mcp-serve
```

### Cursor

Create or merge `.cursor/mcp.json` in the workspace (or `~/.cursor/mcp.json` globally):

```json
{
  "mcpServers": {
    "git-cortex": {
      "command": "git",
      "args": ["cortex", "mcp-serve"]
    }
  }
}
```

Pair `.cursor/mcp.json` with a project rule in `.cursor/rules/git-cortex.mdc` (see [Agent operational guide](./AGENT-GUIDE.md#cursor-rule-cursorrulesgit-cortexmdc)) so Cursor does not emit an empty `IMPORT` call on cold starts.

### Gemini CLI

Add the server block to `.gemini/settings.json` (project) or `~/.gemini/settings.json` (user):

```json
{
  "mcpServers": {
    "git-cortex": {
      "command": "git",
      "args": ["cortex", "mcp-serve"]
    }
  }
}
```

### Codex CLI

Add an entry to `~/.codex/config.toml` (or `.codex/config.toml`):

```toml
[mcp_servers.git-cortex]
command = "git"
args = ["cortex", "mcp-serve"]
```

### Windsurf

Add the entry under `mcpServers` in `~/.codeium/windsurf/mcp_config.json`, using the repository path or workspace directory as `cwd` if your Windsurf launcher starts outside the repo:

```json
{
  "mcpServers": {
    "git-cortex": {
      "command": "git",
      "args": ["cortex", "mcp-serve"],
      "cwd": "/path/to/your/repo"
    }
  }
}
```

### Custom stdio orchestrators

Custom orchestrators can speak newline-delimited JSON-RPC 2.0 directly over stdin/stdout pipes:

```python
import json
import subprocess

proc = subprocess.Popen(
    ["git", "cortex", "mcp-serve"],
    cwd="/path/to/repo",
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
    bufsize=1,
)

def rpc(req_id, method, params=None):
    msg = {"jsonrpc": "2.0", "method": method}
    if req_id is not None:
        msg["id"] = req_id
    if params is not None:
        msg["params"] = params
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    if req_id is None:
        return None
    return json.loads(proc.stdout.readline())

init_resp = rpc(1, "initialize", {"clientInfo": {"name": "my-orchestrator", "version": "0.1"}})
rpc(None, "notifications/initialized", {})
tools = rpc(2, "tools/list", {})
inspect = rpc(3, "tools/call", {
    "name": "cortex_inspect_symbol",
    "arguments": {"filepath": "src/main.rs"},
})
assert not inspect["result"]["isError"], inspect["result"]["content"][0]["text"]
```

Check `resp["result"]["isError"]` after every `tools/call`. `git-cortex` returns tool-level failures (including missing `envelope` on `IMPORT` and `BLOCKING` invariant violations) with `isError: true` inside a JSON-RPC `result`, while transport parse and unknown-method errors use JSON-RPC `error`.

## Git hooks and workflow integration

### Hook installation and `core.hooksPath`

`git cortex init` resolves the hook directory from `core.hooksPath` when configured (resolving relative paths against `repo.workdir`) and otherwise uses `<git_dir>/hooks` (`<commondir>/hooks` for linked worktrees). For each of the 4 hook files (`pre-commit`, `post-commit`, `post-checkout`, `post-merge`), `init` creates `#!/bin/sh` if absent, preserves existing hook commands, sets `0o755` permissions on Unix, and appends an idempotent block if `# --- git-cortex:begin ---` is not already present:

```sh
# --- git-cortex:begin ---
if command -v git-cortex >/dev/null 2>&1; then
    git-cortex hook pre-commit || exit $?
elif command -v cortex >/dev/null 2>&1; then
    cortex hook pre-commit || exit $?
fi
# --- git-cortex:end ---
```

Each hook exits early with `|| exit $?` on failure so pre-existing hooks run beforehand and non-zero exits propagate cleanly.

### Hook behavior matrix

| Hook | Engine call | Side effects and exit behavior |
| :--- | :--- | :--- |
| `pre-commit` | `verify_and_reaffirm_invariants(repo, db, DiffScope::Staged, true)` | Runs `verification_test_cmd` (`sh -c` on Unix, `cmd /C` on Windows, 30s timeout, exit `124` on timeout) when any `BLOCKING` invariant drifts. Exit `0` reaffirms the invariant in SQLite and allows the commit; non-zero prints violations and exits `1`. |
| `post-commit` | `promote_local_records(repo, db)` | Promotes unpromoted `TOMBSTONE`, `INVARIANT`, and `EVENT` records to `refs/cortex/ledger` and marks them `origin = "LEDGER"`, `promoted = true`. Always exits `0`. |
| `post-checkout` | `reconcile_candidates(repo, db)` | Reconciles pending `FAILED` candidates against working-tree symbols after branch switches. Always exits `0`. |
| `post-merge` | `CortexLedger::reindex_into_db` then `reconcile_candidates` | Indexes merged ledger records into SQLite and reconciles `FAILED` candidates. Always exits `0`. |

To disable automatic test execution during `git commit` in an untrusted repository, inspect `git cortex invariant list --json` and remove the `# --- git-cortex:begin ---` through `# --- git-cortex:end ---` block from `pre-commit` (or commit with `--no-verify` after auditing all hooks).

### Remote ledger fetch refspec and sync

`git cortex init` configures `remote.origin.fetch` with:

```ini
+refs/cortex/ledger:refs/remotes/origin/cortex/ledger
```

Keeping ledger fetches under `refs/remotes/origin/cortex/ledger` prevents `git fetch` from overwriting unpushed local ledger commits. If `init` detects the legacy `+refs/cortex/ledger:refs/cortex/ledger` refspec, it replaces that entry with the remote-tracking refspec and ensures `+refs/heads/*:refs/remotes/origin/*` remains present.

Run `git cortex sync` to fetch remote records, perform a deterministic set-union tree merge (aborting with a tamper alert if the same `v1/<kind>/<shard>/<id>.json` path has conflicting blob OIDs), reindex into SQLite, and push back to the remote with exponential backoff (`50 * 2^attempt` ms over up to 5 attempts):

```sh
git cortex sync
git cortex sync upstream --rebase
```

## Related documentation

- [Agent operational guide](./AGENT-GUIDE.md)
- [CLI reference](./CLI-REFERENCE.md)
- [Getting started](./GETTING-STARTED.md)
- [Security boundary and fencing](../SECURITY.md)
