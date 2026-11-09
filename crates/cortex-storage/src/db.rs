use crate::migrations::apply_migrations;
use cortex_core::error::{CortexError, Result};
use cortex_core::schema::{ActiveHypothesis, CallGraphEdge};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub struct CortexDb {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRecord {
    pub id: String,
    pub kind: String,
    pub json: String,
    pub origin: String,
    pub promoted: bool,
    pub branch: String,
    pub created_at: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCandidate {
    pub id: String,
    pub state: String,
    pub base_commit: String,
    pub symbols_json: String,
    pub diff: String,
    pub exec_ids: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredExec {
    pub id: String,
    pub cmd: String,
    pub exit_code: i32,
    pub signature: String,
    pub output_tail: String,
    pub tree_hash: String,
    pub started_at: String,
    pub duration_ms: i64,
}

impl CortexDb {
    pub fn open(db_path: &Path) -> Result<Self> {
        let mut conn = Connection::open(db_path)
            .map_err(|e| CortexError::Database(format!("Failed to open state.db: {}", e)))?;

        apply_migrations(&mut conn)?;

        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()
            .map_err(|e| CortexError::Database(format!("Failed to open memory db: {}", e)))?;

        apply_migrations(&mut conn)?;

        Ok(Self { conn })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_record(
        &mut self,
        id: &str,
        kind: &str,
        json: &str,
        origin: &str,
        promoted: bool,
        branch: &str,
        content_hash: &str,
    ) -> Result<()> {
        let created_at = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
                INSERT INTO records (id, kind, json, origin, promoted, branch, created_at, content_hash)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(id) DO UPDATE SET
                    json = excluded.json,
                    origin = excluded.origin,
                    promoted = excluded.promoted,
                    branch = excluded.branch,
                    content_hash = excluded.content_hash
                "#,
                params![
                    id,
                    kind,
                    json,
                    origin,
                    if promoted { 1 } else { 0 },
                    branch,
                    created_at,
                    content_hash
                ],
            )
            .map_err(|e| CortexError::Database(format!("Failed to insert record: {}", e)))?;

        Ok(())
    }

    pub fn index_record_symbols(
        &mut self,
        record_id: &str,
        symbols: &[(&str, &str, &str, &str)],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| CortexError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM record_symbols WHERE record_id = ?1",
            params![record_id],
        )
        .map_err(|e| CortexError::Database(e.to_string()))?;

        for (symbol_uri, path, ast_hash, role) in symbols {
            tx.execute(
                r#"
                INSERT INTO record_symbols (record_id, symbol_uri, path, ast_hash, role)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![record_id, symbol_uri, path, ast_hash, role],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        }

        tx.commit()
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn query_records_by_path(&self, path: &str) -> Result<Vec<StoredRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT DISTINCT r.id, r.kind, r.json, r.origin, r.promoted, r.branch, r.created_at, r.content_hash
                FROM records r
                JOIN record_symbols rs ON r.id = rs.record_id
                WHERE rs.path = ?1
                ORDER BY r.created_at DESC
                "#,
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![path], |row| {
                Ok(StoredRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    json: row.get(2)?,
                    origin: row.get(3)?,
                    promoted: row.get::<_, i32>(4)? == 1,
                    branch: row.get(5)?,
                    created_at: row.get(6)?,
                    content_hash: row.get(7)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn query_records_by_symbol(&self, symbol_uri: &str) -> Result<Vec<StoredRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT DISTINCT r.id, r.kind, r.json, r.origin, r.promoted, r.branch, r.created_at, r.content_hash
                FROM records r
                JOIN record_symbols rs ON r.id = rs.record_id
                WHERE rs.symbol_uri = ?1
                ORDER BY r.created_at DESC
                "#,
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![symbol_uri], |row| {
                Ok(StoredRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    json: row.get(2)?,
                    origin: row.get(3)?,
                    promoted: row.get::<_, i32>(4)? == 1,
                    branch: row.get(5)?,
                    created_at: row.get(6)?,
                    content_hash: row.get(7)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_record(&self, id: &str) -> Result<Option<StoredRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, json, origin, promoted, branch, created_at, content_hash FROM records WHERE id = ?1",
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(StoredRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    json: row.get(2)?,
                    origin: row.get(3)?,
                    promoted: row.get::<_, i32>(4)? == 1,
                    branch: row.get(5)?,
                    created_at: row.get(6)?,
                    content_hash: row.get(7)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        if let Some(first) = rows.next() {
            Ok(Some(
                first.map_err(|e| CortexError::Database(e.to_string()))?,
            ))
        } else {
            Ok(None)
        }
    }

    pub fn list_records_by_kind(&self, kind: &str) -> Result<Vec<StoredRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, json, origin, promoted, branch, created_at, content_hash FROM records WHERE kind = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![kind], |row| {
                Ok(StoredRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    json: row.get(2)?,
                    origin: row.get(3)?,
                    promoted: row.get::<_, i32>(4)? == 1,
                    branch: row.get(5)?,
                    created_at: row.get(6)?,
                    content_hash: row.get(7)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn record_file_read(&mut self, path: &str, blob_oid: &str, branch: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
                INSERT INTO file_reads (path, blob_oid, branch, read_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(path, branch) DO UPDATE SET
                    blob_oid = excluded.blob_oid,
                    read_at = excluded.read_at
                "#,
                params![path, blob_oid, branch, now],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_file_reads(&self, branch: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, blob_oid FROM file_reads WHERE branch = ?1")
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![branch], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(list)
    }

    pub fn save_call_graph_edge(&mut self, edge: &CallGraphEdge, branch: &str) -> Result<()> {
        self.conn
            .execute(
                r#"
                INSERT INTO call_graph_edges (caller, callee, relationship, verified_ast_hash, branch)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(caller, callee, branch) DO UPDATE SET
                    relationship = excluded.relationship,
                    verified_ast_hash = excluded.verified_ast_hash
                "#,
                params![
                    edge.caller,
                    edge.callee,
                    edge.relationship,
                    edge.verified_ast_hash,
                    branch
                ],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_call_graph_edges(&self, branch: &str) -> Result<Vec<CallGraphEdge>> {
        let mut stmt = self
            .conn
            .prepare("SELECT caller, callee, relationship, verified_ast_hash FROM call_graph_edges WHERE branch = ?1")
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![branch], |row| {
                Ok(CallGraphEdge {
                    caller: row.get(0)?,
                    callee: row.get(1)?,
                    relationship: row.get(2)?,
                    verified_ast_hash: row.get(3)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut edges = Vec::new();
        for r in rows {
            edges.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(edges)
    }

    pub fn save_hypothesis(&mut self, hyp: &ActiveHypothesis, branch: &str) -> Result<()> {
        self.conn
            .execute(
                r#"
                INSERT INTO hypotheses (hypothesis_id, statement, confidence, status, next_verification_step, branch)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(hypothesis_id, branch) DO UPDATE SET
                    statement = excluded.statement,
                    confidence = excluded.confidence,
                    status = excluded.status,
                    next_verification_step = excluded.next_verification_step
                "#,
                params![
                    hyp.hypothesis_id,
                    hyp.statement,
                    hyp.confidence,
                    hyp.status,
                    hyp.next_verification_step,
                    branch
                ],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_hypotheses(&self, branch: &str) -> Result<Vec<ActiveHypothesis>> {
        let mut stmt = self
            .conn
            .prepare("SELECT hypothesis_id, statement, confidence, status, next_verification_step FROM hypotheses WHERE branch = ?1")
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![branch], |row| {
                Ok(ActiveHypothesis {
                    hypothesis_id: row.get(0)?,
                    statement: row.get(1)?,
                    confidence: row.get(2)?,
                    status: row.get(3)?,
                    next_verification_step: row.get(4)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut hyps = Vec::new();
        for r in rows {
            hyps.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(hyps)
    }

    pub fn insert_exec(&mut self, exec: &StoredExec) -> Result<()> {
        self.conn
            .execute(
                r#"
                INSERT INTO execs (id, cmd, exit_code, signature, output_tail, tree_hash, started_at, duration_ms)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    exec.id,
                    exec.cmd,
                    exec.exit_code,
                    exec.signature,
                    exec.output_tail,
                    exec.tree_hash,
                    exec.started_at,
                    exec.duration_ms
                ],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn save_candidate(&mut self, candidate: &StoredCandidate) -> Result<()> {
        self.conn
            .execute(
                r#"
                INSERT INTO candidates (id, state, base_commit, symbols_json, diff, exec_ids, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(id) DO UPDATE SET
                    state = excluded.state,
                    diff = excluded.diff,
                    exec_ids = excluded.exec_ids
                "#,
                params![
                    candidate.id,
                    candidate.state,
                    candidate.base_commit,
                    candidate.symbols_json,
                    candidate.diff,
                    candidate.exec_ids,
                    candidate.created_at
                ],
            )
            .map_err(|e| CortexError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_candidates_by_state(&self, state: &str) -> Result<Vec<StoredCandidate>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, state, base_commit, symbols_json, diff, exec_ids, created_at FROM candidates WHERE state = ?1")
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![state], |row| {
                Ok(StoredCandidate {
                    id: row.get(0)?,
                    state: row.get(1)?,
                    base_commit: row.get(2)?,
                    symbols_json: row.get(3)?,
                    diff: row.get(4)?,
                    exec_ids: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| CortexError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_exec(&self, id: &str) -> Result<Option<StoredExec>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, cmd, exit_code, signature, output_tail, tree_hash, started_at, duration_ms FROM execs WHERE id = ?1")
            .map_err(|e| CortexError::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(StoredExec {
                    id: row.get(0)?,
                    cmd: row.get(1)?,
                    exit_code: row.get(2)?,
                    signature: row.get(3)?,
                    output_tail: row.get(4)?,
                    tree_hash: row.get(5)?,
                    started_at: row.get(6)?,
                    duration_ms: row.get(7)?,
                })
            })
            .map_err(|e| CortexError::Database(e.to_string()))?;

        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(|e| CortexError::Database(e.to_string()))?))
        } else {
            Ok(None)
        }
    }
}
