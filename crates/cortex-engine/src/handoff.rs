use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::schema::EpistemicHandoff;
use cortex_storage::{CortexDb, CortexRepo};
use serde::{Deserialize, Serialize};

pub const MAX_HANDOFF_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HandoffMode {
    Import,
    Export,
}

pub fn create_handoff_envelope(
    repo: &CortexRepo,
    db: &CortexDb,
    branch_override: Option<&str>,
    objective_override: Option<&str>,
) -> Result<EpistemicHandoff> {
    let branch = match branch_override {
        Some(b) => b.to_string(),
        None => repo.current_branch()?,
    };

    let handoff_id = PrefixedId::new(IdPrefix::Handoff).to_string();
    let head_sha = repo.head_commit_sha()?;
    let dirty_hash = repo.working_tree_dirty_hash()?;
    let objective = objective_override
        .unwrap_or("Active development task")
        .to_string();

    let mut envelope = EpistemicHandoff::new(handoff_id, head_sha, dirty_hash, objective);

    // Populate active tombstone IDs
    let tombstones = db.list_records_by_kind("TOMBSTONE")?;
    for t in &tombstones {
        envelope.active_tombstone_ids.push(t.id.clone());
    }

    // Populate edges & hypotheses
    envelope.verified_call_graph = db.get_call_graph_edges(&branch)?;
    envelope.active_hypotheses = db.get_hypotheses(&branch)?;

    // Populate unchanged files
    let reads = db.get_file_reads(&branch)?;
    for (path, blob_oid) in reads {
        let full = repo.workdir.join(&path);
        if full.exists() {
            if let Ok(bytes) = std::fs::read(&full) {
                let curr_oid = cortex_core::jcs::sha256_digest(&bytes);
                if curr_oid == blob_oid {
                    envelope.do_not_re_read_unchanged.push(path);
                }
            }
        }
    }

    // Enforce 4096-byte token budget
    enforce_byte_budget(&mut envelope, MAX_HANDOFF_BYTES)?;

    Ok(envelope)
}

fn enforce_byte_budget(envelope: &mut EpistemicHandoff, max_bytes: usize) -> Result<()> {
    let mut json = serde_json::to_string(envelope).map_err(CortexError::Json)?;

    if json.len() <= max_bytes {
        return Ok(());
    }

    // Priority 1: Clear or truncate do_not_re_read_unchanged
    envelope.do_not_re_read_unchanged.clear();
    json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Priority 2: Truncate verified_call_graph (keep newest)
    while json.len() > max_bytes && !envelope.verified_call_graph.is_empty() {
        envelope.verified_call_graph.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Priority 3: Truncate active_tombstone_ids
    while json.len() > max_bytes && !envelope.active_tombstone_ids.is_empty() {
        envelope.active_tombstone_ids.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Priority 4: Drop active hypotheses from lowest confidence to highest
    envelope.active_hypotheses.sort_by(|a, b| {
        a.confidence
            .partial_cmp(&b.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    while json.len() > max_bytes && !envelope.active_hypotheses.is_empty() {
        envelope.active_hypotheses.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Priority 5: Truncate objective safely at UTF-8 character boundaries
    while json.len() > max_bytes && envelope.objective.len() > 3 {
        let excess = json.len() - max_bytes;
        let truncate_len = envelope.objective.len().saturating_sub(excess + 3);
        let valid_len = envelope
            .objective
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|&idx| idx <= truncate_len)
            .last()
            .unwrap_or(0);
        envelope.objective.truncate(valid_len);
        envelope.objective.push_str("...");
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
        if envelope.objective == "..." {
            break;
        }
    }

    Ok(())
}

pub fn import_handoff_envelope(
    _repo: &CortexRepo,
    db: &mut CortexDb,
    envelope: &EpistemicHandoff,
    branch: &str,
) -> Result<()> {
    for edge in &envelope.verified_call_graph {
        db.save_call_graph_edge(edge, branch)?;
    }
    for hyp in &envelope.active_hypotheses {
        db.save_hypothesis(hyp, branch)?;
    }
    Ok(())
}

/// Format an EpistemicHandoff as a Google Agent2Agent (A2A) task extension envelope
pub fn format_a2a_task_envelope(
    envelope: &EpistemicHandoff,
    task_id: &str,
    instruction: &str,
) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tasks/send",
        "params": {
            "id": task_id,
            "message": {
                "role": "user",
                "parts": [{ "type": "text", "text": instruction }]
            },
            "metadata": {
                "https://git-cortex.dev/a2a-extension/v0.1": {
                    "handoff_id": envelope.handoff_id,
                    "epistemic_handoff": envelope
                }
            }
        }
    }))
}
