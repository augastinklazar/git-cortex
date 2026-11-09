use crate::candidate::{reconcile_candidates, TouchedSymbol};
use crate::signature::extract_error_signature;
use cortex_ast::extract_symbols;
use cortex_core::error::{CortexError, Result};
use cortex_core::id::{IdPrefix, PrefixedId};
use cortex_storage::{CortexDb, CortexRepo, StoredCandidate, StoredExec};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

pub fn safe_tail_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

pub fn safe_truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn execute_command(
    repo: &CortexRepo,
    db: &mut CortexDb,
    cmd_args: &[String],
    timeout_secs: Option<u64>,
) -> Result<i32> {
    if cmd_args.is_empty() {
        return Err(CortexError::Other("Empty command specified".to_string()));
    }

    // 1. Snapshot dirty symbols vs HEAD before running command
    let touched_symbols = find_dirty_symbols(repo)?;
    let base_sha = repo.head_commit_sha()?;
    let dirty_tree_hash = repo.working_tree_dirty_hash()?;

    // 2. Execute process
    let start_time = Instant::now();
    let started_iso = chrono::Utc::now().to_rfc3339();

    let prog = &cmd_args[0];
    let args = &cmd_args[1..];

    let mut child = Command::new(prog)
        .args(args)
        .current_dir(&repo.workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CortexError::Other(format!("Failed to spawn command '{}': {}", prog, e)))?;

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| CortexError::Other("Failed to capture child stdout".to_string()))?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| CortexError::Other("Failed to capture child stderr".to_string()))?;

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });

    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let mut timed_out = false;
    let status = if let Some(limit) = timeout_secs
        .filter(|&t| t > 0)
        .map(std::time::Duration::from_secs)
    {
        loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) => {
                    if start_time.elapsed() >= limit {
                        let _ = child.kill();
                        timed_out = true;
                        break child
                            .wait()
                            .map_err(|e| CortexError::Other(e.to_string()))?;
                    }
                    thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => return Err(CortexError::Other(e.to_string())),
            }
        }
    } else {
        child
            .wait()
            .map_err(|e| CortexError::Other(e.to_string()))?
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    let duration_ms = start_time.elapsed().as_millis() as i64;
    let exit_code = if timed_out {
        124
    } else {
        status.code().unwrap_or(1)
    };

    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    let stderr_str = String::from_utf8_lossy(&stderr_bytes);

    // Forward output to terminal
    print!("{}", stdout_str);
    eprint!("{}", stderr_str);

    let full_output = format!("{}\n{}", stdout_str, stderr_str);
    let signature = if exit_code != 0 {
        extract_error_signature(&full_output)
    } else {
        "SUCCESS".to_string()
    };

    let tail_bytes = safe_tail_utf8(&full_output, 4096);

    let exec_id = PrefixedId::new(IdPrefix::Checkpoint).to_string();
    let cmd_str = cmd_args.join(" ");

    db.insert_exec(&StoredExec {
        id: exec_id.clone(),
        cmd: cmd_str.clone(),
        exit_code,
        signature: signature.clone(),
        output_tail: tail_bytes,
        tree_hash: dirty_tree_hash,
        started_at: started_iso,
        duration_ms,
    })?;

    // 3. If failed and symbols were touched, verify this wasn't an existing baseline failure
    if exit_code != 0 && !touched_symbols.is_empty() {
        let is_baseline_failure = check_baseline_failure(db, &cmd_str, &signature)?;

        if !is_baseline_failure {
            let cand_id = PrefixedId::new(IdPrefix::Candidate).to_string();
            let sym_json = serde_json::to_string(&touched_symbols).unwrap_or_default();
            let diff_summary = format!("{} symbols modified", touched_symbols.len());

            db.save_candidate(&StoredCandidate {
                id: cand_id,
                state: "FAILED".to_string(),
                base_commit: base_sha,
                symbols_json: sym_json,
                diff: diff_summary,
                exec_ids: exec_id,
                created_at: chrono::Utc::now().to_rfc3339(),
            })?;
        }
    }

    // 4. Run reconcile pass (checks if any previously failed candidates have reverted!)
    let _ = reconcile_candidates(repo, db);

    Ok(exit_code)
}

fn check_baseline_failure(db: &CortexDb, cmd: &str, signature: &str) -> Result<bool> {
    let past_execs = db.get_execs_by_cmd_and_signature(cmd, signature)?;
    for exec in past_execs {
        if exec.tree_hash == "wt:clean" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn find_dirty_symbols(repo: &CortexRepo) -> Result<Vec<TouchedSymbol>> {
    let mut status_opts = git2::StatusOptions::new();
    status_opts.include_untracked(false).include_ignored(false);

    let statuses = repo
        .repo
        .statuses(Some(&mut status_opts))
        .map_err(|e| CortexError::Git(e.to_string()))?;

    let head_tree = repo.repo.head().and_then(|h| h.peel_to_tree()).ok();

    let mut touched = Vec::new();

    for entry in statuses.iter() {
        if let Some(rel_path_str) = entry.path() {
            let rel_path = Path::new(rel_path_str);
            let full_path = repo.workdir.join(rel_path);

            if !full_path.exists() {
                continue;
            }

            let current_src = match std::fs::read(&full_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let current_symbols = extract_symbols(rel_path, &current_src).unwrap_or_default();

            let head_symbols = if let Some(ref tree) = head_tree {
                if let Ok(tree_entry) = tree.get_path(rel_path) {
                    if let Ok(blob) = repo.repo.find_blob(tree_entry.id()) {
                        extract_symbols(rel_path, blob.content()).unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let head_map: HashMap<String, String> = head_symbols
                .into_iter()
                .map(|s| (s.uri, s.ast_hash))
                .collect();

            for curr in current_symbols {
                if let Some(base_hash) = head_map.get(&curr.uri) {
                    if base_hash != &curr.ast_hash {
                        touched.push(TouchedSymbol {
                            symbol_uri: curr.uri,
                            base_ast_hash: base_hash.clone(),
                            dirty_ast_hash: curr.ast_hash,
                        });
                    }
                }
            }
        }
    }

    Ok(touched)
}
