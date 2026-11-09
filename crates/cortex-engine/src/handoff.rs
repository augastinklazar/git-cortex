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

    // Filter out revoked records via LifecycleEvent
    let mut revoked_ids = std::collections::HashSet::new();
    if let Ok(events) = db.list_records_by_kind("EVENT") {
        for evt_rec in events {
            if let Ok(evt) =
                serde_json::from_str::<cortex_core::schema::LifecycleEvent>(&evt_rec.json)
            {
                if evt.event_type == cortex_core::schema::EventType::Revoke {
                    revoked_ids.insert(evt.target_id);
                }
            }
        }
    }

    // Populate active tombstone IDs (excluding revoked tombstones)
    let tombstones = db.list_records_by_kind("TOMBSTONE")?;
    for t in &tombstones {
        if !revoked_ids.contains(&t.id) {
            envelope.active_tombstone_ids.push(t.id.clone());
        }
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

    // Enforce 4096-byte token budget adaptively
    enforce_byte_budget(&mut envelope, MAX_HANDOFF_BYTES)?;

    Ok(envelope)
}

fn enforce_byte_budget(envelope: &mut EpistemicHandoff, max_bytes: usize) -> Result<()> {
    let mut json = serde_json::to_string(envelope).map_err(CortexError::Json)?;

    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 1: Clear do_not_re_read_unchanged file cache hints (safest to drop)
    envelope.do_not_re_read_unchanged.clear();
    json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 2: Shed speculative low-confidence hypotheses (confidence < 0.5), lowest first
    envelope.active_hypotheses.sort_by(|a, b| {
        a.confidence
            .partial_cmp(&b.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    while json.len() > max_bytes {
        if let Some(pos) = envelope
            .active_hypotheses
            .iter()
            .position(|h| h.confidence < 0.5)
        {
            envelope.active_hypotheses.remove(pos);
            json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
        } else {
            break;
        }
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 3: Truncate verified call-graph edges (oldest first)
    while json.len() > max_bytes && !envelope.verified_call_graph.is_empty() {
        envelope.verified_call_graph.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 4: Shed medium-confidence hypotheses (0.5 <= confidence < 0.8), lowest first
    while json.len() > max_bytes {
        if let Some(pos) = envelope
            .active_hypotheses
            .iter()
            .position(|h| h.confidence < 0.8)
        {
            envelope.active_hypotheses.remove(pos);
            json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
        } else {
            break;
        }
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 5: Truncate active tombstone IDs (oldest first, preserving recent negative knowledge)
    while json.len() > max_bytes && !envelope.active_tombstone_ids.is_empty() {
        envelope.active_tombstone_ids.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 6: Shed remaining high-confidence hypotheses (lowest confidence first)
    while json.len() > max_bytes && !envelope.active_hypotheses.is_empty() {
        envelope.active_hypotheses.remove(0);
        json = serde_json::to_string(envelope).map_err(CortexError::Json)?;
    }
    if json.len() <= max_bytes {
        return Ok(());
    }

    // Tier 7: Truncate objective safely at UTF-8 character boundaries
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

    if json.len() > max_bytes {
        envelope.objective.clear();
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
