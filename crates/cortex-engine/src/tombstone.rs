use cortex_ast::extract_symbols;
use cortex_ast::uri::{format_symbol_uri, normalize_rel_path, parse_symbol_uri};
use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::jcs::{canonical_hash, to_canonical_json};
use cortex_core::redact::scrub;
use cortex_core::schema::{CaptureMode, TargetSymbol, TombstoneRecord, VerificationEvidence};
use cortex_storage::{CortexDb, CortexRepo};
use std::path::Path;

pub struct RecordTombstoneParams<'a> {
    pub target_symbols: &'a [String],
    pub attempted_approach: &'a str,
    pub failure_reason: &'a str,
    pub test_command: &'a str,
    pub error_signature: &'a str,
    pub author_harness: &'a str,
    pub capture_mode: CaptureMode,
}

pub fn record_tombstone(
    repo: &CortexRepo,
    db: &mut CortexDb,
    params: RecordTombstoneParams,
) -> Result<TombstoneRecord> {
    let id = PrefixedId::new(IdPrefix::Tombstone).to_string();
    let branch = repo.current_branch()?;
    let base_sha = repo.head_commit_sha()?;

    // Scrub confidential data
    let (clean_approach, _) = scrub(params.attempted_approach);
    let (clean_reason, _) = scrub(params.failure_reason);
    let (clean_command, _) = scrub(params.test_command);
    let (clean_sig, _) = scrub(params.error_signature);

    let mut target_symbol_records = Vec::new();
    let mut sym_tuples = Vec::new();

    for sym_str in params.target_symbols {
        let (rel_path, scope) = if let Some((p, s)) = parse_symbol_uri(sym_str) {
            (normalize_rel_path(Path::new(&p)), s)
        } else {
            (normalize_rel_path(Path::new(sym_str)), "<file>".to_string())
        };

        let full_path = repo.workdir.join(&rel_path);
        let ast_hash = if full_path.exists() {
            let src = std::fs::read(&full_path).map_err(CortexError::Io)?;
            let symbols = extract_symbols(Path::new(&rel_path), &src)?;
            if let Some(s) = symbols.iter().find(|s| s.scope_chain == scope) {
                s.ast_hash.clone()
            } else if scope == "<file>" {
                "ast:sha256:0000000000000000".to_string()
            } else {
                return Err(CortexError::SymbolNotFound(scope, rel_path));
            }
        } else {
            "ast:sha256:0000000000000000".to_string()
        };

        let uri = format_symbol_uri(&rel_path, &scope);
        target_symbol_records.push(TargetSymbol {
            symbol_uri: uri.clone(),
            ast_hash: ast_hash.clone(),
        });
        sym_tuples.push((uri, rel_path, ast_hash, "TARGET"));
    }

    let invalidate_symbols = target_symbol_records
        .iter()
        .map(|ts| ts.symbol_uri.clone())
        .collect();

    let record = TombstoneRecord::new(
        id.clone(),
        params.author_harness.to_string(),
        branch.clone(),
        base_sha,
        params.capture_mode,
        target_symbol_records,
        clean_approach,
        clean_reason,
        VerificationEvidence {
            command: clean_command,
            exit_code: 1,
            error_signature: clean_sig,
        },
        invalidate_symbols,
    );

    let canonical_json = to_canonical_json(&record)?;
    let content_hash = canonical_hash(&record)?;

    db.save_record(
        &id,
        "TOMBSTONE",
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

    Ok(record)
}
