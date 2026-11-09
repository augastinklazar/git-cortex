use cortex_core::error::Result;
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::schema::{ActiveHypothesis, CallGraphEdge};
use cortex_storage::{CortexDb, CortexRepo, CortexSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointResult {
    pub checkpoint_id: String,
    pub snapshot_ref: String,
    pub branch: String,
    pub objective: String,
    pub edge_count: usize,
    pub hypothesis_count: usize,
}

pub fn checkpoint_state(
    repo: &CortexRepo,
    db: &mut CortexDb,
    objective: &str,
    verified_edges: &[CallGraphEdge],
    active_hypotheses: &[ActiveHypothesis],
) -> Result<CheckpointResult> {
    let chk_id = PrefixedId::new(IdPrefix::Checkpoint).to_string();
    let branch = repo.current_branch()?;

    // Create snapshot
    let (_, snapshot_ref) = CortexSnapshot::create_snapshot(
        &repo.repo,
        "checkpoints",
        &chk_id,
        objective,
        "git-cortex",
        "cortex@git-cortex.dev",
    )?;

    // Upsert edges into db
    for edge in verified_edges {
        db.save_call_graph_edge(edge, &branch)?;
    }

    // Upsert hypotheses into db
    for hyp in active_hypotheses {
        db.save_hypothesis(hyp, &branch)?;
    }

    // Persist checkpoint metadata in SQLite checkpoints table
    let state_json = serde_json::json!({
        "edge_count": verified_edges.len(),
        "hypothesis_count": active_hypotheses.len(),
    })
    .to_string();
    db.save_checkpoint(&chk_id, &branch, &snapshot_ref, objective, &state_json)?;

    Ok(CheckpointResult {
        checkpoint_id: chk_id,
        snapshot_ref,
        branch,
        objective: objective.to_string(),
        edge_count: verified_edges.len(),
        hypothesis_count: active_hypotheses.len(),
    })
}
