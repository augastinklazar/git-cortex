use cortex_ast::uri::{normalize_rel_path, parse_symbol_uri};
use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::jcs::{canonical_hash, to_canonical_json};
use cortex_core::schema::{InvariantContract, InvariantSeverity, TargetSymbol, UpstreamDependency};
use cortex_storage::{CortexDb, CortexRepo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiffScope {
    Staged,
    WorkingTree,
    Branch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantViolation {
    pub invariant_id: String,
    pub guarded_symbol: String,
    pub upstream_symbol: String,
    pub expected_hash: String,
    pub actual_hash: String,
    pub severity: InvariantSeverity,
    pub assertion: String,
    pub verification_test_cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantVerificationReport {
    pub scope: DiffScope,
    pub checked_count: usize,
    pub passed: bool,
    pub violations: Vec<InvariantViolation>,
    pub warnings: Vec<InvariantViolation>,
}

/// Create and store an InvariantContract in SQLite state.db
pub fn record_invariant(
    repo: &CortexRepo,
    db: &mut CortexDb,
    guarded_symbol_uri: &str,
    assertion: &str,
    upstream_dependencies: &[String],
    verification_test_cmd: &str,
    severity: InvariantSeverity,
) -> Result<InvariantContract> {
    let id = PrefixedId::new(IdPrefix::Invariant).to_string();
    let branch = repo.current_branch()?;
    let commit_sha = repo.head_commit_sha()?;

    let (guarded_path, guarded_scope) = parse_symbol_uri(guarded_symbol_uri)
        .ok_or_else(|| CortexError::Other("Invalid guarded symbol URI format".to_string()))?;

    let guarded_hash = get_symbol_hash(repo, &guarded_path, &guarded_scope)?;
    let guarded = TargetSymbol {
        symbol_uri: guarded_symbol_uri.to_string(),
        ast_hash: guarded_hash.clone(),
    };

    let mut upstreams = Vec::new();
    let mut sym_tuples = Vec::new();

    sym_tuples.push((
        guarded_symbol_uri.to_string(),
        guarded_path,
        guarded_hash,
        "GUARDED",
    ));

    for dep_uri in upstream_dependencies {
        if let Some((dep_path, dep_scope)) = parse_symbol_uri(dep_uri) {
            let dep_hash = get_symbol_hash(repo, &dep_path, &dep_scope)?;
            upstreams.push(UpstreamDependency {
                symbol_uri: dep_uri.clone(),
                expected_ast_hash: dep_hash.clone(),
            });
            sym_tuples.push((dep_uri.clone(), dep_path, dep_hash, "UPSTREAM_DEP"));
        }
    }

    let contract = InvariantContract::new(
        id.clone(),
        commit_sha,
        guarded,
        severity,
        assertion.to_string(),
        upstreams,
        verification_test_cmd.to_string(),
    );

    let canonical_json = to_canonical_json(&contract)?;
    let content_hash = canonical_hash(&contract)?;

    db.save_record(
        &id,
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
    db.index_record_symbols(&id, &borrow_tuples)?;

    Ok(contract)
}

/// Verify active invariant contracts against specified DiffScope.
/// Verifies both guarded_symbol itself (catching mutations/deletions) and all upstream dependencies.
pub fn verify_invariants(
    repo: &CortexRepo,
    db: &CortexDb,
    scope: DiffScope,
) -> Result<InvariantVerificationReport> {
    let records = db.list_records_by_kind("INVARIANT")?;
    let events = db.list_records_by_kind("EVENT").unwrap_or_default();
    let mut revoked_invariants = std::collections::HashSet::new();
    for evt_rec in events {
        if let Ok(evt) = serde_json::from_str::<cortex_core::schema::LifecycleEvent>(&evt_rec.json)
        {
            if evt.event_type == cortex_core::schema::EventType::Revoke {
                revoked_invariants.insert(evt.target_id);
            }
        }
    }

    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let mut file_cache: HashMap<String, Vec<cortex_ast::Symbol>> = HashMap::new();

    for stored in &records {
        if let Ok(contract) = serde_json::from_str::<InvariantContract>(&stored.json) {
            if revoked_invariants.contains(&contract.invariant_id) {
                continue;
            }

            // Check both guarded symbol itself AND all upstream dependencies
            let mut check_targets: Vec<(&str, &str, bool)> =
                Vec::with_capacity(1 + contract.upstream_dependencies.len());
            check_targets.push((
                &contract.guarded_symbol.symbol_uri,
                &contract.guarded_symbol.ast_hash,
                true,
            ));
            for dep in &contract.upstream_dependencies {
                check_targets.push((&dep.symbol_uri, &dep.expected_ast_hash, false));
            }

            for (target_uri, expected_hash, is_guarded_self) in check_targets {
                let Some((dep_path, dep_scope)) = parse_symbol_uri(target_uri) else {
                    continue;
                };
                let norm_path = normalize_rel_path(Path::new(&dep_path));
                let symbols = if let Some(s) = file_cache.get(&norm_path) {
                    s.clone()
                } else {
                    let syms = read_symbols_by_scope(repo, &norm_path, scope)?;
                    file_cache.insert(norm_path.clone(), syms.clone());
                    syms
                };

                let current_hash = symbols
                    .iter()
                    .find(|s| s.scope_chain == dep_scope || s.uri == target_uri)
                    .map(|s| s.ast_hash.clone());

                let drifted = match &current_hash {
                    Some(h) => h != expected_hash,
                    None => true, // Symbol or file was deleted
                };

                if drifted {
                    let actual_hash =
                        current_hash.unwrap_or_else(|| "<deleted_or_missing>".to_string());
                    let v = InvariantViolation {
                        invariant_id: contract.invariant_id.clone(),
                        guarded_symbol: contract.guarded_symbol.symbol_uri.clone(),
                        upstream_symbol: if is_guarded_self {
                            format!("{} (guarded symbol mutated/deleted)", target_uri)
                        } else {
                            target_uri.to_string()
                        },
                        expected_hash: expected_hash.to_string(),
                        actual_hash,
                        severity: contract.severity,
                        assertion: contract.assertion.clone(),
                        verification_test_cmd: contract.verification_test_cmd.clone(),
                    };

                    if contract.severity == InvariantSeverity::Blocking {
                        violations.push(v);
                    } else {
                        warnings.push(v);
                    }
                }
            }
        }
    }

    let passed = violations.is_empty();

    Ok(InvariantVerificationReport {
        scope,
        checked_count: records.len(),
        passed,
        violations,
        warnings,
    })
}

fn get_symbol_hash(repo: &CortexRepo, rel_path: &str, scope: &str) -> Result<String> {
    let full = repo.workdir.join(rel_path);
    if !full.exists() {
        return Ok("ast:sha256:0000000000000000".to_string());
    }
    let src = std::fs::read(&full).map_err(CortexError::Io)?;
    let syms = repo.extract_symbols_cached(Path::new(rel_path), &src)?;
    if let Some(s) = syms.iter().find(|s| s.scope_chain == scope) {
        Ok(s.ast_hash.clone())
    } else {
        Ok("ast:sha256:0000000000000000".to_string())
    }
}

fn read_symbols_by_scope(
    repo: &CortexRepo,
    rel_path: &str,
    scope: DiffScope,
) -> Result<Vec<cortex_ast::Symbol>> {
    match scope {
        DiffScope::WorkingTree => {
            let full = repo.workdir.join(rel_path);
            if full.exists() {
                let src = std::fs::read(&full).map_err(CortexError::Io)?;
                repo.extract_symbols_cached(Path::new(rel_path), &src)
            } else {
                Ok(Vec::new())
            }
        }
        DiffScope::Staged => {
            let index = repo
                .repo
                .index()
                .map_err(|e| CortexError::Git(e.to_string()))?;
            if let Some(entry) = index.get_path(Path::new(rel_path), 0) {
                let blob = repo
                    .repo
                    .find_blob(entry.id)
                    .map_err(|e| CortexError::Git(e.to_string()))?;
                repo.extract_symbols_cached(Path::new(rel_path), blob.content())
            } else if let Ok(head_tree) = repo.repo.head().and_then(|h| h.peel_to_tree()) {
                if let Ok(tree_entry) = head_tree.get_path(Path::new(rel_path)) {
                    let blob = repo
                        .repo
                        .find_blob(tree_entry.id())
                        .map_err(|e| CortexError::Git(e.to_string()))?;
                    repo.extract_symbols_cached(Path::new(rel_path), blob.content())
                } else {
                    Ok(Vec::new())
                }
            } else {
                read_symbols_by_scope(repo, rel_path, DiffScope::WorkingTree)
            }
        }
        DiffScope::Branch => {
            if let Ok(head_tree) = repo.repo.head().and_then(|h| h.peel_to_tree()) {
                if let Ok(tree_entry) = head_tree.get_path(Path::new(rel_path)) {
                    let blob = repo
                        .repo
                        .find_blob(tree_entry.id())
                        .map_err(|e| CortexError::Git(e.to_string()))?;
                    repo.extract_symbols_cached(Path::new(rel_path), blob.content())
                } else {
                    Ok(Vec::new())
                }
            } else {
                read_symbols_by_scope(repo, rel_path, DiffScope::WorkingTree)
            }
        }
    }
}
