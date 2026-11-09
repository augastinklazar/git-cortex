use cortex_core::schema::{ActiveHypothesis, CallGraphEdge, CaptureMode, InvariantSeverity};
use cortex_engine::*;
use cortex_storage::{CortexDb, CortexRepo};
use git2::Repository;
use std::fs;
use tempfile::TempDir;

fn setup_repo() -> (TempDir, CortexRepo, CortexDb) {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("calc.rs");
    fs::write(
        &file,
        "pub struct Calc;\nimpl Calc {\n  pub fn add(&self, a: i32, b: i32) -> i32 {\n    a + b\n  }\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
    let db = CortexDb::open_in_memory().unwrap();

    (temp, cortex_repo, db)
}

#[test]
fn test_inspect_and_tombstone() {
    let (_temp, repo, mut db) = setup_repo();

    let record = record_tombstone(
        &repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/calc.rs#Calc::add".to_string()],
            attempted_approach: "Tried bitwise addition",
            failure_reason: "Overflow panic on negative integers",
            test_command: "cargo test",
            error_signature: "attempt to add with overflow",
            author_harness: "test-harness",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    assert!(record.id.starts_with("tmb_"));

    let inspect = inspect_symbol(&repo, &db, "src/calc.rs", Some("Calc::add")).unwrap();
    assert_eq!(inspect.active_tombstones.len(), 1);
    assert!(inspect
        .formatted_markdown
        .contains("Tried bitwise addition"));
    assert!(inspect.formatted_markdown.contains("Overflow panic"));
}

#[test]
fn test_invariant_violation_detection() {
    let (temp, repo, mut db) = setup_repo();

    // Guard Calc::add, upstream dep on calc.rs#Calc
    record_invariant(
        &repo,
        &mut db,
        "src/calc.rs#Calc::add",
        "Calc::add requires Calc struct layout unchanged",
        &["src/calc.rs#Calc".to_string()],
        "cargo test",
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // Verification before change -> passes
    let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(report.passed);

    // Now mutate upstream dependency (Calc struct)
    let file = temp.path().join("src/calc.rs");
    fs::write(
        &file,
        "pub struct Calc { count: usize }\nimpl Calc {\n  pub fn add(&self, a: i32, b: i32) -> i32 {\n    a + b\n  }\n}\n",
    )
    .unwrap();

    let report2 = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(!report2.passed);
    assert_eq!(report2.violations.len(), 1);
    assert_eq!(
        report2.violations[0].guarded_symbol,
        "src/calc.rs#Calc::add"
    );
}

#[test]
fn test_handoff_and_budget() {
    let (_temp, repo, mut db) = setup_repo();

    let chk = checkpoint_state(
        &repo,
        &mut db,
        "Refactoring Calc for high throughput",
        &[CallGraphEdge {
            caller: "src/main.rs#main".into(),
            callee: "src/calc.rs#Calc::add".into(),
            relationship: "INVOKES".into(),
            verified_ast_hash: "ast:sha256:1234567812345678".into(),
        }],
        &[ActiveHypothesis {
            hypothesis_id: "hyp_01".into(),
            statement: "SIMD instruction acceleration".into(),
            confidence: 0.95,
            status: "IN_PROGRESS".into(),
            next_verification_step: "cargo bench".into(),
        }],
    )
    .unwrap();

    assert!(chk.checkpoint_id.starts_with("chk_"));

    let envelope = create_handoff_envelope(&repo, &db, None, Some(&chk.objective)).unwrap();
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.len() <= MAX_HANDOFF_BYTES);
    assert_eq!(envelope.active_hypotheses.len(), 1);

    let a2a = format_a2a_task_envelope(&envelope, "task_1", "Optimize add").unwrap();
    assert!(a2a
        .to_string()
        .contains("https://git-cortex.dev/a2a-extension/v0.1"));
}

#[test]
fn test_promote_includes_lifecycle_events() {
    let (_temp, repo, mut db) = setup_repo();

    let tombstone = record_tombstone(
        &repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/calc.rs#Calc::add".to_string()],
            attempted_approach: "Naive recursion",
            failure_reason: "Stack overflow",
            test_command: "cargo test",
            error_signature: "fatal runtime error: stack overflow",
            author_harness: "test",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    let revoke_evt = cortex_engine::lifecycle::revoke_record(
        &repo,
        &mut db,
        &tombstone.id,
        "Stack size was increased in runtime config",
        "tester",
    )
    .unwrap();

    let promoted_count = promote_local_records(&repo, &mut db).unwrap();
    assert_eq!(promoted_count, 2);

    let commit = cortex_storage::CortexLedger::current_commit(&repo.repo)
        .unwrap()
        .unwrap();
    let tree = commit.tree().unwrap();
    let mut found_event = false;
    let event_filename = format!("{}.json", revoke_evt.event_id);
    let _ = tree.walk(git2::TreeWalkMode::PreOrder, |_root, entry| {
        if let Some(name) = entry.name() {
            if name == event_filename {
                found_event = true;
            }
        }
        git2::TreeWalkResult::Ok
    });
    assert!(
        found_event,
        "Revocation event record was not found in ledger tree!"
    );
}
