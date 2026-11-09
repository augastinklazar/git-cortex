use cortex_ast::extract_symbols;
use cortex_ast::uri::parse_symbol_uri;
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

pub fn verify_invariants(
    repo: &CortexRepo,
    db: &CortexDb,
    scope: DiffScope,
) -> Result<InvariantVerificationReport> {
    let records = db.list_records_by_kind("INVARIANT")?;
    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let mut file_cache: HashMap<String, Vec<cortex_ast::Symbol>> = HashMap::new();

    for stored in &records {
        if let Ok(contract) = serde_json::from_str::<InvariantContract>(&stored.json) {
            for dep in &contract.upstream_dependencies {
                if let Some((dep_path, dep_scope)) = parse_symbol_uri(&dep.symbol_uri) {
                    let symbols = if let Some(s) = file_cache.get(&dep_path) {
                        s.clone()
                    } else {
                        let syms = read_symbols_by_scope(repo, &dep_path, scope)?;
                        file_cache.insert(dep_path.clone(), syms.clone());
                        syms
                    };

                    let current_hash = symbols
                        .iter()
                        .find(|s| s.scope_chain == dep_scope)
                        .map(|s| s.ast_hash.clone())
                        .unwrap_or_else(|| "ast:sha256:MISSING".to_string());

                    if current_hash != dep.expected_ast_hash {
                        let v = InvariantViolation {
                            invariant_id: contract.invariant_id.clone(),
                            guarded_symbol: contract.guarded_symbol.symbol_uri.clone(),
                            upstream_symbol: dep.symbol_uri.clone(),
                            expected_hash: dep.expected_ast_hash.clone(),
                            actual_hash: current_hash,
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
    let syms = extract_symbols(Path::new(rel_path), &src)?;
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
                extract_symbols(Path::new(rel_path), &src)
            } else {
                Ok(Vec::new())
            }
        }
        DiffScope::Staged => {
            let index = repo.repo.index().map_err(|e| CortexError::Git(e.to_string()))?;
            if let Some(entry) = index.get_path(Path::new(rel_path), 0) {
                let blob = repo.repo.find_blob(entry.id).map_err(|e| CortexError::Git(e.to_string()))?;
                extract_symbols(Path::new(rel_path), blob.content())
            } else {
                read_symbols_by_scope(repo, rel_path, DiffScope::WorkingTree)
            }
        }
        DiffScope::Branch => {
            read_symbols_by_scope(repo, rel_path, DiffScope::WorkingTree)
        }
    }
}
