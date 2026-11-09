use cortex_core::error::{CortexError, Result};
use git2::Repository;
use std::path::{Path, PathBuf};

pub struct CortexRepo {
    pub repo: Repository,
    pub workdir: PathBuf,
    pub git_dir: PathBuf,
    pub cortex_dir: PathBuf,
}

impl CortexRepo {
    /// Discover git repository starting from current dir or specified path
    pub fn discover(start_path: Option<&Path>) -> Result<Self> {
        let repo = match start_path {
            Some(p) => Repository::discover(p),
            None => Repository::discover("."),
        }
        .map_err(|e| CortexError::Git(format!("Failed to discover git repository: {}", e)))?;

        let workdir = repo
            .workdir()
            .ok_or_else(|| CortexError::Git("Bare repositories are not supported".to_string()))?
            .to_path_buf();

        // git_dir handles worktrees: repo.path() points to gitdir
        let git_dir = repo.path().to_path_buf();
        let cortex_dir = git_dir.join("cortex");

        if !cortex_dir.exists() {
            std::fs::create_dir_all(&cortex_dir)
                .map_err(|e| CortexError::Io(e))?;
        }

        Ok(Self {
            repo,
            workdir,
            git_dir,
            cortex_dir,
        })
    }

    /// Return path to SQLite database (.git/cortex/state.db)
    pub fn state_db_path(&self) -> PathBuf {
        self.cortex_dir.join("state.db")
    }

    /// Return current branch name, or "HEAD" if detached
    pub fn current_branch(&self) -> Result<String> {
        match self.repo.head() {
            Ok(head) => {
                if let Some(shorthand) = head.shorthand() {
                    Ok(shorthand.to_string())
                } else {
                    Ok("HEAD".to_string())
                }
            }
            Err(_) => Ok("main".to_string()),
        }
    }

    /// Return current HEAD commit SHA
    pub fn head_commit_sha(&self) -> Result<String> {
        match self.repo.head() {
            Ok(head) => {
                let target = head
                    .target()
                    .ok_or_else(|| CortexError::Git("HEAD reference has no target".to_string()))?;
                Ok(target.to_string())
            }
            Err(_) => Ok("0000000000000000000000000000000000000000".to_string()),
        }
    }

    /// Compute working tree dirty hash over modified tracked and untracked files
    pub fn working_tree_dirty_hash(&self) -> Result<String> {
        let mut status_opts = git2::StatusOptions::new();
        status_opts
            .include_untracked(true)
            .include_ignored(false)
            .recurse_untracked_dirs(true);

        let statuses = self
            .repo
            .statuses(Some(&mut status_opts))
            .map_err(|e| CortexError::Git(format!("Failed to get repository status: {}", e)))?;

        let mut hasher = sha2::Sha256::default();
        let mut count = 0;

        for entry in statuses.iter() {
            if let Some(path) = entry.path() {
                sha2::Digest::update(&mut hasher, path.as_bytes());
                sha2::Digest::update(&mut hasher, format!("{:?}", entry.status()).as_bytes());
                count += 1;
            }
        }

        if count == 0 {
            Ok("wt:clean".to_string())
        } else {
            let digest = hex::encode(sha2::Digest::finalize(hasher));
            Ok(format!("wt:sha256:{}", &digest[..16]))
        }
    }
}
