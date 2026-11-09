use crate::tombstone::{record_tombstone, RecordTombstoneParams};
use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_core::schema::CaptureMode;
use cortex_storage::{CortexDb, CortexRepo, CortexSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindResult {
    pub target_checkpoint_id: String,
    pub backup_ref: String,
    pub synthesized_tombstone_id: String,
    pub status_message: String,
}

pub fn rewind_to_checkpoint(
    repo: &CortexRepo,
    db: &mut CortexDb,
    checkpoint_id: &str,
    reason: Option<&str>,
) -> Result<RewindResult> {
    let checkpoint_ref = format!("refs/cortex/checkpoints/{}", checkpoint_id);
    let _check_reference = repo
        .repo
        .find_reference(&checkpoint_ref)
        .map_err(|e| CortexError::Rewind(format!("Checkpoint {} not found: {}", checkpoint_id, e)))?;

    // Step 0: Backup current dirty/aborted state safely so user work is never destroyed!
    let backup_id = PrefixedId::new(IdPrefix::Checkpoint).to_string();
    let (_, backup_ref) = CortexSnapshot::create_snapshot(
        &repo.repo,
        "backups",
        &backup_id,
        "Pre-rewind automatic backup",
        "git-cortex",
        "cortex@git-cortex.dev",
    )?;

    // Step 1: Synthesize a summary TombstoneRecord covering the diff
    let fallback_reason = reason.unwrap_or("Aborted exploration rolled back via cortex rewind");
    let tombstone = record_tombstone(
        repo,
        db,
        RecordTombstoneParams {
            target_symbols: &[],
            attempted_approach: "Exploratory changes leading up to rewind",
            failure_reason: fallback_reason,
            test_command: "cortex rewind",
            error_signature: "ABORTED_WORKTREE_REWIND",
            author_harness: "git-cortex/rewind",
            capture_mode: CaptureMode::RewindSynthesized,
        },
    )?;

    // Step 2 & 3: Restore working tree and index cleanly to checkpoint
    CortexSnapshot::restore_snapshot(&repo.repo, &checkpoint_ref)?;

    Ok(RewindResult {
        target_checkpoint_id: checkpoint_id.to_string(),
        backup_ref,
        synthesized_tombstone_id: tombstone.id,
        status_message: format!(
            "Successfully rewound to {}. Working tree clean. Immunity tombstone recorded.",
            checkpoint_id
        ),
    })
}
