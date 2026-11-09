use cortex_capture::*;
use cortex_storage::{CortexDb, CortexRepo};
use git2::Repository;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_passive_capture_edit_fail_revert_cycle() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("worker.rs");
    let original_code = "pub fn work() -> bool {\n  true\n}\n";
    fs::write(&file, original_code).unwrap();

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
    let mut db = CortexDb::open_in_memory().unwrap();

    // 1. Edit code (dirty mutation)
    fs::write(&file, "pub fn work() -> bool {\n  false /* broken */\n}\n").unwrap();

    // 2. Fail: simulate test failure using a command that exits with 1
    // We can run powershell / cmd exiting with 1 or python -c "import sys; print('AssertionError: failed'); sys.exit(1)"
    let cmd = vec![
        "python".to_string(),
        "-c".to_string(),
        "import sys; print('AssertionError: expected true but got false'); sys.exit(1)".to_string(),
    ];

    let exit_code = execute_command(&cortex_repo, &mut db, &cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // Verify candidate failure was recorded
    let candidates = db.get_candidates_by_state("FAILED").unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].symbols_json.contains("worker.rs#work"));

    // 3. Revert: restore file to original code
    fs::write(&file, original_code).unwrap();

    // 4. Reconcile: Passive capture automatically detects revert and promotes candidate!
    let promoted = reconcile_candidates(&cortex_repo, &mut db).unwrap();
    assert_eq!(promoted.len(), 1);
    assert_eq!(
        promoted[0].capture_mode,
        cortex_core::schema::CaptureMode::AutoRevertDetected
    );
    assert!(promoted[0].failure_reason.contains("AssertionError"));

    // Candidate should now be marked PROMOTED
    let pending = db.get_candidates_by_state("FAILED").unwrap();
    assert!(pending.is_empty());
}

#[test]
fn test_baseline_failure_does_not_create_false_tombstone() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("worker.rs");
    let original_code = "pub fn work() -> bool {\n  true\n}\n";
    fs::write(&file, original_code).unwrap();

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
    let mut db = CortexDb::open_in_memory().unwrap();

    let failing_cmd = vec![
        "python".to_string(),
        "-c".to_string(),
        "import sys; print('BaselineFailure: suite broken'); sys.exit(1)".to_string(),
    ];

    // 1. Run failing command while working tree is clean (baseline failure)
    let exit_code = execute_command(&cortex_repo, &mut db, &failing_cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // 2. Now modify code
    fs::write(&file, "pub fn work() -> bool {\n  false\n}\n").unwrap();

    // 3. Run failing command again with modified code
    let exit_code = execute_command(&cortex_repo, &mut db, &failing_cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // 4. Verify that NO candidate was created because the failure was already happening at baseline!
    let candidates = db.get_candidates_by_state("FAILED").unwrap();
    assert_eq!(candidates.len(), 0);
}
