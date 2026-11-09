use crate::db::{BatchRecordItem, CortexDb};
use cortex_core::error::{CortexError, Result};
use cortex_core::jcs::sha256_digest;
use cortex_core::schema::{InvariantContract, TombstoneRecord};
use git2::{Commit, Oid, Repository, Signature, Tree};

pub const LEDGER_REF: &str = "refs/cortex/ledger";

pub struct LedgerRecord {
    pub id: String,
    pub kind: String, // "tombstones", "invariants", "events"
    pub json: String,
}

impl LedgerRecord {
    pub fn shard(&self) -> String {
        // Shard on the trailing 2 characters of the ID to distribute uniformly across 1,024 shards
        if self.id.len() >= 2 {
            self.id[self.id.len() - 2..].to_ascii_lowercase()
        } else {
            "00".to_string()
        }
    }

    pub fn tree_path(&self) -> String {
        format!("v1/{}/{}/{}.json", self.kind, self.shard(), self.id)
    }
}

pub struct CortexLedger;

impl CortexLedger {
    /// Get the current HEAD commit of refs/cortex/ledger, if it exists
    pub fn current_commit<'a>(repo: &'a Repository) -> Result<Option<Commit<'a>>> {
        match repo.find_reference(LEDGER_REF) {
            Ok(reference) => {
                let target = reference.target().ok_or_else(|| {
                    CortexError::Git(format!("Reference {} has no target", LEDGER_REF))
                })?;
                let commit = repo.find_commit(target).map_err(|e| {
                    CortexError::Git(format!("Failed to find commit {}: {}", target, e))
                })?;
                Ok(Some(commit))
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(CortexError::Git(format!(
                "Failed to lookup {}: {}",
                LEDGER_REF, e
            ))),
        }
    }

    /// Append records to refs/cortex/ledger with CAS conflict retry
    pub fn append_records(
        repo: &Repository,
        records: &[LedgerRecord],
        author_name: &str,
        author_email: &str,
    ) -> Result<Oid> {
        if records.is_empty() {
            return Err(CortexError::Other("No records to append".to_string()));
        }

        const MAX_RETRIES: usize = 5;
        let mut last_err = None;

        for attempt in 0..MAX_RETRIES {
            let parent_commit = Self::current_commit(repo)?;
            let mut builder = match &parent_commit {
                Some(parent) => {
                    let tree = parent.tree().map_err(|e| {
                        CortexError::Git(format!("Failed to get parent tree: {}", e))
                    })?;
                    repo.treebuilder(Some(&tree))
                }
                None => repo.treebuilder(None),
            }
            .map_err(|e| CortexError::Git(format!("Failed to create treebuilder: {}", e)))?;

            // Helper to insert nested paths into git tree
            for record in records {
                let blob_oid = repo.blob(record.json.as_bytes()).map_err(|e| {
                    CortexError::Git(format!("Failed to create blob for {}: {}", record.id, e))
                })?;

                let path = record.tree_path();
                Self::insert_entry_at_path(repo, &mut builder, &path, blob_oid)?;
            }

            let tree_oid = builder
                .write()
                .map_err(|e| CortexError::Git(format!("Failed to write tree: {}", e)))?;
            let tree = repo.find_tree(tree_oid).map_err(|e| {
                CortexError::Git(format!("Failed to find tree {}: {}", tree_oid, e))
            })?;

            let sig = Signature::now(author_name, author_email)
                .map_err(|e| CortexError::Git(format!("Failed to create signature: {}", e)))?;

            let message = format!("cortex: append {} records to ledger", records.len());

            let commit_res = match &parent_commit {
                Some(parent) => {
                    repo.commit(Some(LEDGER_REF), &sig, &sig, &message, &tree, &[parent])
                }
                None => repo.commit(Some(LEDGER_REF), &sig, &sig, &message, &tree, &[]),
            };

            match commit_res {
                Ok(commit_oid) => return Ok(commit_oid),
                Err(e) => {
                    let is_conflict = e.code() == git2::ErrorCode::Modified
                        || e.message().contains("changed while committing");
                    if is_conflict && attempt + 1 < MAX_RETRIES {
                        std::thread::sleep(std::time::Duration::from_millis(
                            10 * (attempt as u64 + 1),
                        ));
                        last_err = Some(e);
                        continue;
                    }
                    return Err(CortexError::Git(format!(
                        "Failed to create ledger commit: {}",
                        e
                    )));
                }
            }
        }

        Err(CortexError::Git(format!(
            "Failed to append to ledger after {} attempts due to concurrent updates: {:?}",
            MAX_RETRIES, last_err
        )))
    }

    /// Re-index all records from refs/cortex/ledger tree into local SQLite state.db
    pub fn reindex_into_db(repo: &Repository, db: &mut CortexDb) -> Result<usize> {
        let commit = match Self::current_commit(repo)? {
            Some(c) => c,
            None => return Ok(0),
        };

        let tree = commit
            .tree()
            .map_err(|e| CortexError::Git(format!("Failed to get ledger tree: {}", e)))?;

        let mut batch = Vec::with_capacity(500);
        let mut indexed_count = 0;
        walk_tree_and_index(repo, &tree, "", db, &mut batch, &mut indexed_count)?;

        if !batch.is_empty() {
            db.save_records_batch(&batch)?;
        }

        Ok(indexed_count)
    }

    /// Merge a remote ledger commit into local ledger using a conflict-free set-union of trees.
    /// Returns the merge commit Oid, or None if already up-to-date.
    pub fn merge_commits(
        repo: &Repository,
        remote_commit_oid: Oid,
        author_name: &str,
        author_email: &str,
    ) -> Result<Option<Oid>> {
        let local_commit = match Self::current_commit(repo)? {
            Some(c) => c,
            None => {
                repo.reference(
                    LEDGER_REF,
                    remote_commit_oid,
                    true,
                    "cortex: fast-forward to remote ledger",
                )
                .map_err(|e| CortexError::Git(e.to_string()))?;
                return Ok(Some(remote_commit_oid));
            }
        };

        if local_commit.id() == remote_commit_oid {
            return Ok(None);
        }

        if repo
            .graph_descendant_of(local_commit.id(), remote_commit_oid)
            .unwrap_or(false)
        {
            return Ok(None);
        }

        if repo
            .graph_descendant_of(remote_commit_oid, local_commit.id())
            .unwrap_or(false)
        {
            repo.reference(
                LEDGER_REF,
                remote_commit_oid,
                true,
                "cortex: fast-forward to remote ledger",
            )
            .map_err(|e| CortexError::Git(e.to_string()))?;
            return Ok(Some(remote_commit_oid));
        }

        let remote_commit = repo
            .find_commit(remote_commit_oid)
            .map_err(|e| CortexError::Git(format!("Failed to find remote commit: {}", e)))?;

        let local_tree = local_commit
            .tree()
            .map_err(|e| CortexError::Git(e.to_string()))?;
        let remote_tree = remote_commit
            .tree()
            .map_err(|e| CortexError::Git(e.to_string()))?;

        let mut local_map = std::collections::HashMap::new();
        collect_tree_blobs(repo, &local_tree, "", &mut local_map)?;

        let mut remote_map = std::collections::HashMap::new();
        collect_tree_blobs(repo, &remote_tree, "", &mut remote_map)?;

        let mut builder = repo
            .treebuilder(Some(&local_tree))
            .map_err(|e| CortexError::Git(e.to_string()))?;

        for (path, remote_oid) in remote_map {
            if let Some(&local_oid) = local_map.get(&path) {
                if local_oid != remote_oid {
                    return Err(CortexError::Other(format!(
                        "Tamper alert: conflicting content detected for record path '{}' (local {} vs remote {})",
                        path, local_oid, remote_oid
                    )));
                }
            } else {
                Self::insert_entry_at_path(repo, &mut builder, &path, remote_oid)?;
            }
        }

        let union_tree_oid = builder
            .write()
            .map_err(|e| CortexError::Git(e.to_string()))?;
        let union_tree = repo
            .find_tree(union_tree_oid)
            .map_err(|e| CortexError::Git(e.to_string()))?;

        let sig = Signature::now(author_name, author_email)
            .map_err(|e| CortexError::Git(e.to_string()))?;

        let merge_msg = format!(
            "cortex: merge remote ledger {} into local {}",
            remote_commit_oid,
            local_commit.id()
        );

        let merge_commit_oid = repo
            .commit(
                Some(LEDGER_REF),
                &sig,
                &sig,
                &merge_msg,
                &union_tree,
                &[&local_commit, &remote_commit],
            )
            .map_err(|e| CortexError::Git(format!("Failed to create merge commit: {}", e)))?;

        Ok(Some(merge_commit_oid))
    }

    pub fn insert_entry_at_path(
        repo: &Repository,
        root_builder: &mut git2::TreeBuilder,
        full_path: &str,
        blob_oid: Oid,
    ) -> Result<()> {
        let segments: Vec<&str> = full_path.split('/').collect();
        if segments.is_empty() {
            return Ok(());
        }
        insert_recursive(repo, root_builder, &segments, blob_oid)
    }
}

