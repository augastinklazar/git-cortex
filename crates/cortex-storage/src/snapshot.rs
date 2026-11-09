use cortex_core::error::{CortexError, Result};
use git2::build::CheckoutBuilder;
use git2::{IndexAddOption, Oid, Repository, Signature};

pub struct CortexSnapshot;

impl CortexSnapshot {
    /// Create a working-tree snapshot commit stored under `refs/cortex/<category>/<id>`
    /// Uses an isolated temporary index to prevent mutating the repository's live `.git/index`.
    pub fn create_snapshot(
        repo: &Repository,
        category: &str, // "checkpoints" or "backups"
        id: &str,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<(Oid, String)> {
        let ref_name = format!("refs/cortex/{}/{}", category, id);

        // Save reference to current live index
        let mut live_index = repo
            .index()
            .map_err(|e| CortexError::Git(format!("Failed to open repo index: {}", e)))?;

        // Create an isolated temporary index file inside .git/cortex/
        let cortex_dir = repo.path().join("cortex");
        std::fs::create_dir_all(&cortex_dir).map_err(CortexError::Io)?;
        let temp_index_path = cortex_dir.join(format!("tmp_index_{}", id));
        let mut temp_index =
            git2::Index::open(&temp_index_path).map_err(|e| CortexError::Git(e.to_string()))?;

        // Associate temp_index with repo so update_all and add_all have workdir context
        repo.set_index(&mut temp_index)
            .map_err(|e| CortexError::Git(e.to_string()))?;

        // Seed temporary index from HEAD tree if HEAD exists
        if let Ok(head_tree) = repo.head().and_then(|h| h.peel_to_tree()) {
            let _ = temp_index.read_tree(&head_tree);
        }

        // Reflect working-tree deletions and modifications/additions in temp index
        let update_res = temp_index.update_all(["*"].iter(), None);
        let add_res = temp_index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None);
        let tree_oid_res = temp_index.write_tree_to(repo);

        // Always restore the live repository index before returning
        let _ = repo.set_index(&mut live_index);
        let _ = std::fs::remove_file(&temp_index_path);

        update_res.map_err(|e| CortexError::Git(format!("Failed to update temp index: {}", e)))?;
        add_res.map_err(|e| CortexError::Git(format!("Failed to add to temp index: {}", e)))?;
        let tree_oid = tree_oid_res
            .map_err(|e| CortexError::Git(format!("Failed to write snapshot tree: {}", e)))?;

        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| CortexError::Git(e.to_string()))?;

        let sig = Signature::now(author_name, author_email)
            .map_err(|e| CortexError::Git(e.to_string()))?;

        // Get HEAD commit as parent if available
        let head_commit = match repo.head().and_then(|h| h.peel_to_commit()) {
            Ok(c) => vec![c],
            Err(_) => vec![],
        };
        let parents: Vec<&git2::Commit> = head_commit.iter().collect();

        let commit_oid = repo
            .commit(Some(&ref_name), &sig, &sig, message, &tree, &parents)
            .map_err(|e| {
                CortexError::Git(format!(
                    "Failed to create snapshot commit {}: {}",
                    ref_name, e
                ))
            })?;

        Ok((commit_oid, ref_name))
    }

    /// Restore working tree to a snapshot commit
    pub fn restore_snapshot(repo: &Repository, snapshot_ref: &str) -> Result<()> {
        let reference = repo.find_reference(snapshot_ref).map_err(|e| {
            CortexError::Git(format!("Snapshot ref {} not found: {}", snapshot_ref, e))
        })?;

        let commit = reference
            .peel_to_commit()
            .map_err(|e| CortexError::Git(format!("Failed to peel ref to commit: {}", e)))?;

        let tree = commit.tree().map_err(|e| CortexError::Git(e.to_string()))?;

        let mut opts = CheckoutBuilder::new();
        opts.force().remove_untracked(true);

        repo.checkout_tree(tree.as_object(), Some(&mut opts))
            .map_err(|e| CortexError::Git(format!("Failed to checkout snapshot tree: {}", e)))?;

        // Reset index to match snapshot tree
        let mut index = repo.index().map_err(|e| CortexError::Git(e.to_string()))?;
        index
            .read_tree(&tree)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        index.write().map_err(|e| CortexError::Git(e.to_string()))?;

        Ok(())
    }
}
