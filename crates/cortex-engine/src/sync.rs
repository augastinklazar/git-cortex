use cortex_core::error::Result;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo, LEDGER_REF};
use std::process::Command;

pub fn sync_ledger(repo: &CortexRepo, db: &mut CortexDb, remote: Option<&str>) -> Result<String> {
    let remote_name = remote.unwrap_or("origin");

    // 1. Fetch remote ledger ref using git CLI
    let fetch_refspec = format!("{}:{}", LEDGER_REF, LEDGER_REF);
    let output = Command::new("git")
        .current_dir(&repo.workdir)
        .args(["fetch", remote_name, &fetch_refspec])
        .output();

    let fetch_msg = match output {
        Ok(out) => {
            if out.status.success() {
                format!("Fetched {} from {}", LEDGER_REF, remote_name)
            } else {
                format!(
                    "Fetch notice (ref may not exist on remote yet): {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )
            }
        }
        Err(e) => format!("Could not run git fetch: {}", e),
    };

    // 2. Reindex ledger into local DB
    let reindexed = CortexLedger::reindex_into_db(&repo.repo, db)?;

    // 3. Push ledger to remote
    let push_output = Command::new("git")
        .current_dir(&repo.workdir)
        .args(["push", remote_name, &fetch_refspec])
        .output();

    let push_msg = match push_output {
        Ok(out) => {
            if out.status.success() {
                format!("Pushed {} to {}", LEDGER_REF, remote_name)
            } else {
                format!(
                    "Push failed or up to date: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )
            }
        }
        Err(e) => format!("Could not run git push: {}", e),
    };

    Ok(format!(
        "{}\nRe-indexed {} records into local state cache.\n{}",
        fetch_msg, reindexed, push_msg
    ))
}
