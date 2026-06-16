//! Phase 4: per-patch rollup (案B: full recompute, no incremental state).
//!
//! Counting rule (案Z): a participant board is counted only if its puuid is in the
//! Master+ set from the latest league snapshot. High-elo lobbies are almost entirely
//! Master+, so this keeps near-full volume at seed-player purity.

use anyhow::Result;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};

pub struct Thresholds {
    pub comp: i64,
    pub pair: i64,
    pub entity: i64,
}

#[derive(Default)]
struct Acc {
    games: i64,
    sum_place: i64,
    top4: i64,
    wins: i64,
}

impl Acc {
    fn add(&mut self, place: i64) {
        self.games += 1;
        self.sum_place += place;
        if place <= 4 {
            self.top4 += 1;
        }
        if place == 1 {
            self.wins += 1;
        }
    }
    fn avg_place(&self) -> f32 {
        self.sum_place as f32 / self.games.max(1) as f32
    }
    fn top4_rate(&self) -> f32 {
        self.top4 as f32 / self.games.max(1) as f32
    }
    fn win_rate(&self) -> f32 {
        self.wins as f32 / self.games.max(1) as f32
    }
}

#[derive(Default)]
struct UnitAcc {
    overall: Acc,
    sum_star: i64,
    by_star: HashMap<i64, Acc>,
}

#[derive(Default)]
struct CompAcc {
    acc: Acc,
    name: String,
    features: Value,
}

