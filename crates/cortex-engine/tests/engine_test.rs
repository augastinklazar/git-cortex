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

#[test]
fn test_invariant_guarded_symbol_drift_detection() {
    let (temp, repo, mut db) = setup_repo();

    // Guard Calc::add with NO upstream dependencies
    record_invariant(
        &repo,
        &mut db,
        "src/calc.rs#Calc::add",
        "Calc::add must not be mutated",
        &[],
        "cargo test",
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // Verify passes initially
    let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(report.passed);

    // Mutate Calc::add directly
    let file = temp.path().join("src/calc.rs");
    fs::write(
        &file,
        "pub struct Calc;\nimpl Calc {\n  pub fn add(&self, a: i32, b: i32) -> i32 {\n    a * b\n  }\n}\n",
    )
    .unwrap();

    let report_mutated = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(!report_mutated.passed);
    assert_eq!(report_mutated.violations.len(), 1);
    assert_eq!(
        report_mutated.violations[0].guarded_symbol,
        "src/calc.rs#Calc::add"
    );
    assert!(report_mutated.violations[0]
        .upstream_symbol
        .contains("guarded symbol mutated/deleted"));

    // Delete calc.rs completely
    fs::remove_file(&file).unwrap();
    let report_deleted = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(!report_deleted.passed);
    assert_eq!(report_deleted.violations.len(), 1);
    assert_eq!(
        report_deleted.violations[0].actual_hash,
        "<deleted_or_missing>"
    );
}

#[test]
fn test_handoff_byte_budget_enforcement() {
    let (_temp, repo, mut db) = setup_repo();

    let branch = repo.current_branch().unwrap();
    for i in 0..100 {
        let hyp = ActiveHypothesis {
            hypothesis_id: format!("hyp_{:03}", i),
            statement: format!(
                "Hypothesis number {} with detailed rationale and testing notes",
                i
            ),
            confidence: (i as f64) / 100.0,
            status: "ACTIVE".to_string(),
            next_verification_step: "run test suite".to_string(),
        };
        db.save_hypothesis(&hyp, &branch).unwrap();
    }

    let long_objective = "A".repeat(8192);
    let envelope = create_handoff_envelope(&repo, &db, None, Some(&long_objective)).unwrap();

    let json = serde_json::to_string(&envelope).unwrap();
    assert!(
        json.len() <= MAX_HANDOFF_BYTES,
        "Handoff envelope size {} exceeded max {}",
        json.len(),
        MAX_HANDOFF_BYTES
    );
    assert!(envelope.objective.ends_with("..."));
}

#[test]
fn test_inspect_symbol_deleted_guarded_symbol() {
    let (temp, repo, mut db) = setup_repo();

    let _contract = record_invariant(
        &repo,
        &mut db,
        "src/calc.rs#Calc::add",
        "Calc::add invariant",
        &[],
        "cargo test",
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // Remove file
    let file = temp.path().join("src/calc.rs");
    fs::remove_file(&file).unwrap();

    let res = inspect_symbol(&repo, &db, "src/calc.rs", Some("Calc::add")).unwrap();
    assert_eq!(res.active_invariants.len(), 0);
    assert_eq!(res.stale_invariants.len(), 1);
    assert!(res.stale_invariants[0].1.contains("missing or deleted"));
}

#[test]
fn test_checkpoint_persists_to_sqlite() {
    let (_temp, repo, mut db) = setup_repo();

    let res = checkpoint_state(&repo, &mut db, "checkpoint test objective", &[], &[]).unwrap();

    let stored = db.get_checkpoint(&res.checkpoint_id).unwrap();
    assert!(stored.is_some());
    let chk = stored.unwrap();
    assert_eq!(chk.id, res.checkpoint_id);
    assert_eq!(chk.objective, "checkpoint test objective");
    assert_eq!(chk.snapshot_ref, res.snapshot_ref);
}

#[test]
fn test_symbol_cache_memoization_across_inspections() {
    let (_temp, repo, mut db) = setup_repo();

    let _contract = record_invariant(
        &repo,
        &mut db,
        "src/calc.rs#Calc::add",
        "Add invariant",
        &[],
        "cargo test",
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // record_invariant already populates cache for src/calc.rs
    assert_eq!(repo.symbol_cache.len(), 1);

    // First inspection reuses cached symbols without growing cache
    let res1 = inspect_symbol(&repo, &db, "src/calc.rs", Some("Calc::add")).unwrap();
    assert_eq!(res1.active_invariants.len(), 1);
    assert_eq!(repo.symbol_cache.len(), 1);

    // Second inspection reuses cached symbols
    let res2 = inspect_symbol(&repo, &db, "src/calc.rs", Some("Calc::add")).unwrap();
    assert_eq!(res2.active_invariants.len(), 1);
    assert_eq!(repo.symbol_cache.len(), 1);

    // Invariant verification also hits cached symbols
    let verify_rep = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(verify_rep.passed);
    assert_eq!(repo.symbol_cache.len(), 1);
}

#[test]
fn test_adaptive_handoff_compactor_tier_shedding() {
    let (_temp, repo, mut db) = setup_repo();
    let branch = repo.current_branch().unwrap();

    // 1. Record an active tombstone and a revoked tombstone
    let t1 = record_tombstone(
        &repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/calc.rs#Calc::add".to_string()],
            attempted_approach: "Approach 1",
            failure_reason: "Reason 1",
            test_command: "cargo test",
            error_signature: "sig 1",
            author_harness: "agent",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    let t2 = record_tombstone(
        &repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/calc.rs#Calc::add".to_string()],
            attempted_approach: "Approach 2 (to revoke)",
            failure_reason: "Reason 2",
            test_command: "cargo test",
            error_signature: "sig 2",
            author_harness: "agent",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    revoke_record(&repo, &mut db, &t2.id, "Deprecated", "lead").unwrap();

    // 2. Add multiple hypotheses across confidence tiers
    let hyp_low = ActiveHypothesis {
        hypothesis_id: "hyp_low".to_string(),
        statement: "Low confidence speculation".to_string(),
        confidence: 0.2,
        status: "ACTIVE".to_string(),
        next_verification_step: "run test".to_string(),
    };
    let hyp_med = ActiveHypothesis {
        hypothesis_id: "hyp_med".to_string(),
        statement: "Medium confidence insight".to_string(),
        confidence: 0.65,
        status: "ACTIVE".to_string(),
        next_verification_step: "run test".to_string(),
    };
    let hyp_high = ActiveHypothesis {
        hypothesis_id: "hyp_high".to_string(),
        statement: "High confidence ground truth".to_string(),
        confidence: 0.98,
        status: "ACTIVE".to_string(),
        next_verification_step: "run test".to_string(),
    };

    db.save_hypothesis(&hyp_low, &branch).unwrap();
    db.save_hypothesis(&hyp_med, &branch).unwrap();
    db.save_hypothesis(&hyp_high, &branch).unwrap();

    // 3. Add numerous call graph edges and heavy objective to trigger compaction
    for i in 0..100 {
        let edge = CallGraphEdge {
            caller: format!("src/calc.rs#caller_{}", i),
            callee: format!("src/calc.rs#callee_{}", i),
            relationship: "CALLS".to_string(),
            verified_ast_hash: "ast:sha256:0123456789abcdef".to_string(),
        };
        db.save_call_graph_edge(&edge, &branch).unwrap();
    }

    let long_objective = "A".repeat(2000);
    let envelope =
        create_handoff_envelope(&repo, &db, Some(&branch), Some(&long_objective)).unwrap();

    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.len() <= MAX_HANDOFF_BYTES);

    // Assert revoked tombstone t2 is excluded while t1 remains
    assert!(envelope.active_tombstone_ids.contains(&t1.id));
    assert!(!envelope.active_tombstone_ids.contains(&t2.id));

    // Assert high confidence hypothesis is preserved
    assert!(envelope
        .active_hypotheses
        .iter()
        .any(|h| h.hypothesis_id == "hyp_high"));
}

#[test]
fn test_distributed_ledger_divergence_union_merge() {
    let (_temp, repo, mut db) = setup_repo();

    // Record record A
    let _t1 = record_tombstone(
        &repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/calc.rs#Calc::add".to_string()],
            attempted_approach: "Branch A approach",
            failure_reason: "Branch A failure",
            test_command: "cargo test",
            error_signature: "error A",
            author_harness: "agent-a",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    promote_local_records(&repo, &mut db).unwrap();

    let commit_a = cortex_storage::CortexLedger::current_commit(&repo.repo)
        .unwrap()
        .unwrap();

    // Now simulate remote ledger commit on a separate branch with record B
    // Create new branch pointer for ledger
    let sig = git2::Signature::now("Agent B", "agent-b@example.com").unwrap();
    let tree_b_builder = repo.repo.treebuilder(None).unwrap();
    let blob_b = repo
        .repo
        .blob(br#"{"id":"tmb_01HZXREMOTE","kind":"TOMBSTONE"}"#)
        .unwrap();
    let mut builder = tree_b_builder;
    cortex_storage::CortexLedger::insert_entry_at_path(
        &repo.repo,
        &mut builder,
        "v1/tombstones/TE/tmb_01HZXREMOTE.json",
        blob_b,
    )
    .unwrap();
    let tree_b_oid = builder.write().unwrap();
    let tree_b = repo.repo.find_tree(tree_b_oid).unwrap();
    let commit_b_oid = repo
        .repo
        .commit(
            None,
            &sig,
            &sig,
            "cortex: remote ledger commit",
            &tree_b,
            &[],
        )
        .unwrap();

    // Merge remote commit B into local commit A
    let merge_oid = cortex_storage::CortexLedger::merge_commits(
        &repo.repo,
        commit_b_oid,
        "git-cortex",
        "cortex@git-cortex.dev",
    )
    .unwrap();

    assert!(merge_oid.is_some());
    let merge_commit = repo.repo.find_commit(merge_oid.unwrap()).unwrap();
    assert_eq!(merge_commit.parent_count(), 2);
    assert_eq!(merge_commit.parent_id(0).unwrap(), commit_a.id());
    assert_eq!(merge_commit.parent_id(1).unwrap(), commit_b_oid);

    // Reindex verifies union of both records into DB
    let count = cortex_storage::CortexLedger::reindex_into_db(&repo.repo, &mut db).unwrap();
    assert!(count >= 2);
}
