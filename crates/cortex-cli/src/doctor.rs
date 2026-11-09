use cortex_core::error::Result;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo};

pub fn run_doctor(repo: &CortexRepo, db: &CortexDb) -> Result<()> {
    println!("=== git-cortex Doctor Diagnostics ===");

    // 1. Repo info
    println!("✓ Repository Root: {}", repo.workdir.display());
    println!("✓ Git Common Dir:  {}", repo.git_dir.display());
    let branch = repo
        .current_branch()
        .unwrap_or_else(|_| "unknown".to_string());
    let head = repo
        .head_commit_sha()
        .unwrap_or_else(|_| "unknown".to_string());
    println!(
        "✓ Current Branch:  {} ({})",
        branch,
        &head[..8.min(head.len())]
    );

    // 2. State DB
    let db_path = repo.state_db_path();
    if db_path.exists() {
        let t_count = db
            .list_records_by_kind("TOMBSTONE")
            .map(|v| v.len())
            .unwrap_or(0);
        let i_count = db
            .list_records_by_kind("INVARIANT")
            .map(|v| v.len())
            .unwrap_or(0);
        println!(
            "✓ SQLite State DB: {} ({} Tombstones, {} Invariants)",
            db_path.display(),
            t_count,
            i_count
        );
    } else {
        println!("! SQLite State DB: not found (run `git cortex init`)");
    }

    // 3. Distributed Ledger Ref
    match CortexLedger::current_commit(&repo.repo) {
        Ok(Some(commit)) => {
            println!(
                "✓ Ledger Ref:      refs/cortex/ledger (Commit: {})",
                commit.id()
            );
        }
        Ok(None) => {
            println!("- Ledger Ref:      refs/cortex/ledger (empty / not yet initialized)");
        }
        Err(e) => {
            println!("! Ledger Ref:      error reading: {}", e);
        }
    }

    // 4. Hooks
    let custom_hooks_path = repo
        .repo
        .config()
        .ok()
        .and_then(|c| c.get_string("core.hooksPath").ok());
    let hooks_dir = match custom_hooks_path {
        Some(custom) if !custom.trim().is_empty() => {
            let p = std::path::Path::new(&custom);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                repo.workdir.join(p)
            }
        }
        _ => repo.git_dir.join("hooks"),
    };

    let hooks = ["pre-commit", "post-commit", "post-checkout", "post-merge"];
    let mut installed_hooks = Vec::new();
    for h in hooks {
        let p = hooks_dir.join(h);
        if p.exists() {
            if let Ok(c) = std::fs::read_to_string(&p) {
                if c.contains("git-cortex") {
                    installed_hooks.push(h);
                }
            }
        }
    }
    if !installed_hooks.is_empty() {
        println!(
            "✓ Git Hooks:       {} installed ({}) in {}",
            installed_hooks.len(),
            installed_hooks.join(", "),
            hooks_dir.display()
        );
    } else {
        println!(
            "! Git Hooks:       none installed in {} (run `git cortex init --hooks`)",
            hooks_dir.display()
        );
    }

    // 5. Instruction blocks
    let agents_md = repo.workdir.join("AGENTS.md");
    let claude_md = repo.workdir.join("CLAUDE.md");
    let has_agents = agents_md.exists()
        && std::fs::read_to_string(&agents_md)
            .map(|c| c.contains("git-cortex"))
            .unwrap_or(false);
    let has_claude = claude_md.exists()
        && std::fs::read_to_string(&claude_md)
            .map(|c| c.contains("git-cortex"))
            .unwrap_or(false);
    let instr_symbol = if has_agents && has_claude {
        "✓"
    } else if has_agents || has_claude {
        "!"
    } else {
        "-"
    };
    println!(
        "{} Instructions:    AGENTS.md: {}, CLAUDE.md: {}",
        instr_symbol,
        if has_agents { "active" } else { "missing" },
        if has_claude { "active" } else { "missing" }
    );

    // 6. MCP Config
    let mcp_json = repo.workdir.join(".mcp.json");
    if mcp_json.exists() {
        println!("✓ MCP Server:      .mcp.json configured");
    } else {
        println!("- MCP Server:      .mcp.json not present");
    }

    println!("=====================================");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use tempfile::TempDir;

    #[test]
    fn test_doctor_detects_custom_hooks_path() {
        let temp = TempDir::new().unwrap();
        let git_repo = Repository::init(temp.path()).unwrap();

        let mut config = git_repo.config().unwrap();
        config.set_str("core.hooksPath", ".custom_hooks").unwrap();

        let custom_hooks_dir = temp.path().join(".custom_hooks");
        std::fs::create_dir_all(&custom_hooks_dir).unwrap();
        std::fs::write(
            custom_hooks_dir.join("pre-commit"),
            "#!/bin/sh\ngit-cortex hook pre-commit\n",
        )
        .unwrap();

        let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
        let db = CortexDb::open_in_memory().unwrap();

        assert!(run_doctor(&cortex_repo, &db).is_ok());
    }
}