pub async fn run(pool: &PgPool, patch: &str, set_id: i32, th: Thresholds) -> Result<()> {
    let master_plus: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT puuid FROM ranked_entries
         WHERE snapshot_at = (SELECT max(snapshot_at) FROM ranked_entries)
           AND tier IN ('MASTER','GRANDMASTER','CHALLENGER')",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    anyhow::ensure!(
        !master_plus.is_empty(),
        "no Master+ snapshot in ranked_entries — crawl first"
    );

    let unit_names: HashMap<String, String> = name_map(pool, "units", set_id).await?;
    let trait_names: HashMap<String, String> = name_map(pool, "traits", set_id).await?;

    let mut boards: i64 = 0;
    let mut matches_total: i64 = 0;
    let mut comps: HashMap<String, CompAcc> = HashMap::new();
    let mut units: HashMap<String, UnitAcc> = HashMap::new();
    let mut unit_items: HashMap<(String, String), Acc> = HashMap::new();
    let mut items: HashMap<String, Acc> = HashMap::new();
    let mut augments: HashMap<String, Acc> = HashMap::new();
    let mut aug_by_stage: HashMap<String, HashMap<&'static str, Acc>> = HashMap::new();
    let mut traits: HashMap<(String, i64), Acc> = HashMap::new();

    // keyset pagination over the patch partition
    let mut last_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT match_id, data FROM matches
             WHERE patch = $1 AND match_id > $2 ORDER BY match_id LIMIT 500",
        )
        .bind(patch)
        .bind(&last_id)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            last_id = row.get::<String, _>("match_id");
            let data: Value = row.get("data");
            matches_total += 1;

            for p in data["info"]["participants"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let Some(puuid) = p["puuid"].as_str() else {
                    continue;
                };
                if !master_plus.contains(puuid) {
                    continue; // 案Z
                }
                let Some(place) = p["placement"].as_i64() else {
                    continue;
                };
                boards += 1;

                let p_units = p["units"].as_array().cloned().unwrap_or_default();
                let p_traits = p["traits"].as_array().cloned().unwrap_or_default();

                // board-level dedupe: two copies of a unit count once, at max star
                let mut best_star: HashMap<&str, i64> = HashMap::new();
                for u in &p_units {
                    let Some(cid) = u["character_id"].as_str() else {
                        continue;
                    };
                    let star = u["tier"].as_i64().unwrap_or(1);
                    best_star
                        .entry(cid)
                        .and_modify(|s| *s = (*s).max(star))
                        .or_insert(star);
                }
                for (cid, star) in &best_star {
                    let ua = units.entry(cid.to_string()).or_default();
                    ua.overall.add(place);
                    ua.sum_star += star;
                    ua.by_star.entry(*star).or_default().add(place);
                }

                let mut board_items: HashSet<&str> = HashSet::new();
                for u in &p_units {
                    let Some(cid) = u["character_id"].as_str() else {
                        continue;
                    };
                    for it in u["itemNames"].as_array().into_iter().flatten() {
                        let Some(item) = it.as_str() else { continue };
                        board_items.insert(item);
                        unit_items
                            .entry((cid.to_string(), item.to_string()))
                            .or_default()
                            .add(place);
                    }
                }
                for item in board_items {
                    items.entry(item.to_string()).or_default().add(place);
                }

                for (idx, a) in p["augments"].as_array().into_iter().flatten().enumerate() {
                    let Some(aug) = a.as_str() else { continue };
                    augments.entry(aug.to_string()).or_default().add(place);
                    let stage = ["2-1", "3-2", "4-2"].get(idx).copied().unwrap_or("other");
                    aug_by_stage
                        .entry(aug.to_string())
                        .or_default()
                        .entry(stage)
                        .or_default()
                        .add(place);
                }

                for t in &p_traits {
                    let (Some(name), Some(n), Some(style)) = (
                        t["name"].as_str(),
                        t["num_units"].as_i64(),
                        t["style"].as_i64(),
                    ) else {
                        continue;
                    };
                    if style > 0 {
                        traits.entry((name.to_string(), n)).or_default().add(place);
                    }
                }

                let (comp_id, comp_name, features) =
                    comp_label(&p_units, &p_traits, &unit_names, &trait_names);
                let ca = comps.entry(comp_id).or_default();
                ca.name = comp_name;
                ca.features = features;
                ca.acc.add(place);
            }
        }
    }
    anyhow::ensure!(
        boards > 0,
        "no counted boards for patch {patch} — nothing to aggregate"
    );
    tracing::info!(matches_total, boards, "scanned");

    // write everything under one snapshot, atomically flip is_current
    let mut tx = pool.begin().await?;
    let snapshot_id: i64 = sqlx::query_scalar(
        "INSERT INTO stat_snapshots (set_id, patch, games_total, matches_total)
         VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(set_id)
    .bind(patch)
    .bind(boards as i32)
    .bind(matches_total as i32)
    .fetch_one(&mut *tx)
    .await?;

    let mut n = 0;
    for (comp_id, c) in comps.iter().filter(|(_, c)| c.acc.games >= th.comp) {
        sqlx::query(
            "INSERT INTO comp_stats (snapshot_id, comp_id, name, features, games, avg_place, top4_rate, win_rate, pick_rate)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(snapshot_id)
        .bind(comp_id)
        .bind(&c.name)
        .bind(&c.features)
        .bind(c.acc.games as i32)
        .bind(c.acc.avg_place())
        .bind(c.acc.top4_rate())
        .bind(c.acc.win_rate())
        .bind(c.acc.games as f32 / boards as f32)
        .execute(&mut *tx)
        .await?;
        n += 1;
    }
    tracing::info!(comps = n, total = comps.len(), "comp_stats written");

    for (unit, ua) in units.iter().filter(|(_, u)| u.overall.games >= th.entity) {
        let by_star: Value = ua
            .by_star
            .iter()
            .map(|(star, a)| {
                (star.to_string(), json!({"games": a.games, "avg_place": a.avg_place(), "top4_rate": a.top4_rate()}))
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        sqlx::query(
            "INSERT INTO unit_stats (snapshot_id, unit, games, avg_place, top4_rate, win_rate, pick_rate, avg_star, by_star)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(snapshot_id)
        .bind(unit)
        .bind(ua.overall.games as i32)
        .bind(ua.overall.avg_place())
        .bind(ua.overall.top4_rate())
        .bind(ua.overall.win_rate())
        .bind(ua.overall.games as f32 / boards as f32)
        .bind(ua.sum_star as f32 / ua.overall.games.max(1) as f32)
        .bind(by_star)
        .execute(&mut *tx)
        .await?;
    }

    for ((unit, item), a) in unit_items.iter().filter(|(_, a)| a.games >= th.pair) {
        sqlx::query(
            "INSERT INTO unit_item_stats (snapshot_id, unit, item, games, avg_place, top4_rate)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(snapshot_id)
        .bind(unit)
        .bind(item)
        .bind(a.games as i32)
        .bind(a.avg_place())
        .bind(a.top4_rate())
        .execute(&mut *tx)
        .await?;
    }

    for (item, a) in items.iter().filter(|(_, a)| a.games >= th.entity) {
        sqlx::query(
            "INSERT INTO item_stats (snapshot_id, item, games, avg_place, top4_rate, win_rate, pick_rate)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(snapshot_id)
        .bind(item)
        .bind(a.games as i32)
        .bind(a.avg_place())
        .bind(a.top4_rate())
        .bind(a.win_rate())
        .bind(a.games as f32 / boards as f32)
        .execute(&mut *tx)
        .await?;
    }

    for (aug, a) in augments.iter().filter(|(_, a)| a.games >= th.entity) {
        let by_stage: Value = aug_by_stage
            .get(aug)
            .map(|m| {
                m.iter()
                    .map(|(stage, a)| {
                        (
                            stage.to_string(),
                            json!({"games": a.games, "avg_place": a.avg_place()}),
                        )
                    })
                    .collect::<serde_json::Map<_, _>>()
                    .into()
            })
            .unwrap_or(Value::Null);
        sqlx::query(
            "INSERT INTO augment_stats (snapshot_id, augment, games, avg_place, top4_rate, win_rate, pick_rate, by_stage)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(snapshot_id)
        .bind(aug)
        .bind(a.games as i32)
        .bind(a.avg_place())
        .bind(a.top4_rate())
        .bind(a.win_rate())
        .bind(a.games as f32 / boards as f32)
        .bind(by_stage)
        .execute(&mut *tx)
        .await?;
    }

    for ((tr, num_units), a) in traits.iter().filter(|(_, a)| a.games >= th.entity) {
        sqlx::query(
            "INSERT INTO trait_stats (snapshot_id, trait, num_units, games, avg_place, top4_rate)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(snapshot_id)
        .bind(tr)
        .bind(*num_units as i16)
        .bind(a.games as i32)
        .bind(a.avg_place())
        .bind(a.top4_rate())
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE stat_snapshots SET is_current = false
         WHERE set_id=$1 AND patch=$2 AND bracket='master_plus' AND region='global' AND id <> $3",
    )
    .bind(set_id)
    .bind(patch)
    .bind(snapshot_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE stat_snapshots SET is_current = true WHERE id = $1")
        .bind(snapshot_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    tracing::info!(snapshot_id, patch, boards, "aggregate complete");
    Ok(())
}

async fn name_map(pool: &PgPool, table: &str, set_id: i32) -> Result<HashMap<String, String>> {
    let rows = sqlx::query(&format!(
        "SELECT api_name, name FROM {table} WHERE set_id = $1"
    ))
    .bind(set_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<String, _>("api_name"), r.get::<String, _>("name")))
        .collect())
}

/// 案② carry+trait heuristic. comp_id is deterministic; `features` keeps the raw
/// signal so a future clustering labeler can replace this without schema changes.
pub fn comp_label(
    units: &[Value],
    traits: &[Value],
    unit_names: &HashMap<String, String>,
    trait_names: &HashMap<String, String>,
) -> (String, String, Value) {
    // carry = most completed items, tiebreak by rarity (cost) then star
    let carry = units
        .iter()
        .max_by_key(|u| {
            (
                u["itemNames"].as_array().map(|a| a.len()).unwrap_or(0),
                u["rarity"].as_i64().unwrap_or(0),
                u["tier"].as_i64().unwrap_or(0),
            )
        })
        .and_then(|u| u["character_id"].as_str())
        .unwrap_or("unknown");

    // main trait = highest style (bronze<silver<gold<prismatic), tiebreak num_units
    let main_trait = traits
        .iter()
        .filter(|t| t["style"].as_i64().unwrap_or(0) > 0)
        .max_by_key(|t| {
            (
                t["style"].as_i64().unwrap_or(0),
                t["num_units"].as_i64().unwrap_or(0),
            )
        })
        .map(|t| {
            (
                t["name"].as_str().unwrap_or("unknown").to_string(),
                t["num_units"].as_i64().unwrap_or(0),
            )
        })
        .unwrap_or(("unknown".into(), 0));

    let comp_id = format!("{}:{}|{}", main_trait.0, main_trait.1, carry);
    let carry_disp = unit_names
        .get(carry)
        .cloned()
        .unwrap_or_else(|| carry.to_string());
    let trait_disp = trait_names
        .get(&main_trait.0)
        .cloned()
        .unwrap_or_else(|| main_trait.0.clone());
    let name = format!("{carry_disp} {trait_disp}");
    let features = json!({
        "carry": carry,
        "trait": main_trait.0,
        "num_units": main_trait.1,
    });
    (comp_id, name, features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn comp_label_picks_itemed_high_cost_carry_and_strongest_trait() {
        let units = vec![
            json!({"character_id": "TFT17_Briar", "tier": 2, "rarity": 4,
                   "itemNames": ["TFT_Item_InfinityEdge", "TFT_Item_LastWhisper", "TFT_Item_Bloodthirster"]}),
            json!({"character_id": "TFT17_Sona", "tier": 2, "rarity": 1, "itemNames": []}),
        ];
        let traits = vec![
            json!({"name": "TFT17_Anima", "num_units": 4, "style": 2, "tier_current": 2}),
            json!({"name": "TFT17_Rogue", "num_units": 2, "style": 1, "tier_current": 1}),
        ];
        let mut unit_names = HashMap::new();
        unit_names.insert("TFT17_Briar".to_string(), "Briar".to_string());
        let mut trait_names = HashMap::new();
        trait_names.insert("TFT17_Anima".to_string(), "Anima Squad".to_string());

        let (id, name, features) = comp_label(&units, &traits, &unit_names, &trait_names);
        assert_eq!(id, "TFT17_Anima:4|TFT17_Briar");
        assert_eq!(name, "Briar Anima Squad");
        assert_eq!(features["carry"], "TFT17_Briar");
    }

    #[test]
    fn comp_label_inactive_traits_ignored() {
        let units =
            vec![json!({"character_id": "TFT17_Ahri", "tier": 1, "rarity": 3, "itemNames": []})];
        let traits = vec![json!({"name": "TFT17_Sorc", "num_units": 1, "style": 0})];
        let (id, _, _) = comp_label(&units, &traits, &HashMap::new(), &HashMap::new());
        assert_eq!(id, "unknown:0|TFT17_Ahri");
    }
}
