use cortex_capture::execute_command;
use cortex_core::schema::{ActiveHypothesis, CallGraphEdge, CaptureMode, InvariantSeverity};
use cortex_engine::checkpoint::checkpoint_state;
use cortex_engine::handoff::{
    create_handoff_envelope, format_a2a_task_envelope, import_handoff_envelope, MAX_HANDOFF_BYTES,
};
use cortex_engine::inspect::inspect_symbol;
use cortex_engine::invariant::{record_invariant, verify_invariants, DiffScope};
use cortex_engine::lifecycle::reaffirm_invariant;
use cortex_engine::rewind::rewind_to_checkpoint;
use cortex_engine::tombstone::{record_tombstone, RecordTombstoneParams};
use cortex_mcp::McpServer;
use cortex_storage::{CortexDb, CortexRepo};
use git2::Repository;
use serde_json::json;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

fn setup_git_repo() -> (TempDir, Repository, CortexRepo, CortexDb) {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    // Create an initial file to anchor HEAD
    let init_file = temp.path().join("README.md");
    fs::write(&init_file, "# Cortex E2E Test\n").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    {
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Initial anchor commit",
            &tree,
            &[],
        )
        .unwrap();
    }

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
    let db = CortexDb::open_in_memory().unwrap();

    (temp, repo, cortex_repo, db)
}

