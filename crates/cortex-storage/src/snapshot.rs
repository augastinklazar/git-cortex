use cortex_core::error::{CortexError, Result};
use git2::build::CheckoutBuilder;
use git2::{IndexAddOption, Oid, Repository, Signature};

pub struct CortexSnapshot;

impl CortexSnapshot {
    /// Create a working-tree snapshot commit stored under `refs/cortex/<category>/<id>`
    pub fn create_snapshot(
        repo: &Repository,
        category: &str, // "checkpoints" or "backups"
        id: &str,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<(Oid, String)> {
        let ref_name = format!("refs/cortex/{}/{}", category, id);

        // We create a temporary index from worktree to include tracked + untracked files
        let mut index = repo
            .index()
            .map_err(|e| CortexError::Git(format!("Failed to open repo index: {}", e)))?;

        // Add all modified tracked and untracked files respecting .gitignore
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .map_err(|e| CortexError::Git(format!("Failed to add files to snapshot index: {}", e)))?;

        let tree_oid = index
            .write_tree()
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
            .commit(
                Some(&ref_name),
                &sig,
                &sig,
                message,
                &tree,
                &parents,
            )
            .map_err(|e| CortexError::Git(format!("Failed to create snapshot commit {}: {}", ref_name, e)))?;

        Ok((commit_oid, ref_name))
    }

    /// Restore working tree to a snapshot commit
    pub fn restore_snapshot(repo: &Repository, snapshot_ref: &str) -> Result<()> {
        let reference = repo
            .find_reference(snapshot_ref)
            .map_err(|e| CortexError::Git(format!("Snapshot ref {} not found: {}", snapshot_ref, e)))?;

        let commit = reference
            .peel_to_commit()
            .map_err(|e| CortexError::Git(format!("Failed to peel ref to commit: {}", e)))?;

        let tree = commit
            .tree()
            .map_err(|e| CortexError::Git(e.to_string()))?;

        let mut opts = CheckoutBuilder::new();
        opts.force()
            .remove_untracked(true);

        repo.checkout_tree(tree.as_object(), Some(&mut opts))
            .map_err(|e| CortexError::Git(format!("Failed to checkout snapshot tree: {}", e)))?;

        // Reset index to match snapshot tree
        let mut index = repo
            .index()
            .map_err(|e| CortexError::Git(e.to_string()))?;
        index
            .read_tree(&tree)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        index
            .write()
            .map_err(|e| CortexError::Git(e.to_string()))?;

        Ok(())
    }
}
