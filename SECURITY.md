# Security architecture and threat model

`git-cortex` manages repository metadata, local SQLite state, Git references, child test commands, and context returned to language model agents. These security boundaries describe `git-cortex` at commit `b143df9`.

![Four-stage defense and provenance fencing in git-cortex](docs/assets/security-fencing.png)

## Threat matrix

| Threat | Impact | Implemented controls | Known gaps |
|---|---|---|---|
| Arbitrary command execution | Critical | `cortex_verify_invariants`, `cortex_inspect_symbol`, and `git cortex verify-invariants` perform static AST checks only and never execute shell commands. `cortex exec --timeout` terminates timed-out process groups or process trees. | The `pre-commit` Git hook installed by `git cortex init` calls `verify_and_reaffirm_invariants(..., true)` and runs `verification_test_cmd` with a 30s timeout when a `BLOCKING` invariant drifts. `cortex exec` buffers full stdout and stderr in memory before persisting the trailing 4,096 bytes. |
| Prompt injection in shared records | High | `cortex_core::fence::sanitize_text` strips escape sequences, control characters, Bidi overrides, and Unicode tags, breaks HTML comment delimiters and fake fence markers, caps field lengths, and wraps active records in provenance-tagged HTML fences. | Active records depend on the host agent harness treating fenced HTML blocks as data. `STALE` and `REVOKED` status comments in `inspect_symbol` do not pass through `sanitize_text`. |
| Credential leakage | High | `cortex_core::redact::scrub` applies 13 regex rules and a Shannon entropy pass with a camelCase filter to tombstone inputs before persistence. | No standalone rule matches `eyJ...` JWT payloads, and `github_pat_` matches only an 82-character suffix. `execs.output_tail` in `.git/cortex/state.db` and `InvariantContract` fields are not passed through `scrub`. |
| Path traversal in symbol URIs | High | `normalize_rel_path` converts backslashes, strips Windows drive prefixes, and collapses empty, `.`, and `..` segments. `CortexError::PathOutOfBounds` exists for callers that enforce bounds. | `normalize_rel_path` is string normalization only; paths are joined onto `repo.workdir` without a post-normalization canonical containment check against symlinks or leading escapes. |
| Ledger ref tampering and clobbering | Medium | Ledger records live under `refs/cortex/ledger` using RFC 8785 canonical JSON and SHA-256 content hashes. `git cortex init` configures a remote-tracking fetch refspec, and `merge_commits` aborts on conflicting blob OIDs. | Ledger commits are not cryptographically signed by `git-cortex`. Multi-branch `invariant reaffirm` mutates the same invariant path in place and triggers a tamper alert during merge. |

## Command execution boundaries

### Static checks vs. pre-commit execution

`InvariantContract` (`verification_test_cmd`) and `TombstoneRecord` (`verification_evidence.command`, referred to as `reproduction_cmd` in earlier design notes) store shell command strings from the repository or remote ledger. Those strings are treated differently depending on the caller:

- Read-only and static callers never spawn a shell or child command. Calling `cortex_inspect_symbol` over MCP, `git cortex inspect` from the CLI, `cortex_verify_invariants` over MCP, or `git cortex verify-invariants` from the CLI only hashes source trees with Tree-sitter and compares AST digests. Any `verification_test_cmd` shown in output is advisory text for the user or agent.
- The `pre-commit` Git hook installed by `git cortex init` executes commands automatically when a `BLOCKING` invariant drifts. `crates/cortex-cli/src/hook.rs` calls `verify_and_reaffirm_invariants(repo, db, DiffScope::Staged, true)`. If a `BLOCKING` invariant reports a violation and has a non-empty `verification_test_cmd`, `crates/cortex-engine/src/invariant.rs` runs `sh -c <cmd>` on Unix or `cmd /C <cmd>` on Windows inside `repo.workdir` with a 30-second timeout (returning exit code `124` on timeout). If that command exits with `0`, the hook reaffirms the invariant against the new AST hash and allows the commit.

Inspect any repository's hooks before running `git commit` on untrusted code:

