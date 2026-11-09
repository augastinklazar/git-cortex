use cortex_core::error::{CortexError, Result};
use cortex_storage::{CortexDb, CortexRepo};
use std::fs;
use std::path::Path;

pub const AGENTS_MD_BLOCK: &str = r#"<!-- git-cortex:begin -->
## Epistemic Memory & Invariant Protocol (git-cortex)
- **Session Start:** Call `cortex_handoff_pack(mode="IMPORT")` before running broad repo searches to load verified call-graphs and active hypotheses.
- **Before Editing Symbols:** Call `cortex_inspect_symbol(filepath)` before modifying existing functions. Respect all ACTIVE Tombstones and Invariant Contracts.
- **On Failed Experiments:** Whenever a test fails and you revert or abandon an approach, call `cortex_record_tombstone` before trying an alternative.
- **Before Committing:** Call `cortex_verify_invariants(diff_scope="STAGED")` to verify cross-file causal contracts.
<!-- git-cortex:end -->
"#;

pub struct InitOptions {
    pub install_hooks: bool,
    pub setup_agents_md: bool,
    pub setup_mcp_config: bool,
}

pub fn run_init(repo: &CortexRepo, opts: &InitOptions) -> Result<Vec<String>> {
    let mut actions = Vec::new();

    // 1. Initialize SQLite database
    let db_path = repo.state_db_path();
    let _ = CortexDb::open(&db_path)?;
    actions.push(format!(
        "Initialized SQLite state database at {}",
        db_path.display()
    ));

    // 2. Configure fetch refspec for refs/cortex/ledger in git config
    let mut git_config = repo
        .repo
        .config()
        .map_err(|e| CortexError::Git(e.to_string()))?;
    let fetch_refspec = "+refs/cortex/ledger:refs/cortex/ledger";
    let fetch_key = "remote.origin.fetch";

    let mut exists = false;
    {
        if let Ok(mut entries) = git_config.entries(Some(fetch_key)) {
            while let Some(Ok(entry)) = entries.next() {
                if let Some(val) = entry.value() {
                    if val.contains("refs/cortex/ledger") {
                        exists = true;
                        break;
                    }
                }
            }
        }
    }

    if !exists {
        let _ = git_config.set_multivar(fetch_key, ".*", fetch_refspec);
        actions.push("Configured git fetch refspec for refs/cortex/ledger".to_string());
    }

    // 3. Install chained git hooks
    if opts.install_hooks {
        let hooks_dir = match git_config.get_string("core.hooksPath") {
            Ok(custom) if !custom.trim().is_empty() => {
                let p = Path::new(&custom);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    repo.workdir.join(p)
                }
            }
            _ => repo.git_dir.join("hooks"),
        };
        fs::create_dir_all(&hooks_dir).map_err(CortexError::Io)?;

        install_chained_hook(&hooks_dir, "pre-commit", "pre-commit")?;
        install_chained_hook(&hooks_dir, "post-commit", "post-commit")?;
        install_chained_hook(&hooks_dir, "post-checkout", "post-checkout")?;
        install_chained_hook(&hooks_dir, "post-merge", "post-merge")?;
        actions.push(
            "Installed native git hooks (pre-commit, post-commit, post-checkout, post-merge)"
                .to_string(),
        );
    }

    // 4. Update AGENTS.md and CLAUDE.md
    if opts.setup_agents_md {
        for target_doc in &["AGENTS.md", "CLAUDE.md"] {
            let doc_path = repo.workdir.join(target_doc);
            append_block_if_missing(&doc_path, AGENTS_MD_BLOCK)?;
            actions.push(format!(
                "Configured normative instructions in {}",
                target_doc
            ));
        }
    }

    // 5. Configure MCP server in .mcp.json
    if opts.setup_mcp_config {
        let mcp_json_path = repo.workdir.join(".mcp.json");
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "git-cortex": {
                    "command": "git",
                    "args": ["cortex", "mcp-serve"]
                }
            }
        });

        if !mcp_json_path.exists() {
            fs::write(
                &mcp_json_path,
                serde_json::to_string_pretty(&mcp_config).unwrap(),
            )
            .map_err(CortexError::Io)?;
            actions.push("Generated .mcp.json configuration for MCP harnesses".to_string());
        } else {
            actions.push(".mcp.json already exists; leaving unchanged".to_string());
        }
    }

    Ok(actions)
}

fn install_chained_hook(hooks_dir: &Path, hook_name: &str, cortex_cmd: &str) -> Result<()> {
    let hook_path = hooks_dir.join(hook_name);
    let marker_begin = "# --- git-cortex:begin ---";
    let marker_end = "# --- git-cortex:end ---";

    let mut existing = if hook_path.exists() {
        fs::read_to_string(&hook_path).unwrap_or_default()
    } else {
        "#!/bin/sh\n".to_string()
    };

    if existing.contains(marker_begin) {
        return Ok(());
    }

    let hook_snippet = format!(
        "\n{}\nif command -v git-cortex >/dev/null 2>&1; then\n    git-cortex hook {}\nelif command -v cortex >/dev/null 2>&1; then\n    cortex hook {}\nfi\n{}\n",
        marker_begin, cortex_cmd, cortex_cmd, marker_end
    );

    existing.push_str(&hook_snippet);
    fs::write(&hook_path, existing).map_err(CortexError::Io)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(&hook_path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&hook_path, perms);
        }
    }

    Ok(())
}

fn append_block_if_missing(file_path: &Path, block: &str) -> Result<()> {
    let mut content = if file_path.exists() {
        fs::read_to_string(file_path).unwrap_or_default()
    } else {
        String::new()
    };

    if !content.contains("<!-- git-cortex:begin -->") {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(block);
        fs::write(file_path, content).map_err(CortexError::Io)?;
    }

    Ok(())
}
