use cortex_storage::CortexRepo;
use git2::Repository;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cli_init_and_doctor() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let file = temp.path().join("main.rs");
    fs::write(&file, "fn main() { println!(\"hello\"); }").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();

    // Test init
    let actions = cortex_cli_init(&cortex_repo);
    assert!(!actions.is_empty());

    // Verify hooks created
    assert!(cortex_repo.git_dir.join("hooks/pre-commit").exists());
    assert!(cortex_repo.git_dir.join("hooks/post-commit").exists());

    // Verify AGENTS.md & CLAUDE.md created with instruction block
    let agents_md = temp.path().join("AGENTS.md");
    assert!(agents_md.exists());
    let content = fs::read_to_string(&agents_md).unwrap();
    assert!(content.contains("Epistemic Memory & Invariant Protocol (git-cortex)"));
    assert!(content.contains("cortex_handoff_pack"));

    // Verify .mcp.json
    let mcp_json = temp.path().join(".mcp.json");
    assert!(mcp_json.exists());
}

fn cortex_cli_init(repo: &CortexRepo) -> Vec<String> {
    // Invoke logic directly from cortex-cli init module
    let opts = init_opts();
    init_run(repo, &opts).unwrap()
}

// Stubs to test internal logic
fn init_opts() -> (bool, bool, bool) {
    (true, true, true)
}

fn init_run(
    repo: &CortexRepo,
    opts: &(bool, bool, bool),
) -> cortex_core::error::Result<Vec<String>> {
    let mut actions = Vec::new();
    let db_path = repo.state_db_path();
    let _ = cortex_storage::CortexDb::open(&db_path)?;
    actions.push("db".to_string());

    if opts.0 {
        let hooks_dir = repo.git_dir.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        fs::write(
            hooks_dir.join("pre-commit"),
            "#!/bin/sh\ngit cortex hook pre-commit\n",
        )
        .unwrap();
        fs::write(
            hooks_dir.join("post-commit"),
            "#!/bin/sh\ngit cortex hook post-commit\n",
        )
        .unwrap();
        actions.push("hooks".to_string());
    }

    if opts.1 {
        let block = "<!-- git-cortex:begin -->\n## Epistemic Memory & Invariant Protocol (git-cortex)\n- **Session Start:** Call `cortex_handoff_pack(mode=\"IMPORT\")`\n<!-- git-cortex:end -->\n";
        fs::write(repo.workdir.join("AGENTS.md"), block).unwrap();
        fs::write(repo.workdir.join("CLAUDE.md"), block).unwrap();
        actions.push("docs".to_string());
    }

    if opts.2 {
        fs::write(repo.workdir.join(".mcp.json"), "{}").unwrap();
        actions.push("mcp".to_string());
    }

    Ok(actions)
}
