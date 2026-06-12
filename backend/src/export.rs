//! Phase 5: render a snapshot to versioned JSON + manifest.
//!
//! Layout under the export dir (mirrors the CDN layout the app fetches):
//!   manifest.json
//!   set{set}/{patch}/{bracket}/{comps,units,unit_items,items,augments,traits}.json
//!
//! manifest.json carries the snapshot identity plus a sha256 per file; the desktop
//! app polls only the manifest and downloads files whose hash changed.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::path::Path;

pub async fn run(pool: &PgPool, snapshot: Option<i64>, out: &Path) -> Result<()> {
    let snap = match snapshot {
        Some(id) => sqlx::query("SELECT * FROM stat_snapshots WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .context("snapshot id not found")?,
        None => sqlx::query(
            "SELECT * FROM stat_snapshots WHERE is_current ORDER BY computed_at DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .context("no current snapshot — aggregate first")?,
    };
    let snapshot_id: i64 = snap.get("id");
    let set_id: i32 = snap.get("set_id");
    let patch: String = snap.get("patch");
    let bracket: String = snap.get("bracket");

    let rel_dir = format!("set{set_id}/{patch}/{bracket}");
    let dir = out.join(&rel_dir);
    std::fs::create_dir_all(&dir)?;

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "comps",
            "SELECT comp_id, name, features, games, avg_place, top4_rate, win_rate, pick_rate
         FROM comp_stats WHERE snapshot_id = $1 ORDER BY avg_place",
            |r| {
                json!({
                    "comp_id": r.get::<String,_>("comp_id"),
                    "name": r.get::<String,_>("name"),
                    "features": r.get::<Value,_>("features"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                    "win_rate": r.get::<f32,_>("win_rate"),
                    "pick_rate": r.get::<f32,_>("pick_rate"),
                })
            },
        )
        .await?,
    );

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "units",
            "SELECT unit, games, avg_place, top4_rate, win_rate, pick_rate, avg_star, by_star
         FROM unit_stats WHERE snapshot_id = $1 ORDER BY avg_place",
            |r| {
                json!({
                    "unit": r.get::<String,_>("unit"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                    "win_rate": r.get::<f32,_>("win_rate"),
                    "pick_rate": r.get::<f32,_>("pick_rate"),
                    "avg_star": r.get::<Option<f32>,_>("avg_star"),
                    "by_star": r.get::<Option<Value>,_>("by_star"),
                })
            },
        )
        .await?,
    );

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "unit_items",
            "SELECT unit, item, games, avg_place, top4_rate
         FROM unit_item_stats WHERE snapshot_id = $1 ORDER BY unit, avg_place",
            |r| {
                json!({
                    "unit": r.get::<String,_>("unit"),
                    "item": r.get::<String,_>("item"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                })
            },
        )
        .await?,
    );

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "items",
            "SELECT item, games, avg_place, top4_rate, win_rate, pick_rate
         FROM item_stats WHERE snapshot_id = $1 ORDER BY avg_place",
            |r| {
                json!({
                    "item": r.get::<String,_>("item"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                    "win_rate": r.get::<f32,_>("win_rate"),
                    "pick_rate": r.get::<f32,_>("pick_rate"),
                })
            },
        )
        .await?,
    );

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "augments",
            "SELECT augment, games, avg_place, top4_rate, win_rate, pick_rate, by_stage
         FROM augment_stats WHERE snapshot_id = $1 ORDER BY avg_place",
            |r| {
                json!({
                    "augment": r.get::<String,_>("augment"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                    "win_rate": r.get::<f32,_>("win_rate"),
                    "pick_rate": r.get::<f32,_>("pick_rate"),
                    "by_stage": r.get::<Option<Value>,_>("by_stage"),
                })
            },
        )
        .await?,
    );

    files.push(
        render(
            pool,
            snapshot_id,
            &rel_dir,
            "traits",
            "SELECT trait, num_units, games, avg_place, top4_rate
         FROM trait_stats WHERE snapshot_id = $1 ORDER BY trait, num_units",
            |r| {
                json!({
                    "trait": r.get::<String,_>("trait"),
                    "num_units": r.get::<i16,_>("num_units"),
                    "games": r.get::<i32,_>("games"),
                    "avg_place": r.get::<f32,_>("avg_place"),
                    "top4_rate": r.get::<f32,_>("top4_rate"),
                })
            },
        )
        .await?,
    );

    let mut manifest_files = serde_json::Map::new();
    for (rel_path, bytes) in &files {
        std::fs::write(out.join(rel_path), bytes)?;
        let hash = hex::encode(Sha256::digest(bytes));
        manifest_files.insert(
            rel_path.clone(),
            json!({"sha256": hash, "bytes": bytes.len()}),
        );
    }

    let manifest = json!({
        "schema_version": 1,
        "snapshot_id": snapshot_id,
        "set": set_id,
        "patch": patch,
        "bracket": bracket,
        "region": snap.get::<String,_>("region"),
        "games_total": snap.get::<i32,_>("games_total"),
        "matches_total": snap.get::<i32,_>("matches_total"),
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "files": Value::Object(manifest_files),
    });
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    sqlx::query("UPDATE stat_snapshots SET published_at = now() WHERE id = $1")
        .bind(snapshot_id)
        .execute(pool)
        .await?;

    tracing::info!(snapshot_id, out = %out.display(), files = files.len() + 1, "export complete");
    Ok(())
}

async fn render(
    pool: &PgPool,
    snapshot_id: i64,
    rel_dir: &str,
    name: &str,
    sql: &str,
    to_json: impl Fn(&sqlx::postgres::PgRow) -> Value,
) -> Result<(String, Vec<u8>)> {
    let rows = sqlx::query(sql).bind(snapshot_id).fetch_all(pool).await?;
    let arr: Vec<Value> = rows.iter().map(to_json).collect();
    Ok((format!("{rel_dir}/{name}.json"), serde_json::to_vec(&arr)?))
}
