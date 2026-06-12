//! Crawl loop: league snapshot -> Master+ set -> per-puuid incremental match ids
//! -> dedup -> raw insert. Works identically against FixtureSource and HttpRiotSource.

use crate::cdragon::parse_patch;
use crate::riot::{platform_to_route, RiotSource};
use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use std::collections::HashSet;

pub async fn run(pool: &PgPool, source: &dyn RiotSource, platforms: &[String]) -> Result<()> {
    let snapshot_at = Utc::now();
    let mut total_new = 0usize;

    for platform in platforms {
        let route = platform_to_route(platform);
        let entries = source.master_plus(platform).await?;
        tracing::info!(platform, count = entries.len(), "master+ league snapshot");

        for e in &entries {
            sqlx::query(
                "INSERT INTO summoners (puuid, platform, game_name, tag_line, last_seen_at)
                 VALUES ($1,$2,$3,$4,now())
                 ON CONFLICT (puuid) DO UPDATE
                 SET platform=$2, game_name=COALESCE($3, summoners.game_name),
                     tag_line=COALESCE($4, summoners.tag_line), last_seen_at=now()",
            )
            .bind(&e.puuid)
            .bind(&e.platform)
            .bind(&e.game_name)
            .bind(&e.tag_line)
            .execute(pool)
            .await?;
            sqlx::query(
                "INSERT INTO ranked_entries (puuid, platform, tier, league_points, snapshot_at)
                 VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
            )
            .bind(&e.puuid)
            .bind(&e.platform)
            .bind(&e.tier)
            .bind(e.league_points)
            .bind(snapshot_at)
            .execute(pool)
            .await?;
        }

        let mut seen_this_run: HashSet<String> = HashSet::new();
        for e in &entries {
            let cursor: Option<i64> = sqlx::query_scalar(
                "SELECT last_match_start_time FROM crawl_state WHERE puuid = $1",
            )
            .bind(&e.puuid)
            .fetch_optional(pool)
            .await?
            .flatten();

            let ids = source.match_ids(route, &e.puuid, cursor).await?;
            let mut max_ts = cursor.unwrap_or(0);
            let mut fetched = 0usize;

            for id in ids {
                if !seen_this_run.insert(id.clone()) {
                    continue;
                }
                let exists: Option<i32> =
                    sqlx::query_scalar("SELECT 1 FROM matches WHERE match_id = $1 LIMIT 1")
                        .bind(&id)
                        .fetch_optional(pool)
                        .await?;
                if exists.is_some() {
                    continue;
                }
                let m = source.match_detail(route, &id).await?;
                if let Some(ts) = ingest_match(pool, route, &id, &m).await? {
                    max_ts = max_ts.max(ts.timestamp());
                    fetched += 1;
                    total_new += 1;
                }
            }

            sqlx::query(
                "INSERT INTO crawl_state (puuid, last_match_start_time, last_crawled_at, matches_seen)
                 VALUES ($1,$2,now(),$3)
                 ON CONFLICT (puuid) DO UPDATE
                 SET last_match_start_time = GREATEST(crawl_state.last_match_start_time, $2),
                     last_crawled_at = now(),
                     matches_seen = crawl_state.matches_seen + $3",
            )
            .bind(&e.puuid)
            .bind(if max_ts > 0 { Some(max_ts) } else { None })
            .bind(fetched as i32)
            .execute(pool)
            .await?;
        }
    }

    tracing::info!(total_new, "crawl complete");
    Ok(())
}

/// Insert one raw match; returns its game_datetime if stored.
async fn ingest_match(
    pool: &PgPool,
    route: &str,
    match_id: &str,
    m: &serde_json::Value,
) -> Result<Option<DateTime<Utc>>> {
    let info = &m["info"];
    // only ranked TFT feeds the stats
    let queue_id = info["queue_id"].as_i64().unwrap_or(0) as i32;
    if queue_id != 1100 {
        return Ok(None);
    }
    let game_version = info["game_version"].as_str().unwrap_or_default();
    let patch = parse_patch(game_version).context("no patch in game_version")?;
    let set_id = info["tft_set_number"].as_i64().context("tft_set_number")? as i32;
    let ts_ms = info["game_datetime"].as_i64().context("game_datetime")?;
    let game_datetime = Utc
        .timestamp_millis_opt(ts_ms)
        .single()
        .context("bad game_datetime")?;

    ensure_partition(pool, &patch).await?;

    sqlx::query(
        "INSERT INTO matches (match_id, patch, set_id, region, queue_id, game_datetime, game_length, game_version, data)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (match_id, patch) DO NOTHING",
    )
    .bind(match_id)
    .bind(&patch)
    .bind(set_id)
    .bind(route)
    .bind(queue_id)
    .bind(game_datetime)
    .bind(info["game_length"].as_f64().map(|v| v as f32))
    .bind(game_version)
    .bind(m)
    .execute(pool)
    .await?;
    Ok(Some(game_datetime))
}

/// One partition per patch so retention = DROP one partition. Must run before the
/// first insert of a new patch, otherwise rows land in matches_default (harmless,
/// but they stay there: list partitions can't be created over existing default rows).
async fn ensure_partition(pool: &PgPool, patch: &str) -> Result<()> {
    if !patch.chars().all(|c| c.is_ascii_digit() || c == '.') {
        anyhow::bail!("unsafe patch string: {patch}");
    }
    let table = format!("matches_p{}", patch.replace('.', "_"));
    let exists: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
        .bind(&table)
        .fetch_one(pool)
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {table} PARTITION OF matches FOR VALUES IN ('{patch}')"
    );
    if let Err(e) = sqlx::query(&sql).execute(pool).await {
        tracing::warn!(patch, error = %e, "partition create failed (rows may exist in default)");
    }
    Ok(())
}