1. Check `git config core.hooksPath` and inspect `.git/hooks/pre-commit` (or the configured hooks directory) for the `# --- git-cortex:begin ---` block.
2. Skip hooks on a single commit with `git commit --no-verify`.
3. Remove the `# --- git-cortex:begin ---` through `# --- git-cortex:end ---` block from the hook files to disable hook execution permanently. Because `git cortex init` currently defines `--hooks`, `--agents-md`, and `--mcp` with `default_value_t = true` and no `--no-*` opt-out flags, remove or audit `.git/hooks/*` after running `init` in untrusted repositories.

### Subprocess isolation in `cortex exec`

`git cortex exec` spawns `command[0]` directly with trailing arguments (without invoking an outer shell unless you pass `sh -c` or `cmd /C` yourself) in `repo.workdir`.

- When `-t` / `--timeout` is greater than `0`, `crates/cortex-capture/src/exec.rs` polls for completion every 25 milliseconds. On Unix, the child is placed in its own process group (`process_group(0)`), and timeout sends `SIGKILL` to the negative PID (`-pid`) before calling `child.kill()` and returning exit code `124`. On Windows, timeout runs `taskkill /F /T /PID <pid>` to terminate the child process tree before calling `child.kill()` and returning exit code `124`.
- Stdout and stderr are drained concurrently by two worker threads using unbounded `read_to_end` calls into memory, then forwarded to the terminal. Only the trailing 4,096 bytes of combined output (aligned to a valid UTF-8 boundary by `safe_tail_utf8`) are stored in the `execs.output_tail` SQLite column. Because `crates/cortex-capture/src/exec.rs` buffers full process output in memory without a 1 MB or 10,000-line cap, avoid running commands that produce unbounded output under `cortex exec`.

## Prompt injection defense

Tombstone prose, error signatures, and invariant assertions written by untrusted contributors or fetched from a remote ledger could attempt to hijack an agent's reasoning. `crates/cortex-core/src/fence.rs` provides `sanitize_text`, `truncate_with_ellipsis`, `fence_tombstone`, and `fence_invariant` to quarantine record payloads before they reach an MCP response.

### Field length limits

`fence_tombstone` and `fence_invariant` bound rendered fields by Unicode scalar count:

| Record field | Cap (characters) | Constant or literal |
|---|---|---|
| `attempted_approach` | 600 | `MAX_APPROACH_LEN` |
| `failure_reason` | 800 | `MAX_REASON_LEN` |
| `error_signature` | 300 | `MAX_SIGNATURE_LEN` |
| `author_harness` | 100 | literal `100` |
| `assertion` | 800 | `MAX_REASON_LEN` |
| `verification_test_cmd` | 300 | literal `300` |

`record.id`, `contract.invariant_id`, and `contract.guarded_symbol.symbol_uri` pass through `sanitize_text` without length truncation.

### Stripping and delimiter neutralization

`sanitize_text` performs three deterministic passes on untrusted strings:

1. Escape sequence stripping removes 7-bit `ESC` (`\x1B`) and 8-bit `CSI` (`\u{009B}`) sequences, including `CSI` parameters and final bytes, `OSC` (`]`), `DCS` (`P`), `APC` (`_`), `PM` (`^`), and `SOS` (`X`) strings terminated by `BEL` (`\x07`), `ST` (`\u{009C}`), or `\x1B\\`, plus single-character ASCII `ESC` sequences.
2. Control and directional codepoint stripping removes `\0`, `\r`, all other C0/C1 `char::is_control()` characters except `\n` and `\t`, Arabic letter mark `\u{061C}`, zero-width and directional formatting codepoints (`\u{200B}..=\u{200F}`, `\u{202A}..=\u{202E}`, `\u{2060}..=\u{2069}`, `\u{FEFF}`), and Unicode Tag characters (`\u{E0000}..=\u{E007F}`).
3. Delimiter neutralization breaks HTML comment escapes and spoofed record markers:
   - `<!--` becomes `< !--`
   - `-->` becomes `-- >`
   - `CORTEX_TOMBSTONE_` becomes `CORTEX_TMB_`
   - `CORTEX_INVARIANT_` becomes `CORTEX_INV_`

### Provenance banners and fencing tags

Active tombstones and invariants returned by `inspect_symbol` are wrapped in structured HTML comments bearing a `Provenance` tag from `Provenance::as_banner()`:

- `LOCAL_WORKING_TREE` (`Provenance::LocalWorkingTree`)
- `LOCAL_CHECKPOINT` (`Provenance::LocalCheckpoint`)
- `LEDGER_COMMITTED` (`Provenance::LedgerShared`)
- `REMOTE_UNTRUSTED` (`Provenance::RemoteUntrusted`)

