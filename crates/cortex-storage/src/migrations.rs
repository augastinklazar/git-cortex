use cortex_core::error::{CortexError, Result};
use rusqlite::Connection;

pub fn apply_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA busy_timeout = 5000;
        PRAGMA foreign_keys = ON;
        "#,
    )
    .map_err(|e| CortexError::Database(format!("Failed to set database pragmas: {}", e)))?;

    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| CortexError::Database(format!("Failed to query user_version: {}", e)))?;

    if version < 1 {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| {
                CortexError::Database(format!("Failed to begin migration transaction: {}", e))
            })?;

        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS records (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                json TEXT NOT NULL,
                origin TEXT NOT NULL,
                promoted INTEGER NOT NULL DEFAULT 0,
                branch TEXT NOT NULL,
                created_at TEXT NOT NULL,
                content_hash TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS record_symbols (
                record_id TEXT NOT NULL,
                symbol_uri TEXT NOT NULL,
                path TEXT NOT NULL,
                ast_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                FOREIGN KEY(record_id) REFERENCES records(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_rs_path ON record_symbols(path);
            CREATE INDEX IF NOT EXISTS idx_rs_symbol_uri ON record_symbols(symbol_uri);
            CREATE INDEX IF NOT EXISTS idx_rs_ast_hash ON record_symbols(ast_hash);
            CREATE INDEX IF NOT EXISTS idx_records_kind_branch ON records(kind, branch);
            CREATE INDEX IF NOT EXISTS idx_record_symbols_record_id ON record_symbols(record_id);
            CREATE INDEX IF NOT EXISTS idx_record_symbols_path_role ON record_symbols(path, role);

            CREATE TABLE IF NOT EXISTS symbol_cache (
                path TEXT NOT NULL,
                blob_oid TEXT NOT NULL,
                symbols_json TEXT NOT NULL,
                PRIMARY KEY(path, blob_oid)
            );

            CREATE TABLE IF NOT EXISTS execs (
                id TEXT PRIMARY KEY,
                cmd TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                signature TEXT NOT NULL,
                output_tail TEXT NOT NULL,
                tree_hash TEXT NOT NULL,
                started_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS candidates (
                id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                base_commit TEXT NOT NULL,
                symbols_json TEXT NOT NULL,
                diff TEXT NOT NULL,
                exec_ids TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                branch TEXT NOT NULL,
                snapshot_ref TEXT NOT NULL,
                objective TEXT NOT NULL,
                state_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_reads (
                path TEXT NOT NULL,
                blob_oid TEXT NOT NULL,
                branch TEXT NOT NULL,
                read_at TEXT NOT NULL,
                PRIMARY KEY(path, branch)
            );

            CREATE TABLE IF NOT EXISTS call_graph_edges (
                caller TEXT NOT NULL,
                callee TEXT NOT NULL,
                relationship TEXT NOT NULL,
                verified_ast_hash TEXT NOT NULL,
                branch TEXT NOT NULL,
                PRIMARY KEY(caller, callee, branch)
            );

            CREATE TABLE IF NOT EXISTS hypotheses (
                hypothesis_id TEXT NOT NULL,
                statement TEXT NOT NULL,
                confidence REAL NOT NULL,
                status TEXT NOT NULL,
                next_verification_step TEXT NOT NULL,
                branch TEXT NOT NULL,
                PRIMARY KEY(hypothesis_id, branch)
            );

            PRAGMA user_version = 1;
            "#,
        )
        .map_err(|e| CortexError::Database(format!("Failed to apply migrations: {}", e)))?;

        tx.commit().map_err(|e| {
            CortexError::Database(format!("Failed to commit migration transaction: {}", e))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_idempotency_and_user_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_migrations(&mut conn).unwrap();

        let v1: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v1, 1);

        // Second call should succeed and leave user_version at 1 without re-executing DDL
        apply_migrations(&mut conn).unwrap();
        let v2: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 1);
    }
}
