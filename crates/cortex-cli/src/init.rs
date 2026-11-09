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
    let safe_ledger_refspec = "+refs/cortex/ledger:refs/remotes/origin/cortex/ledger";
    let default_heads_refspec = "+refs/heads/*:refs/remotes/origin/*";
    let fetch_key = "remote.origin.fetch";

    let mut has_heads = false;
    let mut has_safe_ledger = false;
    let mut has_dangerous_ledger = false;

    if let Ok(mut entries) = git_config.entries(Some(fetch_key)) {
        while let Some(Ok(entry)) = entries.next() {
            if let Some(val) = entry.value() {
                if val.contains("refs/heads/") {
                    has_heads = true;
                }
                if val == safe_ledger_refspec {
                    has_safe_ledger = true;
                }
                if val == "+refs/cortex/ledger:refs/cortex/ledger" {
                    has_dangerous_ledger = true;
                }
            }
        }
    }

    if has_dangerous_ledger {
        // Replace dangerous direct-overwrite refspec with safe remote-tracking refspec
        let _ = git_config.set_multivar(
            fetch_key,
            r"^\+refs/cortex/ledger:refs/cortex/ledger$",
            safe_ledger_refspec,
        );
        actions.push(
            "Updated dangerous git fetch refspec to remote-tracking refs/remotes/origin/cortex/ledger"
                .to_string(),
        );
    } else if !has_safe_ledger {
        // Append safe ledger refspec without replacing existing entries
        let _ = git_config.set_multivar(fetch_key, "^$", safe_ledger_refspec);
        actions.push("Configured git fetch refspec for refs/cortex/ledger".to_string());
    }

    // Ensure primary branch fetch refspec is preserved
    if !has_heads {
        let _ = git_config.set_multivar(fetch_key, "^$", default_heads_refspec);
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
        let msg = setup_or_merge_mcp_config(&mcp_json_path)?;
        actions.push(msg);
    }

    Ok(actions)
}

