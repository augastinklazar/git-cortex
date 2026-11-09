use cortex_core::error::{CortexError, Result};
use cortex_storage::{CortexDb, CortexLedger, CortexRepo, LEDGER_REF};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

pub const MAX_SYNC_RETRIES: usize = 5;

pub fn sync_ledger(repo: &CortexRepo, db: &mut CortexDb, remote: Option<&str>) -> Result<String> {
    sync_ledger_internal(repo, db, remote, false)
}

pub fn sync_ledger_rebase(
    repo: &CortexRepo,
    db: &mut CortexDb,
    remote: Option<&str>,
) -> Result<String> {
    sync_ledger_internal(repo, db, remote, true)
}

fn sync_ledger_internal(
    repo: &CortexRepo,
    db: &mut CortexDb,
    remote: Option<&str>,
    force_rebase: bool,
) -> Result<String> {
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
    let mut reindexed = CortexLedger::reindex_into_db(&repo.repo, db)?;

    // 4. Push local ledger to remote with CAS / backoff retry loop
    let push_refspec = format!("{}:{}", LEDGER_REF, LEDGER_REF);
    let mut last_push_err = String::new();
    let mut push_msg = String::new();

    for attempt in 0..MAX_SYNC_RETRIES {
        let push_output = Command::new("git")
            .current_dir(&repo.workdir)
            .args(["push", remote_name, &push_refspec])
            .output();

        match push_output {
            Ok(out) => {
                if out.status.success() {
                    push_msg = if attempt > 0 {
                        format!(
                            "Pushed {} to {} (after {} rebase/retry attempts)",
                            LEDGER_REF, remote_name, attempt
                        )
                    } else {
                        format!("Pushed {} to {}", LEDGER_REF, remote_name)
                    };
                    break;
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let is_divergent = stderr.contains("rejected")
                        || stderr.contains("non-fast-forward")
                        || stderr.contains("fetch first")
                        || stderr.contains("failed to push some refs")
                        || force_rebase;

                    if is_divergent && attempt + 1 < MAX_SYNC_RETRIES {
                        sleep(Duration::from_millis(50 * (1 << attempt)));

                        // Re-fetch and union-merge divergent commit
                        let _ = Command::new("git")
                            .current_dir(&repo.workdir)
                            .args(["fetch", remote_name, &fetch_refspec])
                            .output();

                        if let Ok(reference) = repo.repo.find_reference(&remote_tracking_ref) {
                            if let Some(remote_oid) = reference.target() {
                                if let Ok(Some(rebase_oid)) = CortexLedger::merge_commits(
                                    &repo.repo,
                                    remote_oid,
                                    "git-cortex",
                                    "cortex@git-cortex.dev",
                                ) {
                                    merge_msg = format!(
                                        "Rebased/merged remote ledger commit {}.",
                                        rebase_oid
                                    );
                                    let extra = CortexLedger::reindex_into_db(&repo.repo, db)?;
                                    reindexed += extra;
                                }
                            }
                        }
                        last_push_err = stderr;
                        continue;
                    } else {
                        return Err(CortexError::Git(format!(
                            "Failed to push ledger ref to {}: {}",
                            remote_name,
                            stderr.trim()
                        )));
                    }
                }
            }
            Err(e) => {
                return Err(CortexError::Git(format!("Could not run git push: {}", e)));
            }
        }
    }

    if push_msg.is_empty() {
        return Err(CortexError::Git(format!(
            "Failed to push ledger ref to {} after {} attempts: {}",
            remote_name,
            MAX_SYNC_RETRIES,
            last_push_err.trim()
        )));
    }

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
