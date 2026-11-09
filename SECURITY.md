# Security Architecture & Threat Model: git-cortex

This document defines the security guarantees, boundaries, and threat mitigation strategies implemented in `git-cortex`.

---

## 1. Threat Modeling & Core Boundaries

`git-cortex` operates at the boundary between developer repositories, local git internals, and autonomous Large Language Model (LLM) agents. We evaluate threats across five primary vectors:

```
+--------------------------------------------------------------------------------+
|                                Threat Matrix                                   |
+------------------------------+--------------------+----------------------------+
| Threat                       | Impact             | Mitigation Mechanism       |
+------------------------------+--------------------+----------------------------+
| Arbitrary Code Execution     | Critical           | No auto-exec of test cmds  |
| Prompt Injection             | High               | Unicode Bidi + ANSI scrub  |
| Credential Leakage in Ledger | High               | Automated regex redactor   |
| Path Traversal Attacks       | High               | Relative path sanitization |
| Ledger Ref Tampering         | Medium             | Isolated refs namespace    |
+------------------------------+--------------------+----------------------------+
```

---

## 2. Remote Code Execution (RCE) Defenses

### Invariant & Tombstone Command Safety
- Both `InvariantContract` (`verification_test_cmd`) and `TombstoneRecord` (`reproduction_cmd`) accept shell command strings.
- **Zero Automatic Execution**: Neither the MCP server nor the CLI hook automatically executes arbitrary commands stored in records pulled from git remotes or branches.
- Verification commands are printed strictly as recommendations/advisories to the human operator or agent, requiring deliberate human-supervised execution.
- Command execution via `cortex exec` enforces configurable timeouts and output truncation caps (default 1 MB / 10,000 lines) to prevent resource exhaustion and hanging subshells.

---

## 3. Prompt Injection Defense

Agent memory engines can be manipulated by malicious code comments or adversarial pull requests containing prompt injections designed to hijack agent reasoning.

### Sanitization Pipeline (`cortex_core::sanitize`)
Before any tombstone or invariant text is presented to an LLM context:
1. **ANSI Sequence Stripping**: All terminal escape codes (e.g., color sequences, cursor jumps) are removed.
2. **Unicode Bidirectional Override Removal**: Control characters used in Trojan Source attacks (`\u202A` through `\u202E`, `\u2066` through `\u2069`) are stripped.
3. **Payload Truncation**: Strings exceeding 2,000 characters are safely capped.
4. **Fencing Headers**: Records are wrapped in structured delimiter banners:
   ```markdown
   <<< BEGIN EPIDEMIOLOGICAL FENCED TOMBSTONE >>>
   ID: tmb_...
   Target: src/auth.rs::verify_token
   Confidence: 0.95
   Failure Reason: ...
   <<< END EPIDEMIOLOGICAL FENCED TOMBSTONE >>>
   ```

---

## 4. Credential & Secret Scrubbing

To prevent sensitive API keys, tokens, and database passwords from being persisted in the SQLite database or committed to `refs/cortex/ledger`, `git-cortex` runs all stdout/stderr captures and error signatures through `cortex_core::redact::scrub`.

### Pattern Detection
The automated scrubber detects and masks:
- Anthropic API keys (`sk-ant-api03-...`)
- OpenAI API keys (`sk-...`)
- GitHub Personal Access Tokens (`ghp_...`, `github_pat_...`)
- AWS Access Keys (`AKIA...`)
- Generic Bearer tokens and JSON Web Tokens (`eyJ...`)
- High-entropy private keys (`-----BEGIN PRIVATE KEY-----`)
- Password patterns in connection URIs (`postgres://user:pass@host`)

Matched strings are replaced with `[REDACTED_SECRET]` prior to database insertion.

---

## 5. Ledger Integrity & Ref Namespace Isolation

- **Isolated Refspace**: All persistent epistemic records are committed to `refs/cortex/ledger`, outside developer branch heads (`refs/heads/*`) and tags (`refs/tags/*`).
- **Immutable Sharded Paths**: Records are stored under monotonic ULID paths (`v1/<kind>/<shard>/<ulid>.json`). Overwriting existing records is disallowed.
- **Deterministic Canon**: Canonical JSON serialization (RFC 8785) is enforced to ensure byte-exact SHA hashing across different machine architectures and runtimes.

---

## 6. Reporting a Vulnerability

If you discover a security vulnerability in `git-cortex`, please submit a private report via GitHub Security Advisories at:
`https://github.com/augastinklazar/git-cortex/security/advisories`

Please do not open public issues for undisclosed security flaws. All reports will be acknowledged within 48 hours.
