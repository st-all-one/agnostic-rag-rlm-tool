use anyhow::Result;
use arlm_search::decay::DecayConfig;
use arlm_storage::Storage;

pub struct DecayArgs<'a> {
    pub dry_run: bool,
    pub project: &'a std::path::Path,
    pub format: crate::output::Format,
}

pub fn execute(args: DecayArgs<'_>) -> Result<()> {
    let storage = Storage::open(&crate::util::data_dir())?;

    let conn = storage.conn();
    let conn = conn.lock();

    let mut stmt = conn.prepare(
        "SELECT id,
                COALESCE(last_accessed_at, created_at) as last_access,
                created_at
         FROM chunks",
    )?;

    let config = DecayConfig::default();

    let rows: Vec<(i64, i64, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .filter_map(std::result::Result::ok)
        .collect();

    let total = rows.len();
    let mut decayed = 0;
    let mut kept = 0;

    for (id, last_access, _created_at) in &rows {
        let age = DecayConfig::age_hours(*last_access);
        let score = config.score(1.0, age);
        if score < 0.1 {
            decayed += 1;
            if !args.dry_run {
                conn.execute("DELETE FROM chunks WHERE id = ?1", [*id])?;
            }
        } else {
            kept += 1;
        }
    }

    let result = serde_json::json!({
        "total_chunks": total,
        "decayed": decayed,
        "kept": kept,
        "dry_run": args.dry_run,
    });

    match args.format {
        crate::output::Format::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        _ => {
            println!("Chunks: {total}, Decayed: {decayed}, Kept: {kept} (dry_run: {})", args.dry_run);
        }
    }

    Ok(())
}
