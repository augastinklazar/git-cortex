use cortex_ast::extract_symbols;
use cortex_ast::uri::{normalize_rel_path, parse_symbol_uri};
use cortex_core::error::{CortexError, Result};
use cortex_core::fence::{fence_invariant, fence_tombstone, Provenance};
use cortex_core::schema::{InvariantContract, TombstoneRecord};
use cortex_storage::{CortexDb, CortexRepo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    pub filepath: String,
    pub symbol_name: Option<String>,
    pub current_symbols: Vec<cortex_ast::Symbol>,
    pub active_tombstones: Vec<TombstoneRecord>,
    pub stale_tombstones: Vec<(TombstoneRecord, String)>,
    pub active_invariants: Vec<InvariantContract>,
    pub stale_invariants: Vec<(InvariantContract, String)>,
    pub formatted_markdown: String,
}

pub fn inspect_symbol(
    repo: &CortexRepo,
    db: &CortexDb,
    filepath: &str,
    symbol_name: Option<&str>,
) -> Result<InspectResult> {
    let norm_path = normalize_rel_path(Path::new(filepath));
    let full_path = repo.workdir.join(&norm_path);

    let symbols = if full_path.exists() {
        let source = std::fs::read(&full_path).map_err(CortexError::Io)?;
        let blob_oid = cortex_core::jcs::sha256_digest(&source);
        if let Ok(Some(cached_json)) = db.get_cached_symbols(&norm_path, &blob_oid) {
            serde_json::from_str::<Vec<cortex_ast::Symbol>>(&cached_json).unwrap_or_else(|_| {
                extract_symbols(Path::new(&norm_path), &source).unwrap_or_default()
            })
        } else {
            let extracted = extract_symbols(Path::new(&norm_path), &source)?;
            if let Ok(json) = serde_json::to_string(&extracted) {
                let _ = db.set_cached_symbols(&norm_path, &blob_oid, &json);
            }
            extracted
        }
    } else {
        Vec::new()
    };

    let mut symbol_map: HashMap<String, String> = HashMap::new();
    for sym in &symbols {
        symbol_map.insert(sym.uri.clone(), sym.ast_hash.clone());
    }

    let records = db.query_records_by_path(&norm_path)?;
    let events = db.list_records_by_kind("EVENT").unwrap_or_default();
    let mut revoked_records = HashMap::new();
    for evt_rec in events {
        if let Ok(evt) = serde_json::from_str::<cortex_core::schema::LifecycleEvent>(&evt_rec.json)
        {
            if evt.event_type == cortex_core::schema::EventType::Revoke {
                revoked_records.insert(evt.target_id, evt.reason);
            }
        }
    }

    let mut active_tombstones = Vec::new();
    let mut stale_tombstones = Vec::new();
    let mut active_invariants = Vec::new();
    let mut stale_invariants = Vec::new();
    let mut markdown_sections = Vec::new();

    for stored in records {
        let provenance = if stored.origin == "LEDGER" {
            Provenance::LedgerShared
        } else {
            Provenance::LocalWorkingTree
        };

        if stored.kind == "TOMBSTONE" {
            if let Ok(record) = serde_json::from_str::<TombstoneRecord>(&stored.json) {
                if let Some(reason) = revoked_records.get(&record.id) {
                    markdown_sections.push(format!(
                        "<!-- REVOKED TOMBSTONE: {} (Reason: {}) -->",
                        record.id, reason
                    ));
                    continue;
                }

                if let Some(target_filter) = symbol_name {
                    let matches_filter = record.target_symbols.iter().any(|ts| {
                        ts.symbol_uri.ends_with(target_filter)
                            || ts.symbol_uri.contains(target_filter)
                    });
                    if !matches_filter {
                        continue;
                    }
                }

                let mut is_active = true;
                let mut changed = Vec::new();

                for ts in &record.target_symbols {
                    if let Some(curr_hash) = symbol_map.get(&ts.symbol_uri) {
                        if curr_hash != &ts.ast_hash {
                            is_active = false;
                            changed.push(format!("{}: hash drifted", ts.symbol_uri));
                        }
                    } else {
                        is_active = false;
                        changed.push(format!("{}: symbol missing", ts.symbol_uri));
                    }
                }

                if is_active {
                    markdown_sections.push(fence_tombstone(&record, provenance));
                    active_tombstones.push(record);
                } else {
                    let reason = changed.join(", ");
                    markdown_sections.push(format!(
                        "<!-- STALE TOMBSTONE: {} (Superseeded: {}) -->",
                        record.id, reason
                    ));
                    stale_tombstones.push((record, reason));
                }
            }
        } else if stored.kind == "INVARIANT" {
            if let Ok(contract) = serde_json::from_str::<InvariantContract>(&stored.json) {
                if let Some(reason) = revoked_records.get(&contract.invariant_id) {
                    markdown_sections.push(format!(
                        "<!-- REVOKED INVARIANT: {} (Reason: {}) -->",
                        contract.invariant_id, reason
                    ));
                    continue;
                }

                if let Some(target_filter) = symbol_name {
                    let matches_filter = contract.guarded_symbol.symbol_uri.contains(target_filter)
                        || contract
                            .upstream_dependencies
                            .iter()
                            .any(|u| u.symbol_uri.contains(target_filter));
                    if !matches_filter {
                        continue;
                    }
                }

                let mut is_active = true;
                let mut changed = Vec::new();

                if let Some(curr_hash) = symbol_map.get(&contract.guarded_symbol.symbol_uri) {
                    if curr_hash != &contract.guarded_symbol.ast_hash {
                        is_active = false;
                        changed.push(format!(
                            "Guarded symbol {} modified",
                            contract.guarded_symbol.symbol_uri
                        ));
                    }
                } else {
                    is_active = false;
                    changed.push(format!(
                        "Guarded symbol {} missing or deleted",
                        contract.guarded_symbol.symbol_uri
                    ));
                }

                for dep in &contract.upstream_dependencies {
                    if let Some((dep_path, dep_scope)) = parse_symbol_uri(&dep.symbol_uri) {
                        let dep_full_path = repo.workdir.join(&dep_path);
                        let dep_curr_hash = if dep_full_path.exists() {
                            if let Ok(src) = std::fs::read(&dep_full_path) {
                                if let Ok(syms) = extract_symbols(Path::new(&dep_path), &src) {
                                    syms.into_iter()
                                        .find(|s| s.scope_chain == dep_scope)
                                        .map(|s| s.ast_hash)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        match dep_curr_hash {
                            Some(h) if h == dep.expected_ast_hash => {}
                            Some(_) => {
                                is_active = false;
                                changed.push(format!(
                                    "Upstream dependency {} drifted",
                                    dep.symbol_uri
                                ));
                            }
                            None => {
                                is_active = false;
                                changed.push(format!(
                                    "Upstream dependency {} missing or deleted",
                                    dep.symbol_uri
                                ));
                            }
                        }
                    }
                }

                if is_active {
                    markdown_sections.push(fence_invariant(&contract, provenance));
                    active_invariants.push(contract);
                } else {
                    let reason = changed.join(", ");
                    markdown_sections.push(format!(
                        "<!-- STALE INVARIANT: {} (Drifted: {}) -->",
                        contract.invariant_id, reason
                    ));
                    stale_invariants.push((contract, reason));
                }
            }
        }
    }

    let formatted_markdown = if markdown_sections.is_empty() {
        format!(
            "<!-- CORTEX_INSPECT: No active Tombstones or Invariants found for {} -->",
            norm_path
        )
    } else {
        markdown_sections.join("\n\n")
    };

    Ok(InspectResult {
        filepath: norm_path,
        symbol_name: symbol_name.map(|s| s.to_string()),
        current_symbols: symbols,
        active_tombstones,
        stale_tombstones,
        active_invariants,
        stale_invariants,
        formatted_markdown,
    })
}
