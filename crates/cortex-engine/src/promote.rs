use cortex_core::error::Result;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo, LedgerRecord};

pub fn promote_local_records(repo: &CortexRepo, db: &mut CortexDb) -> Result<usize> {
    let unpromoted = db.list_records_by_kind("TOMBSTONE")?;
    let unpromoted_invariants = db.list_records_by_kind("INVARIANT")?;

    let mut ledger_records = Vec::new();

    for r in unpromoted.iter().chain(unpromoted_invariants.iter()) {
        if !r.promoted {
            let kind_dir = match r.kind.as_str() {
                "TOMBSTONE" => "tombstones",
                "INVARIANT" => "invariants",
                _ => "events",
            };

            ledger_records.push(LedgerRecord {
                id: r.id.clone(),
                kind: kind_dir.to_string(),
                json: r.json.clone(),
            });
        }
    }

    if ledger_records.is_empty() {
        return Ok(0);
    }

    let count = ledger_records.len();
    let author_name = repo
        .repo
        .config()
        .ok()
        .and_then(|c| c.get_string("user.name").ok())
        .unwrap_or_else(|| "git-cortex".to_string());
    let author_email = repo
        .repo
        .config()
        .ok()
        .and_then(|c| c.get_string("user.email").ok())
        .unwrap_or_else(|| "cortex@git-cortex.dev".to_string());

    CortexLedger::append_records(&repo.repo, &ledger_records, &author_name, &author_email)?;

    // Mark as promoted in SQLite
    for r in ledger_records {
        if let Some(mut stored) = db.get_record(&r.id)? {
            stored.promoted = true;
            stored.origin = "LEDGER".to_string();
            db.save_record(
                &stored.id,
                &stored.kind,
                &stored.json,
                "LEDGER",
                true,
                &stored.branch,
                &stored.content_hash,
            )?;
        }
    }

    Ok(count)
}
