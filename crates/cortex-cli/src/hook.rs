use cortex_capture::reconcile_candidates;
use cortex_core::error::Result;
use cortex_engine::invariant::{verify_invariants, DiffScope};
use cortex_engine::promote::promote_local_records;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo};

pub fn handle_hook(repo: &CortexRepo, db: &mut CortexDb, hook_name: &str) -> Result<i32> {
    match hook_name {
        "pre-commit" => {
            let report = verify_invariants(repo, db, DiffScope::Staged)?;
            if !report.passed {
                eprintln!("\n❌ [git-cortex] Pre-commit Invariant Check Failed!");
                eprintln!(
                    "The staged diff violates {} BLOCKING architectural contract(s):",
                    report.violations.len()
                );
                for v in &report.violations {
                    eprintln!("  • Guarded: {}", v.guarded_symbol);
                    eprintln!("    Drift in: {}", v.upstream_symbol);
                    eprintln!("    Assertion: {}", v.assertion);
                    eprintln!("    Verification Command: {}\n", v.verification_test_cmd);
                }
                eprintln!("To update contracts after intentional refactoring, run: git cortex invariant reaffirm <id>");
                return Ok(1);
            }
            Ok(0)
        }
        "post-commit" => {
            let _ = promote_local_records(repo, db);
            Ok(0)
        }
        "post-checkout" => {
            let _ = reconcile_candidates(repo, db);
            Ok(0)
        }
        "post-merge" => {
            let _ = CortexLedger::reindex_into_db(&repo.repo, db);
            let _ = reconcile_candidates(repo, db);
            Ok(0)
        }
        _ => {
            eprintln!("[git-cortex] Unknown hook: {}", hook_name);
            Ok(0)
        }
    }
}
