//! Phase 2: static master-data ingest from Community Dragon. No Riot key needed.
//!
//! Source files (under CDRAGON_BASE, default raw.communitydragon.org/latest):
//!   content-metadata.json  -> "16.12.7869679+..." -> patch "16.12"
//!   cdragon/tft/en_us.json -> { items: [...], setData: [...], sets: {...} }
//!
//! `setData` contains one entry per set *mutator* (TFTSet17, TFTSet17_PAIRS, ...).
//! The live ranked set is the plain "TFTSet{n}" mutator with the highest n.

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};

pub async fn run(
    pool: &PgPool,
    cfg: &crate::config::Config,
    set_override: Option<i32>,
) -> Result<()> {
    let client = reqwest::Client::new();

    let meta: Value = client
        .get(format!("{}/content-metadata.json", cfg.cdragon_base))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = meta["version"]
        .as_str()
        .context("no version in content-metadata")?;
    let patch = parse_patch(version).context("cannot parse patch from version")?;
    tracing::info!(version, patch, "cdragon version");

    let root: Value = client
        .get(format!("{}/cdragon/tft/en_us.json", cfg.cdragon_base))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let set_data = root["setData"].as_array().context("no setData")?;
    let target = match set_override {
        Some(n) => find_set(set_data, n).context("requested set not found")?,
        None => set_data
            .iter()
            .filter(|s| {
                let n = s["number"].as_i64().unwrap_or(0);
                s["mutator"].as_str() == Some(format!("TFTSet{n}").as_str())
            })
            .max_by_key(|s| s["number"].as_i64().unwrap_or(0))
            .context("no plain TFTSet{n} entry in setData")?,
    };
    let set_id = target["number"].as_i64().context("set number")? as i32;
    let mutator = target["mutator"].as_str().unwrap_or_default();
    tracing::info!(set_id, mutator, "ingesting set");

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO sets (set_id, name, mutator, is_active) VALUES ($1,$2,$3,true)
         ON CONFLICT (set_id) DO UPDATE SET name=$2, mutator=$3, is_active=true",
    )
    .bind(set_id)
    .bind(target["name"].as_str().unwrap_or_default())
    .bind(mutator)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sets SET is_active=false WHERE set_id <> $1")
        .bind(set_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO patches (patch, set_id, cdragon_version, started_at) VALUES ($1,$2,$3,now())
         ON CONFLICT (patch) DO UPDATE SET set_id=$2, cdragon_version=$3",
    )
    .bind(&patch)
    .bind(set_id)
    .bind(version)
    .execute(&mut *tx)
    .await?;

    // traits first: champions reference traits by display name, we store api_names
    let mut trait_name_to_api: HashMap<String, String> = HashMap::new();
    let traits = target["traits"].as_array().context("traits")?;
    for t in traits {
        let api = t["apiName"].as_str().context("trait apiName")?;
        let name = t["name"].as_str().unwrap_or(api);
        trait_name_to_api.insert(name.to_string(), api.to_string());
        let breakpoints: Vec<i32> = t["effects"]
            .as_array()
            .map(|es| {
                es.iter()
                    .filter_map(|e| e["minUnits"].as_i64().map(|v| v as i32))
                    .collect()
            })
            .unwrap_or_default();
        sqlx::query(
            "INSERT INTO traits (set_id, api_name, name, breakpoints, data) VALUES ($1,$2,$3,$4,$5)
             ON CONFLICT (set_id, api_name) DO UPDATE SET name=$3, breakpoints=$4, data=$5",
        )
        .bind(set_id)
        .bind(api)
        .bind(name)
        .bind(&breakpoints)
        .bind(t)
        .execute(&mut *tx)
        .await?;
    }
    tracing::info!(count = traits.len(), "traits upserted");

    let champs = target["champions"].as_array().context("champions")?;
    let mut n_units = 0;
    for c in champs {
        let api = c["apiName"].as_str().context("champ apiName")?;
        // skip non-board entities (anvils, eggs...) that carry no traits and odd costs
        let trait_names = c["traits"].as_array().cloned().unwrap_or_default();
        if trait_names.is_empty() {
            continue;
        }
        let trait_apis: Vec<String> = trait_names
            .iter()
            .filter_map(|v| v.as_str())
            .map(|n| {
                trait_name_to_api
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| n.to_string())
            })
            .collect();
        sqlx::query(
            "INSERT INTO units (set_id, api_name, name, cost, traits, role, data) VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (set_id, api_name) DO UPDATE SET name=$3, cost=$4, traits=$5, role=$6, data=$7",
        )
        .bind(set_id)
        .bind(api)
        .bind(c["name"].as_str().unwrap_or(api))
        .bind(c["cost"].as_i64().unwrap_or(0) as i16)
        .bind(&trait_apis)
        .bind(c["role"].as_str())
        .bind(c)
        .execute(&mut *tx)
        .await?;
        n_units += 1;
    }
    tracing::info!(count = n_units, "units upserted");

    // global item index; setData.items / setData.augments are apiName string lists
    let mut item_index: HashMap<&str, &Value> = HashMap::new();
    for it in root["items"].as_array().context("items")? {
        if let Some(api) = it["apiName"].as_str() {
            item_index.insert(api, it);
        }
    }

    let set_item_names: Vec<&str> = target["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let component_set: HashSet<&str> = set_item_names
        .iter()
        .filter_map(|n| item_index.get(n))
        .filter_map(|it| it["composition"].as_array())
        .flat_map(|c| c.iter().filter_map(|v| v.as_str()))
        .collect();

    let mut n_items = 0;
    for api in &set_item_names {
        let Some(it) = item_index.get(api) else {
            continue;
        };
        let comp: Vec<String> = it["composition"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let name = it["name"].as_str().unwrap_or(api);
        sqlx::query(
            "INSERT INTO items (set_id, api_name, name, components, is_component, is_emblem, data)
             VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (set_id, api_name) DO UPDATE
             SET name=$3, components=$4, is_component=$5, is_emblem=$6, data=$7",
        )
        .bind(set_id)
        .bind(api)
        .bind(name)
        .bind(&comp)
        .bind(comp.is_empty() && component_set.contains(*api))
        .bind(api.contains("EmblemItem") || name.ends_with("Emblem"))
        .bind(*it)
        .execute(&mut *tx)
        .await?;
        n_items += 1;
    }
    tracing::info!(count = n_items, "items upserted");

    let mut n_augs = 0;
    if let Some(augs) = target["augments"].as_array() {
        for a in augs {
            let Some(api) = a.as_str() else { continue };
            let it = item_index.get(api);
            let name = it.and_then(|i| i["name"].as_str()).unwrap_or(api);
            sqlx::query(
                "INSERT INTO augments (set_id, api_name, name, data) VALUES ($1,$2,$3,$4)
                 ON CONFLICT (set_id, api_name) DO UPDATE SET name=$3, data=$4",
            )
            .bind(set_id)
            .bind(api)
            .bind(name)
            .bind(it.copied())
            .execute(&mut *tx)
            .await?;
            n_augs += 1;
        }
    }
    tracing::info!(count = n_augs, "augments upserted");

    tx.commit().await?;
    tracing::info!(set_id, patch, "static refresh complete");
    Ok(())
}

fn find_set(set_data: &[Value], n: i32) -> Option<&Value> {
    set_data.iter().find(|s| {
        s["number"].as_i64() == Some(n as i64)
            && s["mutator"].as_str() == Some(format!("TFTSet{n}").as_str())
    })
}

/// "16.12.7869679+branch..." or "Version 16.12.123 (...)" -> "16.12"
pub fn parse_patch(version: &str) -> Option<String> {
    let bytes = version.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                let mid = i;
                i += 1;
                let frac_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > frac_start {
                    return Some(format!(
                        "{}.{}",
                        &version[start..mid],
                        &version[frac_start..i]
                    ));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_patch;

    #[test]
    fn parses_patch_from_versions() {
        assert_eq!(
            parse_patch("16.12.7869679+branch.releases-16-12"),
            Some("16.12".into())
        );
        assert_eq!(
            parse_patch("Version 14.23.638.2387 (Nov 21 2024)"),
            Some("14.23".into())
        );
        assert_eq!(parse_patch("Linux Version 16.12.1"), Some("16.12".into()));
        assert_eq!(parse_patch("nodigits"), None);
    }
}
