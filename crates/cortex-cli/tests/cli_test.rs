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
    let opts = cortex_cli::init::InitOptions {
        install_hooks: true,
        setup_agents_md: true,
        setup_mcp_config: true,
    };
    cortex_cli::init::run_init(repo, &opts).unwrap()
}

#[test]
fn test_cli_tombstone_and_invariant_show() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let file = temp.path().join("lib.rs");
    fs::write(&file, "pub fn helper() -> bool { true }\n").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
    let mut db = cortex_storage::CortexDb::open_in_memory().unwrap();

    // 1. Record a tombstone
    let tmb = cortex_engine::tombstone::record_tombstone(
        &cortex_repo,
        &mut db,
        cortex_engine::tombstone::RecordTombstoneParams {
            target_symbols: &["lib.rs#helper".to_string()],
            attempted_approach: "Direct pointer arithmetic",
            failure_reason: "Undefined behavior in Miri",
            test_command: "cargo miri test",
            error_signature: "Miri UB check failed",
            author_harness: "cli-test",
            capture_mode: cortex_core::schema::CaptureMode::Manual,
        },
    )
    .unwrap();

    // 2. Record an invariant
    let inv = cortex_engine::invariant::record_invariant(
        &cortex_repo,
        &mut db,
        "lib.rs#helper",
        "helper must remain pure",
        &[],
        "cargo test",
        cortex_core::schema::InvariantSeverity::Blocking,
    )
    .unwrap();

    // Verify records can be retrieved and parsed as structured models
    let tmb_rec = db.get_record(&tmb.id).unwrap().unwrap();
    let parsed_tmb: cortex_core::schema::TombstoneRecord =
        serde_json::from_str(&tmb_rec.json).unwrap();
    assert_eq!(parsed_tmb.id, tmb.id);
    assert_eq!(parsed_tmb.attempted_approach, "Direct pointer arithmetic");

    let inv_rec = db.get_record(&inv.invariant_id).unwrap().unwrap();
    let parsed_inv: cortex_core::schema::InvariantContract =
        serde_json::from_str(&inv_rec.json).unwrap();
    assert_eq!(parsed_inv.invariant_id, inv.invariant_id);
    assert_eq!(parsed_inv.assertion, "helper must remain pure");
}
