use cortex_core::error::Result;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo, LEDGER_REF};
use std::process::Command;

pub fn sync_ledger(repo: &CortexRepo, db: &mut CortexDb, remote: Option<&str>) -> Result<String> {
    let remote_name = remote.unwrap_or("origin");

    // 1. Fetch remote ledger ref into a remote-tracking reference
    let remote_tracking_ref = format!("refs/remotes/{}/cortex/ledger", remote_name);
    let fetch_refspec = format!("{}:{}", LEDGER_REF, remote_tracking_ref);
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

    // 2. If remote tracking ref exists, merge cleanly into local ledger
    let mut merge_msg = String::new();
    if let Ok(reference) = repo.repo.find_reference(&remote_tracking_ref) {
        if let Some(remote_oid) = reference.target() {
            match CortexLedger::merge_commits(
                &repo.repo,
                remote_oid,
                "git-cortex",
                "cortex@git-cortex.dev",
            )? {
                Some(merge_oid) => {
                    merge_msg = format!("Merged remote ledger commit {}.", merge_oid);
                }
                None => {
                    merge_msg = "Ledger already in sync with remote.".to_string();
                }
            }
        }
    }

    // 3. Reindex ledger into local DB
    let reindexed = CortexLedger::reindex_into_db(&repo.repo, db)?;

    // 4. Push local ledger to remote
    let push_refspec = format!("{}:{}", LEDGER_REF, LEDGER_REF);
    let push_output = Command::new("git")
        .current_dir(&repo.workdir)
        .args(["push", remote_name, &push_refspec])
        .output();

    let push_msg = match push_output {
        Ok(out) => {
            if out.status.success() {
                format!("Pushed {} to {}", LEDGER_REF, remote_name)
            } else {
                format!(
                    "Push notice: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )
            }
        }
        Err(e) => format!("Could not run git push: {}", e),
    };

    let full_status = if merge_msg.is_empty() {
        format!(
            "{}\nRe-indexed {} records into local state cache.\n{}",
            fetch_msg, reindexed, push_msg
        )
    } else {
        format!(
            "{}\n{}\nRe-indexed {} records into local state cache.\n{}",
            fetch_msg, merge_msg, reindexed, push_msg
        )
    };

    Ok(full_status)
}
