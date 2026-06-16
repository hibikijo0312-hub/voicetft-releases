# voicetft backend — architecture

TFT stats backend, MetaTFT-style. Ingests data, aggregates it, and publishes
versioned JSON snapshots that the desktop app fetches automatically.

```
[CDragon] --(no key)--> static ingest ┐
                                       ├─> [PostgreSQL] ─> aggregate ─> versioned JSON
[Riot API] --(key req)--> crawl/ingest ┘                                    │
                                                                            ▼
                                                       [jsDelivr / R2 CDN] <── push
                                                                │  manifest diff
                                                                ▼  auto-applied, no user action
                                                       [voicetft desktop (Rust / Tauri)]
```

## Stack
- **PostgreSQL 16**, single instance on a VPS. Scheduled via systemd timers.
- **Rust** backend (matches the desktop app's language): `sqlx` (compile-time
  checked SQL + migrations) + `tokio` + `axum` (only if a live API is ever needed).
- **Static data**: Community Dragon / Data Dragon fetcher — no Riot API key needed.

## Locked design decisions
- **① Raw data model — 案B (lean hybrid).** Raw match-v1 JSON is the single source
  of truth (`matches.data jsonb`). No normalized participant tables. Aggregation
  reads raw → writes summary tables. Lowest storage/write cost; raw is retained so
  anything can be re-derived later. Recompute is per-patch (no incremental
  bookkeeping needed).
- **② Comp definition — heuristic.** A deterministic carry+trait heuristic assigns
  `comp_id` / `name` (e.g. "Jinx Sniper"). `comp_stats.features` stores the raw
  signal so a clustering labeler can replace the heuristic later with no schema change.
- **③ Aggregation axis — all regions, Master+ only.** Single bracket
  (`master_plus`), all regions pooled (`region = 'global'`). Summary keys collapse to
  `(set, patch, entity)`. `bracket`/`region` columns are kept for future breakdowns.
  - **案Z counting rule.** A participant's placement is counted only if its `puuid`
    is in the Master+ set (built from the league snapshot). High-elo lobbies are
    almost entirely Master+, so this keeps the volume of "count the whole lobby"
    with the purity of "count only the seed player."
- **Significance.** Below a per-entity game threshold (e.g. comp 200 / unit×item 50)
  a stat is flagged low-confidence; rates use Wilson interval / shrinkage to tame
  small-sample noise.

## Distribution (cheapest, auto-applied)
The VPS only ingests + aggregates + writes versioned JSON. A small `manifest.json`
(snapshot version + per-file hashes) sits next to the data files on a static CDN.
The desktop app polls the manifest, downloads only changed files, and swaps its
local cache — **no user action, no VPS read load**.
- Start on **jsDelivr × a public `voicetft-data` repo** (free CDN, ~¥0).
- Move to **Cloudflare R2 + CDN** (no egress fees) if volume grows.

## Schema (see `migrations/`)
- `0001_static.sql` — `sets`, `patches`, `units`, `traits`, `items`, `augments`
  (set-versioned; populated from CDragon today, no key).
- `0002_players.sql` — `summoners`, `ranked_entries` (= the Master+ set), `crawl_state`
  (per-puuid incremental cursor).
- `0003_matches.sql` — `matches` (raw jsonb + extracted columns), partitioned
  `BY LIST (patch)` so retention = drop a partition.
- `0004_stats.sql` — `stat_snapshots` + `comp_stats` / `unit_stats` /
  `unit_item_stats` / `item_stats` / `augment_stats` / `trait_stats`.

## Build phases
| Phase | Needs Riot key? | What |
|------|------|------|
| 1 | no | schema + migrations + docker-compose (this folder) |
| 2 | no | CDragon static ingest → fill reference tables (fully working) |
| 3 | no | Riot ingest behind a trait + fixture impl from sample match JSON |
| 4 | no | aggregation rollups (案②/案Z) + tests against fixtures |
| 5 | no | JSON snapshot export + manifest + publish to `voicetft-data` |
| 6 | no | VPS deploy (systemd timers, Postgres backups, retention) |

Only Phase 3's real implementation waits on the Riot key; everything else can be
completed now and the stubbed ingest swapped in when the key arrives.
