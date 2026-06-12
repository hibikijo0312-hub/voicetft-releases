# Desktop app: stats auto-sync client (spec + reference impl)

The consuming half of the pipeline. Lives in the `voicetft` app repo (Rust/Tauri);
written here so it can be lifted in once that repo is in scope. Goal: the app always
has fresh Master+ stats with **no user action** and **no load on the backend VPS** —
all reads hit the jsDelivr CDN.

## Behavior
1. On launch, and then every `POLL_INTERVAL` (e.g. 30 min), fetch `manifest.json`.
2. If `snapshot_id` is unchanged from the cached manifest, stop — nothing new.
3. Otherwise, for each entry in `manifest.files`, compare `sha256` to the local
   cache. Download only files whose hash differs.
4. Verify each download's sha256 against the manifest before accepting it (guards
   against truncated/corrupt CDN responses).
5. Write files to a temp dir, then atomically swap the cache dir and the in-memory
   index. Readers never see a half-applied update.
6. Persist the manifest last, so a crash mid-update re-downloads rather than trusting
   a partial set.

If the network is down, the app keeps serving the last good cache indefinitely — the
stats are advisory, never a hard dependency.

## Cache location
`{app_data_dir}/stats/` with `current/` (active) and `manifest.json`. Use Tauri's
`app_data_dir()`.

## CDN base
```
https://cdn.jsdelivr.net/gh/hibikijo0312-hub/voicetft-data@main
```
Always poll the manifest at `@main` (jsDelivr revalidates it); the per-stat files can
be fetched at `@main` too. If propagation latency matters, the publish step can call
`purge.jsdelivr.net` for `manifest.json` (see voicetft-data README).

## Reference implementation (Rust)
Add deps: `reqwest` (rustls), `serde`, `serde_json`, `sha2`, `hex`, `tokio`.

```rust
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CDN: &str = "https://cdn.jsdelivr.net/gh/hibikijo0312-hub/voicetft-data@main";

#[derive(Deserialize)]
struct FileEntry { sha256: String, #[allow(dead_code)] bytes: u64 }

#[derive(Deserialize)]
struct Manifest {
    snapshot_id: i64,
    set: i64,
    patch: String,
    bracket: String,
    files: BTreeMap<String, FileEntry>,
}

/// Returns Some(new_snapshot_id) if stats were updated, None if already current.
pub async fn sync(stats_dir: &Path) -> Result<Option<i64>> {
    let client = reqwest::Client::builder()
        .user_agent("voicetft-app")
        .build()?;

    let remote: Manifest = client
        .get(format!("{CDN}/manifest.json"))
        .send().await?.error_for_status()?
        .json().await?;
    if remote.snapshot_id == 0 {
        return Ok(None); // placeholder manifest, nothing published yet
    }

    let local_manifest = stats_dir.join("manifest.json");
    if let Ok(bytes) = std::fs::read(&local_manifest) {
        if let Ok(local) = serde_json::from_slice::<Manifest>(&bytes) {
            if local.snapshot_id == remote.snapshot_id {
                return Ok(None);
            }
        }
    }

    let current = stats_dir.join("current");
    let staging = stats_dir.join(".staging");
    if staging.exists() { std::fs::remove_dir_all(&staging)?; }
    std::fs::create_dir_all(&staging)?;

    for (rel, entry) in &remote.files {
        let cached = current.join(rel);
        // reuse a cached file only if its hash already matches
        let bytes = match std::fs::read(&cached) {
            Ok(b) if hex::encode(Sha256::digest(&b)) == entry.sha256 => b,
            _ => {
                let b = client.get(format!("{CDN}/{rel}"))
                    .send().await?.error_for_status()?
                    .bytes().await?.to_vec();
                if hex::encode(Sha256::digest(&b)) != entry.sha256 {
                    bail!("sha256 mismatch for {rel}");
                }
                b
            }
        };
        let dst = staging.join(rel);
        std::fs::create_dir_all(dst.parent().unwrap())?;
        std::fs::write(dst, bytes)?;
    }

    // atomic-ish swap: new dir in place, then persist manifest
    let backup = stats_dir.join(".old");
    if current.exists() { std::fs::rename(&current, &backup).ok(); }
    std::fs::rename(&staging, &current).context("promote staging")?;
    let _ = std::fs::remove_dir_all(&backup);

    let manifest_bytes = client.get(format!("{CDN}/manifest.json"))
        .send().await?.error_for_status()?.bytes().await?;
    std::fs::write(&local_manifest, &manifest_bytes)?;

    Ok(Some(remote.snapshot_id))
}

/// Load a stat file from the active cache, e.g. path_for(17, "16.12", "comps").
pub fn stat_path(stats_dir: &Path, set: i64, patch: &str, bracket: &str, name: &str) -> PathBuf {
    stats_dir
        .join("current")
        .join(format!("set{set}/{patch}/{bracket}/{name}.json"))
}
```

## Tauri wiring (sketch)
```rust
// on setup: spawn a background poller
tauri::async_runtime::spawn(async move {
    let dir = app.path().app_data_dir().unwrap().join("stats");
    loop {
        match voicetft_sync::sync(&dir).await {
            Ok(Some(id)) => { let _ = app.emit("stats-updated", id); }
            Ok(None) => {}
            Err(e) => tracing::warn!("stats sync failed: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
    }
});
```

The frontend listens for `stats-updated` and reloads whatever stat views are open.
Map `comps.json` entries' `features.carry` / `unit` / `item` / `augment` api_names to
display names + icons via the same CDragon data the backend ingests (or ship a trimmed
units/items/traits lookup alongside the stats if the app needs it offline).
```