fn setup_or_merge_mcp_config(mcp_json_path: &Path) -> Result<String> {
    let git_cortex_entry = serde_json::json!({
        "command": "git",
        "args": ["cortex", "mcp-serve"]
    });

    if mcp_json_path.exists() {
        let content = fs::read_to_string(mcp_json_path).map_err(CortexError::Io)?;
        let mut root: serde_json::Value =
            serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));
        if !root.is_object() {
            root = serde_json::json!({});
        }

        let mcp_servers = root
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));

        if !mcp_servers.is_object() {
            *mcp_servers = serde_json::json!({});
        }

        mcp_servers
            .as_object_mut()
            .unwrap()
            .insert("git-cortex".to_string(), git_cortex_entry);

        fs::write(
            mcp_json_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&root).unwrap_or_default()
            ),
        )
        .map_err(CortexError::Io)?;

        Ok("Merged git-cortex into existing .mcp.json".to_string())
    } else {
        let root = serde_json::json!({
            "mcpServers": {
                "git-cortex": git_cortex_entry
            }
        });
        fs::write(
            mcp_json_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&root).unwrap_or_default()
            ),
        )
        .map_err(CortexError::Io)?;

        Ok("Generated .mcp.json configuration for MCP harnesses".to_string())
    }
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
        "\n{}\nif command -v git-cortex >/dev/null 2>&1; then\n    git-cortex hook {} || exit $?\nelif command -v cortex >/dev/null 2>&1; then\n    cortex hook {} || exit $?\nfi\n{}\n",
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

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use tempfile::TempDir;

    #[test]
    fn test_init_preserves_heads_refspec_and_sets_safe_ledger_refspec() {
        let temp = TempDir::new().unwrap();
        let git_repo = Repository::init(temp.path()).unwrap();

        // Simulate a remote 'origin' configured with default fetch refspec
        let mut config = git_repo.config().unwrap();
        config
            .set_multivar(
                "remote.origin.fetch",
                "^$",
                "+refs/heads/*:refs/remotes/origin/*",
            )
            .unwrap();

        let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
        let opts = InitOptions {
            install_hooks: true,
            setup_agents_md: true,
            setup_mcp_config: true,
        };

        let actions = run_init(&cortex_repo, &opts).unwrap();
        assert!(!actions.is_empty());

        // Verify refspecs in config
        let updated_config = git_repo.config().unwrap();
        let mut refspecs = Vec::new();
        if let Ok(mut entries) = updated_config.entries(Some("remote.origin.fetch")) {
            while let Some(Ok(entry)) = entries.next() {
                if let Some(val) = entry.value() {
                    refspecs.push(val.to_string());
                }
            }
        }

        // Heads refspec MUST still be present
        assert!(
            refspecs
                .iter()
                .any(|r| r == "+refs/heads/*:refs/remotes/origin/*"),
            "Default heads fetch refspec was wiped out!"
        );

        // Safe ledger tracking refspec MUST be present
        assert!(
            refspecs
                .iter()
                .any(|r| r == "+refs/cortex/ledger:refs/remotes/origin/cortex/ledger"),
            "Safe remote-tracking ledger refspec is missing!"
        );

        // Direct clobbering refspec MUST NOT be present
        assert!(
            !refspecs
                .iter()
                .any(|r| r == "+refs/cortex/ledger:refs/cortex/ledger"),
            "Dangerous clobbering refspec is present!"
        );
    }

    #[test]
    fn test_init_migrates_dangerous_refspec_to_safe() {
        let temp = TempDir::new().unwrap();
        let git_repo = Repository::init(temp.path()).unwrap();

        let mut config = git_repo.config().unwrap();
        config
            .set_multivar(
                "remote.origin.fetch",
                "^$",
                "+refs/cortex/ledger:refs/cortex/ledger",
            )
            .unwrap();

        let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
        let opts = InitOptions {
            install_hooks: false,
            setup_agents_md: false,
            setup_mcp_config: false,
        };

        let _ = run_init(&cortex_repo, &opts).unwrap();

        let updated_config = git_repo.config().unwrap();
        let mut refspecs = Vec::new();
        if let Ok(mut entries) = updated_config.entries(Some("remote.origin.fetch")) {
            while let Some(Ok(entry)) = entries.next() {
                if let Some(val) = entry.value() {
                    refspecs.push(val.to_string());
                }
            }
        }

        assert!(
            !refspecs
                .iter()
                .any(|r| r == "+refs/cortex/ledger:refs/cortex/ledger"),
            "Dangerous refspec was not migrated!"
        );
        assert!(
            refspecs
                .iter()
                .any(|r| r == "+refs/cortex/ledger:refs/remotes/origin/cortex/ledger"),
            "Migrated safe refspec is missing!"
        );
        assert!(
            refspecs
                .iter()
                .any(|r| r == "+refs/heads/*:refs/remotes/origin/*"),
            "Missing default heads refspec was not restored!"
        );
    }

    #[test]
    fn test_init_merges_mcp_json_preserving_existing_servers() {
        let temp = TempDir::new().unwrap();
        let _git_repo = Repository::init(temp.path()).unwrap();
        let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();

        let mcp_path = temp.path().join(".mcp.json");
        let initial_config = serde_json::json!({
            "mcpServers": {
                "other-tool": {
                    "command": "node",
                    "args": ["server.js"]
                }
            }
        });
        fs::write(
            &mcp_path,
            serde_json::to_string_pretty(&initial_config).unwrap(),
        )
        .unwrap();

        let opts = InitOptions {
            install_hooks: false,
            setup_agents_md: false,
            setup_mcp_config: true,
        };

        let actions = run_init(&cortex_repo, &opts).unwrap();
        assert!(actions
            .iter()
            .any(|a| a.contains("Merged git-cortex into existing .mcp.json")));

        let content = fs::read_to_string(&mcp_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let servers = parsed.get("mcpServers").unwrap();

        // Both 'other-tool' and 'git-cortex' must be present!
        assert!(servers.get("other-tool").is_some());
        assert!(servers.get("git-cortex").is_some());
        assert_eq!(
            servers["git-cortex"]["args"],
            serde_json::json!(["cortex", "mcp-serve"])
        );
    }
}