In `crates/cortex-engine/src/inspect.rs`, records stored with `origin == "LEDGER"` receive `LEDGER_COMMITTED`, and local records receive `LOCAL_WORKING_TREE`. `LOCAL_CHECKPOINT` and `REMOTE_UNTRUSTED` are available in `Provenance` for external or future callers.

The emitted wrappers follow these exact shapes:

```html
<!-- CORTEX_TOMBSTONE_BEGIN id="tmb_..." provenance="LEDGER_COMMITTED" harness="claude-code" -->
[TOMBSTONE: tmb_...]
- Approach Attempted: ...
- Why It Failed: ...
- Error Signature: ...
<!-- CORTEX_TOMBSTONE_END id="tmb_..." -->
```

```html
<!-- CORTEX_INVARIANT_BEGIN id="inv_..." severity="Blocking" provenance="LOCAL_WORKING_TREE" -->
[INVARIANT LOCK: inv_...] (Severity: Blocking)
- Guarded Symbol: ...
- Architectural Assertion: ...
- Note: Verification test `...` must pass before modifying dependencies.
<!-- CORTEX_INVARIANT_END id="inv_..." -->
```

In `inspect_symbol`, records that have already drifted or been revoked emit summary comments (`<!-- STALE TOMBSTONE: ... -->`, `<!-- STALE INVARIANT: ... -->`, `<!-- REVOKED TOMBSTONE: ... -->`, `<!-- REVOKED INVARIANT: ... -->`) without passing the drift summary or revocation reason through `sanitize_text`.

## Credential and secret scrubbing

`crates/cortex-core/src/redact.rs` exposes `scrub(input) -> (String, Vec<RedactionFinding>)` backed by precompiled `LazyLock` regular expressions and a high-entropy token pass. In `b143df9`, `record_tombstone` calls `scrub` on `attempted_approach`, `failure_reason`, `test_command`, and `error_signature` before inserting into SQLite or promoting to `refs/cortex/ledger`.

### The 13 regex redaction rules

| Pattern target | Regular expression summary | Replacement token |
|---|---|---|
| AWS access key ID | `\b(AKIA[0-9A-Z]{16})\b` | `[REDACTED:AWS_KEY]` |
| AWS STS temporary key | `\b(ASIA[0-9A-Z]{16})\b` | `[REDACTED:AWS_KEY]` |
| GitHub token | `\b(gh[pousr]_[A-Za-z0-9_]{36,255})\b` | `[REDACTED:GITHUB_TOKEN]` |
| GitHub fine-grained PAT | `\b(github_pat_[A-Za-z0-9_]{82})\b` | `[REDACTED:GITHUB_PAT]` |
| Bearer token | `(?i)\bBearer\s+([A-Za-z0-9\-._~+/]{24,})\b` | `Bearer [REDACTED:BEARER_TOKEN]` |
| Key/secret assignment | `(?i)(api[_-]?key\|secret\|token\|password\|auth[_-]?token)\s*[:=]\s*['"]?([A-Za-z0-9\-._~+/]{16,})['"]?` | `$1=[REDACTED:SECRET]` |
| PEM private key block | `-----BEGIN\s+[A-Z\s]+PRIVATE\s+KEY-----[\s\S]*?-----END\s+[A-Z\s]+PRIVATE\s+KEY-----` | `[REDACTED:PRIVATE_KEY]` |
| URI userinfo credentials | `((?:https?\|postgres\|postgresql\|mysql\|redis\|rediss\|mongodb\|mongodb\+srv\|amqp\|amqps)://)([^:\s]+):([^@\s]+)@` | `${1}[REDACTED_CREDS]@` |
| Slack token | `\b(xox[baprs]-[0-9a-zA-Z]{10,48})\b` | `[REDACTED:SLACK_TOKEN]` |
| OpenAI project key | `\b(sk-proj-[A-Za-z0-9_-]{48,})\b` | `[REDACTED:API_KEY]` |
| Anthropic API key | `\b(sk-ant-api03-[A-Za-z0-9_-]{80,})\b` | `[REDACTED:API_KEY]` |
| Standard `sk-` API key | `\b(sk-[A-Za-z0-9]{20,64})\b` | `[REDACTED:API_KEY]` |
| Google API key | `\b(AIza[0-9A-Za-z-_]{35})\b` | `[REDACTED:GOOGLE_API_KEY]` |

