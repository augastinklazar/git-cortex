use cortex_storage::*;
use git2::Repository;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_ledger_append_and_reindex() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();

    // Initial commit so HEAD exists
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let tree_id = {
        let mut index = repo.index().unwrap();
        index.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .unwrap();

    let record = LedgerRecord {
        id: "tmb_01TEST123456789".to_string(),
        kind: "tombstones".to_string(),
        json: r#"{"$schema":"https://git-cortex.dev/schemas/v0.1/tombstone.json","id":"tmb_01TEST123456789","schema_version":"0.1.0","created_at":"2026-11-09T09:30:00Z","author_harness":"test","branch_context":"main","base_commit_sha":"0000","capture_mode":"MANUAL","target_symbols":[{"symbol_uri":"src/lib.rs#test","ast_hash":"ast:sha256:1111"}],"attempted_approach":"test approach","failure_reason":"test failure","verification_evidence":{"command":"cargo test","exit_code":1,"error_signature":"err"},"invalidation_policy":{"invalidate_on_any_ast_change":[],"status":"ACTIVE"}}"#.to_string(),
    };

    let commit_oid =
        CortexLedger::append_records(&repo, &[record], "Test Author", "author@example.com")
            .unwrap();

    assert!(!commit_oid.is_zero());

    // Verify commit exists on LEDGER_REF
    let ledger_commit = CortexLedger::current_commit(&repo).unwrap();
    assert!(ledger_commit.is_some());
    assert_eq!(ledger_commit.unwrap().id(), commit_oid);

    // Reindex into fresh DB
    let mut db = CortexDb::open_in_memory().unwrap();
    let count = CortexLedger::reindex_into_db(&repo, &mut db).unwrap();
    assert_eq!(count, 1);

    let queried = db.get_record("tmb_01TEST123456789").unwrap();
    assert!(queried.is_some());
    let rec = queried.unwrap();
    assert_eq!(rec.origin, "LEDGER");
    assert!(rec.promoted);

    let sym_records = db.query_records_by_symbol("src/lib.rs#test").unwrap();
    assert_eq!(sym_records.len(), 1);
}

#[test]
fn test_snapshot_and_restore() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();

    // Create file
    let file_path = temp.path().join("code.txt");
    fs::write(&file_path, "version 1").unwrap();

    let (_, snap_ref) = CortexSnapshot::create_snapshot(
        &repo,
        "checkpoints",
        "chk_01TEST",
        "checkpoint 1",
        "Test",
        "test@example.com",
    )
    .unwrap();

    // Mutate file and add untracked file
    fs::write(&file_path, "version 2 (broken)").unwrap();
    let untracked = temp.path().join("junk.tmp");
    fs::write(&untracked, "untracked junk").unwrap();

    assert_eq!(
        fs::read_to_string(&file_path).unwrap(),
        "version 2 (broken)"
    );
    assert!(untracked.exists());

    // Restore
    CortexSnapshot::restore_snapshot(&repo, &snap_ref).unwrap();

    assert_eq!(fs::read_to_string(&file_path).unwrap(), "version 1");
    assert!(!untracked.exists(), "Untracked file should be cleaned");
}

#[test]
fn test_snapshot_does_not_mutate_live_git_index() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();

    let initial_file = temp.path().join("base.txt");
    fs::write(&initial_file, "base content").unwrap();
    let sig = git2::Signature::now("User", "user@example.com").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("base.txt")).unwrap();
    let tree_id = index.write_tree().unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .unwrap();

    let untracked = temp.path().join("untracked.txt");
    fs::write(&untracked, "untracked content").unwrap();
    fs::write(&initial_file, "modified content").unwrap();

    let idx_before = repo.index().unwrap();
    assert_eq!(idx_before.len(), 1);
    assert!(idx_before
        .get_path(std::path::Path::new("untracked.txt"), 0)
        .is_none());

    let (_, snap_ref) = CortexSnapshot::create_snapshot(
        &repo,
        "checkpoints",
        "chk_ISOLATED",
        "checkpoint test",
        "Test",
        "test@example.com",
    )
    .unwrap();

    let idx_after = repo.index().unwrap();
    assert_eq!(
        idx_after.len(),
        1,
        "Live index should not have added untracked.txt"
    );
    assert!(idx_after
        .get_path(std::path::Path::new("untracked.txt"), 0)
        .is_none());

    let commit = repo
        .find_reference(&snap_ref)
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let snap_tree = commit.tree().unwrap();
    assert!(snap_tree.get_name("untracked.txt").is_some());
    assert!(snap_tree.get_name("base.txt").is_some());
}

#[test]
fn test_working_tree_dirty_hash_content_sensitive() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();

    let sig = git2::Signature::now("User", "user@example.com").unwrap();
    let file = temp.path().join("tracked.txt");
    fs::write(&file, "initial").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("tracked.txt")).unwrap();
    let tree_id = index.write_tree().unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
    assert_eq!(cortex_repo.working_tree_dirty_hash().unwrap(), "wt:clean");

    // Modify file
    fs::write(&file, "first modification").unwrap();
    let hash1 = cortex_repo.working_tree_dirty_hash().unwrap();
    assert!(hash1.starts_with("wt:sha256:"));

    // Modify file again (status is still WT_MODIFIED, but content differs)
    fs::write(&file, "second modification").unwrap();
    let hash2 = cortex_repo.working_tree_dirty_hash().unwrap();
    assert!(hash2.starts_with("wt:sha256:"));

    assert_ne!(
        hash1, hash2,
        "Content modifications must yield distinct dirty hashes"
    );
}

#[test]
fn test_read_file_for_scope_staged_vs_working_tree() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();

    let sig = git2::Signature::now("User", "user@example.com").unwrap();
    let file = temp.path().join("code.rs");
    fs::write(&file, "pub fn original() {}").unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("code.rs")).unwrap();
    let tree_id = index.write_tree().unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .unwrap();

    // Stage a new version
    fs::write(&file, "pub fn staged_version() {}").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("code.rs")).unwrap();
    index.write().unwrap();

    // Now modify working tree without staging
    fs::write(&file, "pub fn unstaged_working_tree_version() {}").unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();

    // Reading with staged_only=true returns staged version
    let staged_bytes = cortex_repo.read_file_for_scope("code.rs", true).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&staged_bytes),
        "pub fn staged_version() {}"
    );

    // Reading with staged_only=false returns unstaged working tree version
    let worktree_bytes = cortex_repo.read_file_for_scope("code.rs", false).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&worktree_bytes),
        "pub fn unstaged_working_tree_version() {}"
    );
}