fn insert_recursive(
    repo: &Repository,
    builder: &mut git2::TreeBuilder,
    segments: &[&str],
    blob_oid: Oid,
) -> Result<()> {
    if segments.is_empty() {
        return Ok(());
    }
    if segments.len() == 1 {
        builder
            .insert(segments[0], blob_oid, 0o100644)
            .map_err(|e| CortexError::Git(format!("Failed to insert blob into tree: {}", e)))?;
        return Ok(());
    }

    let existing_tree = match builder.get(segments[0]) {
        Ok(Some(entry)) if entry.kind() == Some(git2::ObjectType::Tree) => {
            repo.find_tree(entry.id()).ok()
        }
        _ => None,
    };

    let mut sub_builder = match existing_tree {
        Some(ref tree) => repo.treebuilder(Some(tree)),
        None => repo.treebuilder(None),
    }
    .map_err(|e| CortexError::Git(format!("Failed to create sub treebuilder: {}", e)))?;

    insert_recursive(repo, &mut sub_builder, &segments[1..], blob_oid)?;

    let sub_oid = sub_builder
        .write()
        .map_err(|e| CortexError::Git(format!("Failed to write sub tree: {}", e)))?;

    builder
        .insert(segments[0], sub_oid, 0o040000)
        .map_err(|e| CortexError::Git(format!("Failed to insert sub tree: {}", e)))?;

    Ok(())
}

