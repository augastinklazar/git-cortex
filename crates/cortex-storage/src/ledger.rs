use crate::db::CortexDb;
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
        // Shard by first 2 chars of the id after prefix
        let parts: Vec<&str> = self.id.splitn(2, '_').collect();
        if parts.len() == 2 && parts[1].len() >= 2 {
            parts[1][..2].to_ascii_lowercase()
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

    /// Append records to refs/cortex/ledger
    pub fn append_records(
        repo: &Repository,
        records: &[LedgerRecord],
        author_name: &str,
        author_email: &str,
    ) -> Result<Oid> {
        if records.is_empty() {
            return Err(CortexError::Other("No records to append".to_string()));
        }

        let parent_commit = Self::current_commit(repo)?;
        let mut builder = match &parent_commit {
            Some(parent) => {
                let tree = parent
                    .tree()
                    .map_err(|e| CortexError::Git(format!("Failed to get parent tree: {}", e)))?;
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
            insert_entry_at_path(repo, &mut builder, &path, blob_oid)?;
        }

        let tree_oid = builder
            .write()
            .map_err(|e| CortexError::Git(format!("Failed to write tree: {}", e)))?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| CortexError::Git(format!("Failed to find tree {}: {}", tree_oid, e)))?;

        let sig = Signature::now(author_name, author_email)
            .map_err(|e| CortexError::Git(format!("Failed to create signature: {}", e)))?;

        let message = format!("cortex: append {} records to ledger", records.len());

        let commit_oid = match &parent_commit {
            Some(parent) => repo.commit(Some(LEDGER_REF), &sig, &sig, &message, &tree, &[parent]),
            None => repo.commit(Some(LEDGER_REF), &sig, &sig, &message, &tree, &[]),
        }
        .map_err(|e| CortexError::Git(format!("Failed to create ledger commit: {}", e)))?;

        Ok(commit_oid)
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

        let mut indexed_count = 0;
        walk_tree_and_index(repo, &tree, "", db, &mut indexed_count)?;

        Ok(indexed_count)
    }
}

fn insert_entry_at_path(
    repo: &Repository,
    root_builder: &mut git2::TreeBuilder,
    full_path: &str,
    blob_oid: Oid,
) -> Result<()> {
    let segments: Vec<&str> = full_path.split('/').collect();
    if segments.is_empty() {
        return Ok(());
    }

    if segments.len() == 1 {
        root_builder
            .insert(segments[0], blob_oid, 0o100644)
            .map_err(|e| CortexError::Git(format!("Failed to insert blob into tree: {}", e)))?;
        return Ok(());
    }

    let subtree_oid = build_nested_subtree(repo, &segments[1..], blob_oid)?;
    root_builder
        .insert(segments[0], subtree_oid, 0o040000)
        .map_err(|e| CortexError::Git(format!("Failed to insert subtree: {}", e)))?;

    Ok(())
}

fn build_nested_subtree(repo: &Repository, segments: &[&str], blob_oid: Oid) -> Result<Oid> {
    if segments.len() == 1 {
        let mut b = repo
            .treebuilder(None)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        b.insert(segments[0], blob_oid, 0o100644)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        b.write().map_err(|e| CortexError::Git(e.to_string()))
    } else {
        let child_oid = build_nested_subtree(repo, &segments[1..], blob_oid)?;
        let mut b = repo
            .treebuilder(None)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        b.insert(segments[0], child_oid, 0o040000)
            .map_err(|e| CortexError::Git(e.to_string()))?;
        b.write().map_err(|e| CortexError::Git(e.to_string()))
    }
}

fn walk_tree_and_index(
    repo: &Repository,
    tree: &Tree,
    prefix: &str,
    db: &mut CortexDb,
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
            walk_tree_and_index(repo, &subtree, &entry_path, db, count)?;
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

            db.save_record(id, kind, content, "LEDGER", true, "main", &content_hash)?;

            if kind == "TOMBSTONE" {
                if let Ok(record) = serde_json::from_str::<TombstoneRecord>(content) {
                    let mut sym_tuples = Vec::new();
                    for ts in &record.target_symbols {
                        let path = ts.symbol_uri.split('#').next().unwrap_or("");
                        sym_tuples.push((
                            ts.symbol_uri.as_str(),
                            path,
                            ts.ast_hash.as_str(),
                            "TARGET",
                        ));
                    }
                    let _ = db.index_record_symbols(id, &sym_tuples);
                }
            } else if kind == "INVARIANT" {
                if let Ok(contract) = serde_json::from_str::<InvariantContract>(content) {
                    let mut sym_tuples = Vec::new();
                    let guarded_path = contract
                        .guarded_symbol
                        .symbol_uri
                        .split('#')
                        .next()
                        .unwrap_or("");
                    sym_tuples.push((
                        contract.guarded_symbol.symbol_uri.as_str(),
                        guarded_path,
                        contract.guarded_symbol.ast_hash.as_str(),
                        "GUARDED",
                    ));
                    for dep in &contract.upstream_dependencies {
                        let dep_path = dep.symbol_uri.split('#').next().unwrap_or("");
                        sym_tuples.push((
                            dep.symbol_uri.as_str(),
                            dep_path,
                            dep.expected_ast_hash.as_str(),
                            "UPSTREAM_DEP",
                        ));
                    }
                    let _ = db.index_record_symbols(id, &sym_tuples);
                }
            }

            *count += 1;
        }
    }
    Ok(())
}
