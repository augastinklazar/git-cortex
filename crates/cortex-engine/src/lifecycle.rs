use cortex_ast::extract_symbols;
use cortex_ast::uri::parse_symbol_uri;
use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::jcs::{canonical_hash, to_canonical_json};
use cortex_core::schema::{EventType, InvariantContract, LifecycleEvent, UpstreamDependency};
use cortex_storage::{CortexDb, CortexRepo};
use std::path::Path;

/// Revoke a Tombstone or Invariant contract by emitting an append-only LifecycleEvent
pub fn revoke_record(
    repo: &CortexRepo,
    db: &mut CortexDb,
    target_id: &str,
    reason: &str,
    author: &str,
) -> Result<LifecycleEvent> {
    let existing = db
        .get_record(target_id)?
        .ok_or_else(|| CortexError::Other(format!("Target record '{}' not found", target_id)))?;

    let event_id = PrefixedId::new(IdPrefix::Event).to_string();
    let branch = repo.current_branch()?;

    let event = LifecycleEvent::new(
        event_id.clone(),
        target_id.to_string(),
        EventType::Revoke,
        reason.to_string(),
        author.to_string(),
    );

    let canonical_json = to_canonical_json(&event)?;
    let content_hash = canonical_hash(&event)?;

    db.save_record(
        &event_id,
        "EVENT",
        &canonical_json,
        "LOCAL",
        false,
        &branch,
        &content_hash,
    )?;

    // Log contextual feedback
    tracing::info!(
        "Revoked {} record '{}' via event '{}'",
        existing.kind,
        target_id,
        event_id
    );

    Ok(event)
}

/// Reaffirm an Invariant contract after intentional code changes by updating hashes and logging a reaffirm event
pub fn reaffirm_invariant(
    repo: &CortexRepo,
    db: &mut CortexDb,
    invariant_id: &str,
    reason: &str,
    author: &str,
) -> Result<InvariantContract> {
    let stored = db.get_record(invariant_id)?.ok_or_else(|| {
        CortexError::Other(format!("Invariant contract '{}' not found", invariant_id))
    })?;

    if stored.kind != "INVARIANT" {
        return Err(CortexError::Other(format!(
            "Record '{}' is a {}, not an INVARIANT",
            invariant_id, stored.kind
        )));
    }

    let mut contract: InvariantContract = serde_json::from_str(&stored.json)
        .map_err(|e| CortexError::Other(format!("Failed to parse invariant JSON: {}", e)))?;

    let branch = repo.current_branch()?;
    let commit_sha = repo.head_commit_sha()?;

    // Recompute guarded symbol hash
    let (guarded_path, guarded_scope) = parse_symbol_uri(&contract.guarded_symbol.symbol_uri)
        .ok_or_else(|| CortexError::Other("Invalid guarded symbol URI".to_string()))?;
    let new_guarded_hash = get_live_symbol_hash(repo, &guarded_path, &guarded_scope)?;

    contract.commit_sha = commit_sha;
    contract.guarded_symbol.ast_hash = new_guarded_hash.clone();

    // Recompute upstream dependencies
    let mut updated_upstreams = Vec::new();
    let mut sym_tuples = Vec::new();

    sym_tuples.push((
        contract.guarded_symbol.symbol_uri.clone(),
        guarded_path,
        new_guarded_hash,
        "GUARDED",
    ));

    for dep in &contract.upstream_dependencies {
        if let Some((dep_path, dep_scope)) = parse_symbol_uri(&dep.symbol_uri) {
            let dep_hash = get_live_symbol_hash(repo, &dep_path, &dep_scope)?;
            updated_upstreams.push(UpstreamDependency {
                symbol_uri: dep.symbol_uri.clone(),
                expected_ast_hash: dep_hash.clone(),
            });
            sym_tuples.push((dep.symbol_uri.clone(), dep_path, dep_hash, "UPSTREAM_DEP"));
        }
    }
    contract.upstream_dependencies = updated_upstreams;

    // Save updated invariant contract
    let canonical_json = to_canonical_json(&contract)?;
    let content_hash = canonical_hash(&contract)?;

    db.save_record(
        invariant_id,
        "INVARIANT",
        &canonical_json,
        "LOCAL",
        false,
        &branch,
        &content_hash,
    )?;

    let borrow_tuples: Vec<(&str, &str, &str, &str)> = sym_tuples
        .iter()
        .map(|(u, p, h, r)| (u.as_str(), p.as_str(), h.as_str(), *r))
        .collect();
    db.index_record_symbols(invariant_id, &borrow_tuples)?;

    // Record reaffirm lifecycle event
    let event_id = PrefixedId::new(IdPrefix::Event).to_string();
    let event = LifecycleEvent::new(
        event_id.clone(),
        invariant_id.to_string(),
        EventType::Reaffirm,
        reason.to_string(),
        author.to_string(),
    );

    let event_canonical_json = to_canonical_json(&event)?;
    let event_content_hash = canonical_hash(&event)?;

    db.save_record(
        &event_id,
        "EVENT",
        &event_canonical_json,
        "LOCAL",
        false,
        &branch,
        &event_content_hash,
    )?;

    Ok(contract)
}