/// Scenario 1: Passive Capture Edit -> Fail -> Revert Candidate to Promoted Tombstone
#[test]
fn scenario_1_passive_capture_edit_fail_revert_promoted_tombstone() {
    let (temp, repo, cortex_repo, mut db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("algo.rs");
    let original_code =
        "pub fn search(v: &[i32], target: i32) -> bool {\n    v.contains(&target)\n}\n";
    fs::write(&file, original_code).unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Add algo", &tree, &[&parent])
        .unwrap();

    // 1. Edit code (introduce a defect)
    fs::write(
        &file,
        "pub fn search(v: &[i32], target: i32) -> bool {\n    false /* defect */\n}\n",
    )
    .unwrap();

    // 2. Fail: simulate failing test execution
    let failing_cmd = vec![
        "python".to_string(),
        "-c".to_string(),
        "import sys; print('AssertionError: expected true but got false'); sys.exit(1)".to_string(),
    ];
    let exit_code = execute_command(&cortex_repo, &mut db, &failing_cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // Verify candidate created in FAILED state
    let failed_candidates = db.get_candidates_by_state("FAILED").unwrap();
    assert_eq!(failed_candidates.len(), 1);
    assert!(failed_candidates[0].symbols_json.contains("search"));

    // 3. Revert code back to original working state
    fs::write(&file, original_code).unwrap();

    // 4. Succeed: run passing test
    let passing_cmd = vec![
        "python".to_string(),
        "-c".to_string(),
        "import sys; print('SUCCESS: all tests passed'); sys.exit(0)".to_string(),
    ];
    let exit_code = execute_command(&cortex_repo, &mut db, &passing_cmd, None).unwrap();
    assert_eq!(exit_code, 0);

    // Verify candidate promoted and tombstone synthesized
    let pending_candidates = db.get_candidates_by_state("FAILED").unwrap();
    assert_eq!(pending_candidates.len(), 0);

    let tombstones = db.list_records_by_kind("TOMBSTONE").unwrap();
    assert_eq!(tombstones.len(), 1);
    assert!(tombstones[0].json.contains("AssertionError"));
    assert!(tombstones[0].json.contains("algo.rs"));
}

/// Scenario 2: Baseline Failure Suppression
#[test]
fn scenario_2_baseline_failure_suppression() {
    let (temp, repo, cortex_repo, mut db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("worker.rs");
    let original_code = "pub fn work() -> bool {\n    true\n}\n";
    fs::write(&file, original_code).unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Add worker", &tree, &[&parent])
        .unwrap();

    let failing_cmd = vec![
        "python".to_string(),
        "-c".to_string(),
        "import sys; print('BaselineFailure: pre-existing broken test'); sys.exit(1)".to_string(),
    ];

    // 1. Run failing command while tree is clean (baseline)
    let exit_code = execute_command(&cortex_repo, &mut db, &failing_cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // 2. Modify code
    fs::write(
        &file,
        "pub fn work() -> bool {\n    false /* modified */\n}\n",
    )
    .unwrap();

    // 3. Run failing command again
    let exit_code = execute_command(&cortex_repo, &mut db, &failing_cmd, None).unwrap();
    assert_eq!(exit_code, 1);

    // 4. Verify baseline failure was suppressed - zero candidates created!
    let candidates = db.get_candidates_by_state("FAILED").unwrap();
    assert_eq!(candidates.len(), 0);
}

/// Scenario 3: Causal Invariant Upstream Drift Blocking and Reaffirmation
#[test]
fn scenario_3_causal_invariant_drift_and_reaffirmation() {
    let (temp, repo, cortex_repo, mut db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let api_file = src_dir.join("api.rs");
    let model_file = src_dir.join("model.rs");

    fs::write(&api_file, "pub fn endpoint() -> bool {\n    true\n}\n").unwrap();
    fs::write(
        &model_file,
        "pub struct Config {\n    pub timeout: u32,\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Add api and model",
        &tree,
        &[&parent],
    )
    .unwrap();

    // 1. Record invariant contract: api endpoint depends on Config model
    let inv = record_invariant(
        &cortex_repo,
        &mut db,
        "src/api.rs#endpoint",
        "api endpoint assumes stable Config model structure",
        &["src/model.rs#Config".to_string()],
        "cargo test",
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // 2. Initial verification passes
    let report = verify_invariants(&cortex_repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(report.passed);

    // 3. Mutate upstream dependency (Config struct layout)
    fs::write(
        &model_file,
        "pub struct Config {\n    pub timeout: u32,\n    pub retries: u32,\n}\n",
    )
    .unwrap();

    // 4. Invariant verification fails (drift detected!)
    let report_violated = verify_invariants(&cortex_repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(!report_violated.passed);
    assert_eq!(report_violated.violations.len(), 1);
    assert_eq!(
        report_violated.violations[0].guarded_symbol,
        "src/api.rs#endpoint"
    );

    // 5. Reaffirm the invariant with new AST hash
    let reaffirmed = reaffirm_invariant(
        &cortex_repo,
        &mut db,
        &inv.invariant_id,
        "Updated config layout",
        "e2e-tester",
    )
    .unwrap();
    assert_eq!(reaffirmed.invariant_id, inv.invariant_id);

    // 6. Invariant verification now passes
    let report_ok = verify_invariants(&cortex_repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(report_ok.passed);
    assert_eq!(report_ok.violations.len(), 0);
}

/// Scenario 4: Checkpoint Snapshot and Rewind with Synthesized Tombstone
#[test]
fn scenario_4_checkpoint_rewind_restoration_and_immunity_tombstone() {
    let (temp, repo, cortex_repo, mut db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("service.rs");
    let original_content = "pub fn execute() -> i32 {\n    42\n}\n";
    fs::write(&file, original_content).unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Add service", &tree, &[&parent])
        .unwrap();

    // 1. Create a checkpoint snapshot
    let chk = checkpoint_state(&cortex_repo, &mut db, "Stable baseline", &[], &[]).unwrap();

    // 2. Introduce destructive changes in working tree
    fs::write(
        &file,
        "pub fn execute() -> i32 {\n    panic!(\"broken experiment\");\n}\n",
    )
    .unwrap();

    // 3. Rewind back to checkpoint
    let rewind_res = rewind_to_checkpoint(
        &cortex_repo,
        &mut db,
        &chk.checkpoint_id,
        Some("Aborting failed experiment"),
    )
    .unwrap();
    assert_eq!(rewind_res.target_checkpoint_id, chk.checkpoint_id);

    // 4. Verify working tree content restored
    let current_content = fs::read_to_string(&file).unwrap();
    assert_eq!(
        current_content.replace("\r\n", "\n"),
        original_content.replace("\r\n", "\n")
    );

    // 5. Verify synthesized immunity tombstone captures reverted symbol
    let inspect = inspect_symbol(&cortex_repo, &db, "src/service.rs", Some("execute")).unwrap();
    assert_eq!(inspect.active_tombstones.len(), 1);
    assert_eq!(
        inspect.active_tombstones[0].capture_mode,
        CaptureMode::RewindSynthesized
    );
    assert!(inspect
        .formatted_markdown
        .contains("reverted via cortex rewind"));
    assert!(inspect
        .formatted_markdown
        .contains("Aborting failed experiment"));
}

/// Scenario 5: Epistemic Handoff Export/Import Envelope Verification (RFC 8785, <= 4096 bytes)
#[test]
fn scenario_5_epistemic_handoff_export_import_rfc8785() {
    let (_temp, _repo, cortex_repo, mut db) = setup_git_repo();

    // Record a tombstone
    let _tmb = record_tombstone(
        &cortex_repo,
        &mut db,
        RecordTombstoneParams {
            target_symbols: &["src/auth.rs#verify_token".to_string()],
            attempted_approach: "Stateless JWT without salt",
            failure_reason: "Replay attack vulnerability",
            test_command: "cargo test",
            error_signature: "TokenReplayDetected",
            author_harness: "e2e-suite",
            capture_mode: CaptureMode::Manual,
        },
    )
    .unwrap();

    // Create a checkpoint with hypothesis and call graph
    let _chk = checkpoint_state(
        &cortex_repo,
        &mut db,
        "Handoff validation",
        &[CallGraphEdge {
            caller: "src/server.rs#handle".into(),
            callee: "src/auth.rs#verify_token".into(),
            relationship: "CALLS".into(),
            verified_ast_hash: "ast:sha256:abcdef0123456789".into(),
        }],
        &[ActiveHypothesis {
            hypothesis_id: "hyp_session_cache".into(),
            statement: "In-memory LRU cache prevents replay attacks".into(),
            confidence: 0.90,
            status: "IN_PROGRESS".into(),
            next_verification_step: "cargo test --test auth_replay".into(),
        }],
    )
    .unwrap();

    // 1. Export handoff envelope
    let envelope =
        create_handoff_envelope(&cortex_repo, &db, None, Some("Sub-agent delegation")).unwrap();

    // 2. Verify <= 4096 bytes budget
    let envelope_json = serde_json::to_string(&envelope).unwrap();
    assert!(
        envelope_json.len() <= MAX_HANDOFF_BYTES,
        "Envelope size {} exceeds max {}",
        envelope_json.len(),
        MAX_HANDOFF_BYTES
    );

    // 3. Verify RFC 8785 canonical hash computation
    let canonical_hash = cortex_core::jcs::canonical_hash(&envelope).unwrap();
    assert!(canonical_hash.starts_with("sha256:"));
    assert_eq!(canonical_hash.len(), 71); // "sha256:" (7) + 64 hex = 71

    // 4. Verify Agent2Agent (A2A) task extension packaging
    let a2a =
        format_a2a_task_envelope(&envelope, "task_handoff_01", "Continue auth hardening").unwrap();
    assert_eq!(a2a["method"], "tasks/send");
    assert_eq!(a2a["params"]["id"], "task_handoff_01");
    let ext = &a2a["params"]["metadata"]["https://git-cortex.dev/a2a-extension/v0.1"];
    assert_eq!(ext["handoff_id"], envelope.handoff_id);

    // 5. Import into an isolated fresh database (sub-agent context)
    let mut subagent_db = CortexDb::open_in_memory().unwrap();
    import_handoff_envelope(&cortex_repo, &mut subagent_db, &envelope, "main").unwrap();

    let imported_edges = subagent_db.get_call_graph_edges("main").unwrap();
    assert_eq!(imported_edges.len(), 1);
    assert_eq!(imported_edges[0].callee, "src/auth.rs#verify_token");

    let imported_hyp = subagent_db.get_hypotheses("main").unwrap();
    assert_eq!(imported_hyp.len(), 1);
    assert_eq!(imported_hyp[0].hypothesis_id, "hyp_session_cache");
}

/// Scenario 6: MCP Stdio JSON-RPC Session Protocol (initialize -> tools/list -> tools/call)
#[test]
fn scenario_6_mcp_stdio_jsonrpc_session() {
    let (temp, repo, cortex_repo, db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("crypto.rs");
    fs::write(
        &file,
        "pub struct Encryptor;\nimpl Encryptor {\n    pub fn encrypt(&self) -> bool { true }\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Add crypto", &tree, &[&parent])
        .unwrap();

    let mut server = McpServer::new(cortex_repo, db);

    let requests = vec![
        // 1. Initialize
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "e2e-agent",
                    "version": "1.0.0"
                }
            }
        }),
        // 2. Tools list
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        // 3. Record tombstone via tools/call
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "cortex_record_tombstone",
                "arguments": {
                    "target_symbols": ["src/crypto.rs#Encryptor::encrypt"],
                    "attempted_approach": "Used ECB mode for AES encryption",
                    "failure_reason": "Pattern leakage in encrypted blocks",
                    "test_command": "cargo test",
                    "error_signature": "SecurityViolation: ECB mode not permitted"
                }
            }
        }),
        // 4. Inspect symbol via tools/call
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "cortex_inspect_symbol",
                "arguments": {
                    "filepath": "src/crypto.rs",
                    "symbol_name": "Encryptor::encrypt"
                }
            }
        }),
    ];

    let mut input_bytes = Vec::new();
    for req in requests {
        input_bytes.extend_from_slice(serde_json::to_string(&req).unwrap().as_bytes());
        input_bytes.push(b'\n');
    }

    let mut output_bytes = Vec::new();
    server.run_stdio(Cursor::new(input_bytes), &mut output_bytes);

    let output_str = String::from_utf8(output_bytes).unwrap();
    let lines: Vec<&str> = output_str.lines().collect();
    assert_eq!(lines.len(), 4);

    // Verify 1: Initialize response
    let resp1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(resp1["id"], 1);
    assert_eq!(resp1["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(resp1["result"]["serverInfo"]["name"], "git-cortex");

    // Verify 2: Tools list response
    let resp2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(resp2["id"], 2);
    let tools = resp2["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "cortex_inspect_symbol"));
    assert!(tools.iter().any(|t| t["name"] == "cortex_record_tombstone"));
    assert!(tools
        .iter()
        .any(|t| t["name"] == "cortex_verify_invariants"));

    // Verify 3: Record tombstone response
    let resp3: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(resp3["id"], 3);
    let tmb_output = resp3["result"]["content"][0]["text"].as_str().unwrap();
    assert!(tmb_output.contains("tmb_"));

    // Verify 4: Inspect symbol returns newly recorded tombstone
    let resp4: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(resp4["id"], 4);
    let inspect_text = resp4["result"]["content"][0]["text"].as_str().unwrap();
    assert!(inspect_text.contains("ECB mode not permitted"));
    assert!(inspect_text.contains("Pattern leakage in encrypted blocks"));
}

/// Scenario 7: Malicious verification_test_cmd Advisory Safety
#[test]
fn scenario_7_malicious_verification_cmd_advisory_safety() {
    let (temp, repo, cortex_repo, mut db) = setup_git_repo();
    let sig = git2::Signature::now("Test Runner", "tester@git-cortex.dev").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("core.rs");
    fs::write(&file, "pub fn compute() -> i32 { 10 }\n").unwrap();

    // Critical canary file that must NOT be deleted
    let canary_file = temp.path().join("CANARY_CRITICAL_FILE.txt");
    fs::write(&canary_file, "SENSITIVE SYSTEM ASSET").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Add core", &tree, &[&parent])
        .unwrap();

    // Record an invariant with a hostile verification command designed to delete the canary file
    let hostile_cmd = if cfg!(windows) {
        "powershell -Command Remove-Item -Force CANARY_CRITICAL_FILE.txt"
    } else {
        "rm -f CANARY_CRITICAL_FILE.txt"
    };

    let inv = record_invariant(
        &cortex_repo,
        &mut db,
        "src/core.rs#compute",
        "Assumes compute invariant with hostile test cmd",
        &["src/core.rs#compute".to_string()],
        hostile_cmd,
        InvariantSeverity::Blocking,
    )
    .unwrap();

    // Run invariant verification against working tree
    let report = verify_invariants(&cortex_repo, &db, DiffScope::WorkingTree).unwrap();
    assert!(report.passed);

    // Verify the canary file still exists untouched! Invariant verification never executes shell commands.
    assert!(
        canary_file.exists(),
        "CRITICAL SECURITY FAILURE: Hostile verification_test_cmd was executed by verify_invariants!"
    );

    // Invariant stores the command purely as passive advisory metadata
    let stored = db.get_record(&inv.invariant_id).unwrap().unwrap();
    assert!(stored.json.contains(hostile_cmd));
}