fn collect_tree_blobs(
    repo: &Repository,
    tree: &Tree,
    prefix: &str,
    out: &mut std::collections::HashMap<String, Oid>,
) -> Result<()> {
    for entry in tree.iter() {
        let name = entry
            .name()
            .ok_or_else(|| CortexError::Git("Invalid UTF-8 name".to_string()))?;
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", prefix, name)
        };

        if entry.kind() == Some(git2::ObjectType::Tree) {
            let sub = repo
                .find_tree(entry.id())
                .map_err(|e| CortexError::Git(e.to_string()))?;
            collect_tree_blobs(repo, &sub, &path, out)?;
        } else if entry.kind() == Some(git2::ObjectType::Blob) {
            out.insert(path, entry.id());
        }
    }
    Ok(())
}

fn walk_tree_and_index(
    repo: &Repository,
    tree: &Tree,
    prefix: &str,
    db: &mut CortexDb,
    batch: &mut Vec<BatchRecordItem>,
    count: &mut usize,
) -> Result<()> {
    for entry in tree.iter() {
        let name = entry
            .name()
            .ok_or_else(|| CortexError::Git("Invalid UTF-8 in tree entry name".to_string()))?;

        let entry_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", prefix, name)
        };

        if entry.kind() == Some(git2::ObjectType::Tree) {
            let subtree = repo
                .find_tree(entry.id())
                .map_err(|e| CortexError::Git(e.to_string()))?;
            walk_tree_and_index(repo, &subtree, &entry_path, db, batch, count)?;
        } else if entry.kind() == Some(git2::ObjectType::Blob) && name.ends_with(".json") {
            let blob = repo
                .find_blob(entry.id())
                .map_err(|e| CortexError::Git(e.to_string()))?;
            let content = std::str::from_utf8(blob.content())
                .map_err(|e| CortexError::Git(format!("Invalid UTF-8 in record: {}", e)))?;

            let id = name.trim_end_matches(".json");
            let content_hash = sha256_digest(blob.content());

            let kind = if entry_path.contains("/tombstones/") {
                "TOMBSTONE"
            } else if entry_path.contains("/invariants/") {
                "INVARIANT"
            } else {
                "EVENT"
            };

            let mut branch = "main".to_string();
            let mut created_at = chrono::Utc::now().to_rfc3339();

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(b) = v.get("branch_context").and_then(|x| x.as_str()) {
                    branch = b.to_string();
                }
                if let Some(c) = v.get("created_at").and_then(|x| x.as_str()) {
                    created_at = c.to_string();
                }
            }

            let mut symbol_refs = Vec::new();
            if kind == "TOMBSTONE" {
                if let Ok(record) = serde_json::from_str::<TombstoneRecord>(content) {
                    for ts in &record.target_symbols {
                        let path = ts.symbol_uri.split('#').next().unwrap_or("").to_string();
                        symbol_refs.push((
                            ts.symbol_uri.clone(),
                            path,
                            ts.ast_hash.clone(),
                            "TARGET".to_string(),
                        ));
                    }
                }
            } else if kind == "INVARIANT" {
                if let Ok(contract) = serde_json::from_str::<InvariantContract>(content) {
                    let guarded_path = contract
                        .guarded_symbol
                        .symbol_uri
                        .split('#')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    symbol_refs.push((
                        contract.guarded_symbol.symbol_uri.clone(),
                        guarded_path,
                        contract.guarded_symbol.ast_hash.clone(),
                        "GUARDED".to_string(),
                    ));
                    for dep in &contract.upstream_dependencies {
                        let dep_path = dep.symbol_uri.split('#').next().unwrap_or("").to_string();
                        symbol_refs.push((
                            dep.symbol_uri.clone(),
                            dep_path,
                            dep.expected_ast_hash.clone(),
                            "UPSTREAM_DEP".to_string(),
                        ));
                    }
                }
            }

            batch.push(BatchRecordItem {
                id: id.to_string(),
                kind: kind.to_string(),
                json: content.to_string(),
                origin: "LEDGER".to_string(),
                promoted: true,
                branch,
                created_at,
                content_hash,
                symbol_refs,
            });

            *count += 1;

            if batch.len() >= 500 {
                db.save_records_batch(batch)?;
                batch.clear();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, Repository) {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        (temp, repo)
    }

    #[test]
    fn test_nested_tree_preservation() {
        let (_temp, repo) = create_test_repo();

        let rec1 = LedgerRecord {
            id: "tmb_000000000000000000000000ab".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"tmb_1"}"#.to_string(),
        };
        let rec2 = LedgerRecord {
            id: "tmb_000000000000000000000000cd".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"tmb_2"}"#.to_string(),
        };
        let rec3 = LedgerRecord {
            id: "inv_000000000000000000000000ef".to_string(),
            kind: "invariants".to_string(),
            json: r#"{"id":"inv_3"}"#.to_string(),
        };

        // Append 1
        CortexLedger::append_records(&repo, &[rec1], "Test", "test@test.com").unwrap();
        // Append 2 (different shard under tombstones)
        CortexLedger::append_records(&repo, &[rec2], "Test", "test@test.com").unwrap();
        // Append 3 (different kind: invariants)
        CortexLedger::append_records(&repo, &[rec3], "Test", "test@test.com").unwrap();

        let commit = CortexLedger::current_commit(&repo).unwrap().unwrap();
        let tree = commit.tree().unwrap();

        let mut blobs = std::collections::HashMap::new();
        collect_tree_blobs(&repo, &tree, "", &mut blobs).unwrap();

        assert_eq!(blobs.len(), 3);
        assert!(blobs.contains_key("v1/tombstones/ab/tmb_000000000000000000000000ab.json"));
        assert!(blobs.contains_key("v1/tombstones/cd/tmb_000000000000000000000000cd.json"));
        assert!(blobs.contains_key("v1/invariants/ef/inv_000000000000000000000000ef.json"));
    }

    #[test]
    fn test_set_union_merge_and_tamper_detection() {
        let (_temp, repo) = create_test_repo();

        // Commit base
        let base_rec = LedgerRecord {
            id: "tmb_01000000000000000000001".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"base"}"#.to_string(),
        };
        let base_oid =
            CortexLedger::append_records(&repo, &[base_rec], "Test", "test@test.com").unwrap();

        // Branch A
        let rec_a = LedgerRecord {
            id: "tmb_02000000000000000000002".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"branch_a"}"#.to_string(),
        };
        let oid_a = CortexLedger::append_records(&repo, &[rec_a], "Test", "test@test.com").unwrap();

        // Reset ledger ref back to base, then commit Branch B
        repo.reference(LEDGER_REF, base_oid, true, "reset").unwrap();
        let rec_b = LedgerRecord {
            id: "tmb_03000000000000000000003".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"branch_b"}"#.to_string(),
        };
        let oid_b = CortexLedger::append_records(&repo, &[rec_b], "Test", "test@test.com").unwrap();

        // Merge branch A into branch B
        let merge_oid = CortexLedger::merge_commits(&repo, oid_a, "Test", "test@test.com")
            .unwrap()
            .unwrap();

        let merge_commit = repo.find_commit(merge_oid).unwrap();
        assert_eq!(merge_commit.parent_count(), 2);
        assert_eq!(merge_commit.parent_id(0).unwrap(), oid_b);
        assert_eq!(merge_commit.parent_id(1).unwrap(), oid_a);

        let tree = merge_commit.tree().unwrap();
        let mut blobs = std::collections::HashMap::new();
        collect_tree_blobs(&repo, &tree, "", &mut blobs).unwrap();

        assert_eq!(blobs.len(), 3);
        assert!(blobs.contains_key("v1/tombstones/01/tmb_01000000000000000000001.json"));
        assert!(blobs.contains_key("v1/tombstones/02/tmb_02000000000000000000002.json"));
        assert!(blobs.contains_key("v1/tombstones/03/tmb_03000000000000000000003.json"));

        // Now test tamper detection: attempt to merge a commit with same ID but different content
        let tamper_rec = LedgerRecord {
            id: "tmb_01000000000000000000001".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{"id":"tampered_content"}"#.to_string(),
        };
        repo.reference(LEDGER_REF, base_oid, true, "reset").unwrap();
        let tamper_oid =
            CortexLedger::append_records(&repo, &[tamper_rec], "Test", "test@test.com").unwrap();

        // Reset to merge_oid and attempt to merge tamper_oid
        repo.reference(LEDGER_REF, merge_oid, true, "reset")
            .unwrap();
        let err =
            CortexLedger::merge_commits(&repo, tamper_oid, "Test", "test@test.com").unwrap_err();
        assert!(err.to_string().contains("Tamper alert"));
    }

    #[test]
    fn test_reindex_preserves_branch_and_timestamp() {
        let (temp, repo) = create_test_repo();
        let db_path = temp.path().join("state.db");
        let mut db = CortexDb::open(&db_path).unwrap();

        let rec = LedgerRecord {
            id: "tmb_00000000000000000000000099".to_string(),
            kind: "tombstones".to_string(),
            json: r#"{
                "$schema": "https://git-cortex.dev/schemas/v0.1/tombstone.json",
                "id": "tmb_00000000000000000000000099",
                "schema_version": "0.1.0",
                "created_at": "2026-01-01T00:00:00Z",
                "author_harness": "test",
                "branch_context": "feature/branch-x",
                "base_commit_sha": "0000",
                "capture_mode": "MANUAL",
                "target_symbols": [],
                "attempted_approach": "test",
                "failure_reason": "test",
                "verification_evidence": {"command": "test", "exit_code": 1, "error_signature": "err"},
                "invalidation_policy": {"invalidate_on_any_ast_change": []}
            }"#.to_string(),
        };

        CortexLedger::append_records(&repo, &[rec], "Author", "author@test.com").unwrap();

        let count = CortexLedger::reindex_into_db(&repo, &mut db).unwrap();
        assert_eq!(count, 1);

        let stored = db
            .get_record("tmb_00000000000000000000000099")
            .unwrap()
            .unwrap();
        assert_eq!(stored.branch, "feature/branch-x");
        assert_eq!(stored.created_at, "2026-01-01T00:00:00Z");
    }
}
