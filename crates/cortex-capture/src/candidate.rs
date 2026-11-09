use cortex_ast::extract_symbols;
use cortex_ast::uri::parse_symbol_uri;
use cortex_core::error::Result;
use cortex_core::schema::CaptureMode;
use cortex_engine::tombstone::{record_tombstone, RecordTombstoneParams};
use cortex_storage::{CortexDb, CortexRepo};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchedSymbol {
    pub symbol_uri: String,
    pub base_ast_hash: String,
    pub dirty_ast_hash: String,
}

pub fn reconcile_candidates(
    repo: &CortexRepo,
    db: &mut CortexDb,
) -> Result<Vec<cortex_core::schema::TombstoneRecord>> {
    let failed_candidates = db.get_candidates_by_state("FAILED")?;
    let mut promoted_records = Vec::new();

    for cand in failed_candidates {
        let symbols: Vec<TouchedSymbol> =
            serde_json::from_str(&cand.symbols_json).unwrap_or_default();
        if symbols.is_empty() {
            continue;
        }

        let mut all_reverted = true;

        for ts in &symbols {
            if let Some((rel_path, scope)) = parse_symbol_uri(&ts.symbol_uri) {
                let full = repo.workdir.join(&rel_path);
                if full.exists() {
                    if let Ok(src) = std::fs::read(&full) {
                        if let Ok(syms) = extract_symbols(Path::new(&rel_path), &src) {
                            if let Some(s) = syms.iter().find(|s| s.scope_chain == scope) {
                                if s.ast_hash != ts.base_ast_hash {
                                    all_reverted = false;
                                    break;
                                }
                            } else {
                                all_reverted = false;
                                break;
                            }
                        } else {
                            all_reverted = false;
                            break;
                        }
                    } else {
                        all_reverted = false;
                        break;
                    }
                } else {
                    all_reverted = false;
                    break;
                }
            } else {
                all_reverted = false;
                break;
            }
        }

        if all_reverted {
            let latest_exec_id = cand.exec_ids.split(',').next_back().unwrap_or("").trim();

            let (cmd, sig) = if let Ok(Some(exec)) = db.get_exec(latest_exec_id) {
                (exec.cmd, exec.signature)
            } else {
                (
                    "unknown test command".to_string(),
                    "Test failed before revert".to_string(),
                )
            };

            let sym_uris: Vec<String> = symbols.iter().map(|s| s.symbol_uri.clone()).collect();
            let approach = format!("Modified {} ({})", sym_uris.join(", "), cand.diff);
            let reason = format!("Failed verification with signature: {}", sig);

            let record = record_tombstone(
                repo,
                db,
                RecordTombstoneParams {
                    target_symbols: &sym_uris,
                    attempted_approach: &approach,
                    failure_reason: &reason,
                    test_command: &cmd,
                    error_signature: &sig,
                    author_harness: "git-cortex/passive-capture",
                    capture_mode: CaptureMode::AutoRevertDetected,
                },
            )?;

            let mut updated = cand;
            updated.state = "PROMOTED".to_string();
            db.save_candidate(&updated)?;

            promoted_records.push(record);
        }
    }

    Ok(promoted_records)
}