fn get_live_symbol_hash(repo: &CortexRepo, rel_path: &str, scope: &str) -> Result<String> {
    let full = repo.workdir.join(rel_path);
    if !full.exists() {
        return Ok("ast:sha256:0000000000000000".to_string());
    }
    let src = std::fs::read(&full).map_err(CortexError::Io)?;
    let syms = extract_symbols(Path::new(rel_path), &src)?;
    if let Some(s) = syms.iter().find(|s| s.scope_chain == scope) {
        Ok(s.ast_hash.clone())
    } else {
        Ok("ast:sha256:0000000000000000".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invariant::{record_invariant, verify_invariants, DiffScope};
    use crate::tombstone::{record_tombstone, RecordTombstoneParams};
    use cortex_core::schema::{CaptureMode, InvariantSeverity};
    use tempfile::TempDir;

    fn setup_test_repo() -> (TempDir, CortexRepo, CortexDb) {
        let temp = TempDir::new().unwrap();
        let git_repo = git2::Repository::init(temp.path()).unwrap();

        // Create an initial commit with a Rust file
        let src_file = temp.path().join("lib.rs");
        std::fs::write(
            &src_file,
            "pub fn token_guard() -> bool { true }\npub fn auth_check() -> bool { token_guard() }\n",
        )
        .unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(Path::new("lib.rs")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();

        let repo = CortexRepo::discover(Some(temp.path())).unwrap();
        let db = CortexDb::open_in_memory().unwrap();
        (temp, repo, db)
    }

    #[test]
    fn test_invariant_reaffirm_and_revoke_lifecycle() {
        let (_temp, repo, mut db) = setup_test_repo();

        // 1. Record an invariant guarding auth_check with upstream token_guard
        let contract = record_invariant(
            &repo,
            &mut db,
            "lib.rs#auth_check",
            "auth_check must depend on token_guard",
            &["lib.rs#token_guard".to_string()],
            "cargo test",
            InvariantSeverity::Blocking,
        )
        .unwrap();

        // Initially verified
        let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
        assert!(report.passed);

        // 2. Modify upstream token_guard
        let src_file = repo.workdir.join("lib.rs");
        std::fs::write(
            &src_file,
            "pub fn token_guard() -> bool { false /* changed */ }\npub fn auth_check() -> bool { token_guard() }\n",
        )
        .unwrap();

        // Now verification fails due to hash drift
        let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 1);

        // 3. Reaffirm invariant after intentional change
        let reaffirmed = reaffirm_invariant(
            &repo,
            &mut db,
            &contract.invariant_id,
            "Intentional token_guard refactor",
            "developer",
        )
        .unwrap();
        assert_ne!(
            reaffirmed.upstream_dependencies[0].expected_ast_hash,
            contract.upstream_dependencies[0].expected_ast_hash
        );

        // Verification now passes!
        let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
        assert!(report.passed);

        // 4. Test revoke: modify code again to fail, then revoke
        std::fs::write(
            &src_file,
            "pub fn token_guard() -> bool { true /* changed again */ }\npub fn auth_check() -> bool { token_guard() }\n",
        )
        .unwrap();
        let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
        assert!(!report.passed);

        let evt = revoke_record(
            &repo,
            &mut db,
            &contract.invariant_id,
            "Deprecated architectural contract",
            "lead",
        )
        .unwrap();
        assert_eq!(evt.target_id, contract.invariant_id);

        // Revoked invariant is skipped during verification
        let report = verify_invariants(&repo, &db, DiffScope::WorkingTree).unwrap();
        assert!(report.passed);
    }

    #[test]
    fn test_tombstone_revoke() {
        let (_temp, repo, mut db) = setup_test_repo();

        let tombstone = record_tombstone(
            &repo,
            &mut db,
            RecordTombstoneParams {
                target_symbols: &["lib.rs#token_guard".to_string()],
                attempted_approach: "Naive token cache",
                failure_reason: "Cache desync",
                test_command: "cargo test",
                error_signature: "FAIL",
                author_harness: "test",
                capture_mode: CaptureMode::Manual,
            },
        )
        .unwrap();

        let inspect_before = crate::inspect_symbol(&repo, &db, "lib.rs", None).unwrap();
        assert_eq!(inspect_before.active_tombstones.len(), 1);

        // Revoke tombstone
        let evt = revoke_record(
            &repo,
            &mut db,
            &tombstone.id,
            "Approach is now viable in v2",
            "architect",
        )
        .unwrap();
        assert_eq!(evt.target_id, tombstone.id);

        let inspect_after = crate::inspect_symbol(&repo, &db, "lib.rs", None).unwrap();
        assert_eq!(inspect_after.active_tombstones.len(), 0);
        assert!(inspect_after
            .formatted_markdown
            .contains("<!-- REVOKED TOMBSTONE:"));
    }
}