### High-entropy scanner and false-positive filter

After the 13 regex rules run, `scrub_high_entropy_words` scans tokens matching `\b([A-Za-z0-9+/=_-]{32,})\b`. A token is replaced with `[REDACTED:HIGH_ENTROPY]` when all of these conditions hold:

1. It is not pure hexadecimal (`!is_pure_hex`), which avoids masking SHA-256 commit IDs and AST digests.
2. It contains at least one ASCII lowercase letter, one ASCII uppercase letter, and one ASCII digit.
3. Its Shannon entropy exceeds `3.5` bits per character.
4. `looks_like_camel_identifier` returns `false`. That helper accepts only `[A-Za-z0-9_]+` strings with both uppercase and lowercase ASCII letters and checks whether the string contains at least two title-case segments matching `[A-Z][a-z]{2,}` (for example, `OAuth2TokenRefreshV2PayloadHandler`).

### Known redaction gaps

- There is no dedicated rule for JWT strings (`eyJ...`). A JWT is masked only if it follows `Bearer ` (with 24 or more valid characters), appears in a key/secret assignment matching rule 6, or contains individual dot-separated segments of 32 or more characters that satisfy the high-entropy pass.
- `github_pat_` requires an exact 82-character suffix.
- Generic `sk-` matching (`sk-[A-Za-z0-9]{20,64}`) excludes hyphens and underscores, so non-`sk-proj-` / non-`sk-ant-api03-` dashed `sk-` secrets rely on the high-entropy fallback.
- `crates/cortex-capture/src/exec.rs` writes `execs.signature` and `execs.output_tail` to `.git/cortex/state.db` without calling `scrub`; redaction runs only when an error string flows into `record_tombstone`. `InvariantContract` fields (`assertion` and `verification_test_cmd`) and `EpistemicHandoff` fields are also stored without `scrub`.

## Path boundaries, ledger isolation, and worktrees

- `crates/cortex-ast/src/uri.rs` provides `normalize_rel_path`, which converts backslashes to `/`, strips Windows drive prefixes (`C:`), drops empty and `.` segments, and pops `..` segments. `parse_symbol_uri` splits on the last `#` (`rsplit_once('#')`) so paths containing `#` remain valid and normalizes the path segment. `CortexError::PathOutOfBounds` is defined in `crates/cortex-core/src/error.rs`, although filesystem reads join normalized relative paths onto `repo.workdir` directly without a post-join canonical prefix check.
- `crates/cortex-storage/src/ledger.rs` commits promoted tombstones, invariants, and lifecycle events to `refs/cortex/ledger`, outside `refs/heads/*` and `refs/tags/*`.
- `git cortex init` writes `+refs/cortex/ledger:refs/remotes/origin/cortex/ledger` to `remote.origin.fetch` and migrates any legacy `+refs/cortex/ledger:refs/cortex/ledger` refspec so remote fetches do not overwrite local ledger commits before merging.
- Records are written at `v1/<kind>/<shard>/<id>.json`, where `<shard>` is the trailing 2 lowercase characters of the ULID record ID (or `00` if shorter than 2 characters).
- Record structs serialize with RFC 8785 canonical JSON (`serde_jcs::to_string`) and hash with SHA-256 (`sha256:<64-hex>`).
- During `sync`, `CortexLedger::merge_commits` merges disjoint paths by tree union and aborts with `Tamper alert: conflicting content detected for record path '...'` if local and remote trees contain different blob OIDs at the same path. Note that `reaffirm_invariant` in `v0.1.0` rewrites an existing `INVARIANT` JSON with `promoted = false`, which can trigger this tamper alert when two branches reaffirm and promote the same invariant ID differently.
- `CortexRepo::resolve_commondir` follows `<gitdir>/commondir` so linked `git worktree` checkouts share the primary repository's `state.db` under `<commondir>/cortex/state.db` and Git ref namespace.

## Reporting a vulnerability

Report suspected security vulnerabilities privately through GitHub Security Advisories at `https://github.com/augastinklazar/git-cortex/security/advisories`.

Do not open public GitHub issues for undisclosed security vulnerabilities. Maintainers acknowledge reports within 48 hours.
