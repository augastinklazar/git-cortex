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
    let hooks = ["pre-commit", "post-commit", "post-checkout", "post-merge"];
    let mut installed_hooks = Vec::new();
    for h in hooks {
        let p = repo.git_dir.join("hooks").join(h);
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
            "✓ Git Hooks:       {} installed ({})",
            installed_hooks.len(),
            installed_hooks.join(", ")
        );
    } else {
        println!("! Git Hooks:       none installed (run `git cortex init --hooks`)");
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
    println!(
        "✓ Instructions:    AGENTS.md: {}, CLAUDE.md: {}",
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
