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
    let _check_reference = repo.repo.find_reference(&checkpoint_ref).map_err(|e| {
        CortexError::Rewind(format!("Checkpoint {} not found: {}", checkpoint_id, e))
    })?;

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
    let modified_symbols =
        find_modified_symbols_against_checkpoint(repo, &checkpoint_ref).unwrap_or_default();

    let approach = if modified_symbols.is_empty() {
        "Exploratory changes leading up to rewind".to_string()
    } else {
        format!(
            "Exploratory changes touching {} reverted via cortex rewind",
            modified_symbols.join(", ")
        )
    };

    // Step 2: Restore working tree and index cleanly to checkpoint
    CortexSnapshot::restore_snapshot(&repo.repo, &checkpoint_ref)?;

    // Step 3: Record immunity tombstone on restored target symbols
    let tombstone = record_tombstone(
        repo,
        db,
        RecordTombstoneParams {
            target_symbols: &modified_symbols,
            attempted_approach: &approach,
            failure_reason: fallback_reason,
            test_command: "cortex rewind",
            error_signature: "ABORTED_WORKTREE_REWIND",
            author_harness: "git-cortex/rewind",
            capture_mode: CaptureMode::RewindSynthesized,
        },
    )?;

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

fn find_modified_symbols_against_checkpoint(
    repo: &CortexRepo,
    checkpoint_ref: &str,
) -> Result<Vec<String>> {
    let reference = repo
        .repo
        .find_reference(checkpoint_ref)
        .map_err(|e| CortexError::Rewind(format!("Checkpoint reference not found: {}", e)))?;
    let commit = reference
        .peel_to_commit()
        .map_err(|e| CortexError::Rewind(format!("Failed to peel checkpoint to commit: {}", e)))?;
    let tree = commit.tree().map_err(|e| CortexError::Git(e.to_string()))?;

    let mut status_opts = git2::StatusOptions::new();
    status_opts.include_untracked(true).include_ignored(false);
    let statuses = repo
        .repo
        .statuses(Some(&mut status_opts))
        .map_err(|e| CortexError::Git(e.to_string()))?;

    let mut modified = Vec::new();
    for entry in statuses.iter() {
        if let Some(rel_path_str) = entry.path() {
            let rel_path = std::path::Path::new(rel_path_str);
            let full_path = repo.workdir.join(rel_path);
            if !full_path.exists() {
                continue;
            }
            if let Ok(current_src) = std::fs::read(&full_path) {
                if let Ok(current_syms) = cortex_ast::extract_symbols(rel_path, &current_src) {
                    let tree_syms = if let Ok(tree_entry) = tree.get_path(rel_path) {
                        if let Ok(blob) = repo.repo.find_blob(tree_entry.id()) {
                            cortex_ast::extract_symbols(rel_path, blob.content())
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    let tree_map: std::collections::HashMap<String, String> =
                        tree_syms.into_iter().map(|s| (s.uri, s.ast_hash)).collect();

                    for curr in current_syms {
                        match tree_map.get(&curr.uri) {
                            Some(base_hash) if base_hash != &curr.ast_hash => {
                                modified.push(curr.uri);
                            }
                            None => {
                                modified.push(curr.uri);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::checkpoint_state;
    use cortex_storage::CortexDb;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_rewind_captures_modified_symbols() {
        let temp = TempDir::new().unwrap();
        let git_repo = git2::Repository::init(temp.path()).unwrap();

        // Commit initial code
        let file = temp.path().join("service.rs");
        std::fs::write(&file, "pub fn process_order() -> bool { true }\n").unwrap();

        let mut index = git_repo.index().unwrap();
        index.add_path(Path::new("service.rs")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();

        let repo = CortexRepo::discover(Some(temp.path())).unwrap();
        let mut db = CortexDb::open_in_memory().unwrap();

        // 1. Create a checkpoint
        let chk = checkpoint_state(&repo, &mut db, "Stable baseline", &[], &[]).unwrap();

        // 2. Modify process_order in working tree
        std::fs::write(
            &file,
            "pub fn process_order() -> bool { false /* broken */ }\n",
        )
        .unwrap();

        // 3. Rewind back to checkpoint
        let rewind =
            rewind_to_checkpoint(&repo, &mut db, &chk.checkpoint_id, Some("Logic broke")).unwrap();

        // Verify working tree restored
        let restored_src = std::fs::read_to_string(&file).unwrap();
        assert!(restored_src.contains("true"));

        // Verify synthesized tombstone captured target symbol
        let record = db
            .get_record(&rewind.synthesized_tombstone_id)
            .unwrap()
            .unwrap();
        let tombstone: cortex_core::schema::TombstoneRecord =
            serde_json::from_str(&record.json).unwrap();
        assert_eq!(tombstone.target_symbols.len(), 1);
        assert_eq!(
            tombstone.target_symbols[0].symbol_uri,
            "service.rs#process_order"
        );
        assert!(tombstone
            .attempted_approach
            .contains("service.rs#process_order"));
    }
}
