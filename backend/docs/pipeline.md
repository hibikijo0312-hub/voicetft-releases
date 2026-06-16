# voicetft backend — pipeline spec

Three stages run on a schedule (systemd timers): **static refresh**, **crawl/ingest**,
**aggregate + export**. Only crawl/ingest needs a Riot API key.

## 1. Static refresh (no key)
Trigger: on patch change — detect by polling the CDragon/DDragon version string and
comparing to `patches.cdragon_version`.

1. Fetch the set's `units`, `traits`, `items`, `augments` from Community Dragon.
2. Upsert into the set-versioned static tables (keyed by `set_id`, `api_name`).
3. Record the new `patches` row (`patch`, `set_id`, `cdragon_version`, `started_at`).

This stage alone makes the reference tables fully real **today**, with no Riot key.

## 2. Crawl / ingest (Riot key required — stub until then)
Two routing systems — keep them straight:
- **platform** (summoner / league-v1): `na1, br1, la1, la2, oc1, kr, jp1, euw1,
  eun1, tr1, ru, sg2, tw2, vn2`
- **regional route** (match-v1): `americas, asia, europe, sea`

Loop per platform:
1. **Seed.** `tft-league-v1` challenger + grandmaster + master → the Master+ puuid
   set. Upsert into `ranked_entries` (tier + LP + `snapshot_at`). This set is both the
   crawl seed and the 案Z aggregation filter.
2. **Discover.** For each puuid, map platform → regional route, then
   `matches/by-puuid/{puuid}/ids?startTime=<crawl_state.last_match_start_time>` to get
   only new match ids. Advance the cursor.
3. **Dedup.** A match shared by N crawled players appears N times → the `matches`
   primary key drops duplicates on insert.
4. **Fetch + store.** `matches/{id}` → insert raw payload into `matches` (creating the
   patch partition on first sight of a new patch).

### Rate limiting
- Limits apply **per regional route** → one token-bucket limiter per route.
- On HTTP 429, honor `Retry-After`; exponential backoff otherwise.
- Dev key is tiny (20 req/s, 100 req/2min); build the limiter/queue/scheduler now and
  only swap the configured limits when the production key arrives.

### Stub strategy (build the rest now)
Put the Riot client behind a trait:
```rust
trait RiotSource {
    async fn master_plus(&self, platform: &str) -> Vec<RankedEntry>;
    async fn match_ids(&self, route: &str, puuid: &str, start: Option<i64>) -> Vec<String>;
    async fn match_detail(&self, route: &str, match_id: &str) -> MatchDto;
}
```
A `FixtureSource` backed by sample match JSON lets stages 3–5 be developed and tested
end-to-end. Swap in `HttpRiotSource` once the key lands.

## 3. Aggregate + export (no key)
Recompute per patch (案B — no incremental state):
1. Build the **Master+ puuid set** from the league snapshot being aggregated against.
2. Open a new `stat_snapshots` row (`set`, `patch`, `bracket='master_plus'`,
   `region='global'`).
3. Stream `matches` for the patch. For each match, count only participants whose puuid
   is in the Master+ set (案Z). Accumulate:
   - **comps** — heuristic (carry + core traits) → `comp_id`/`name`/`features`.
   - **units** — overall + `by_star`; **unit×item** cross for "best items on unit".
   - **items**, **augments** (`by_stage`), **traits** (per breakpoint `num_units`).
4. Apply significance threshold + Wilson/shrinkage; write the stat tables referencing
   the snapshot id.
5. Flip `is_current` to the new snapshot atomically.

### Export
6. Render the snapshot to compact versioned JSON (per entity / per page the app needs).
7. Write `manifest.json` (snapshot id + per-file sha256) and publish files to the
   `voicetft-data` repo (jsDelivr) or R2. The desktop app diffs the manifest and
   auto-applies changed files — no user action, no VPS read traffic.

## Retention
Master+ all-region ≈ tens of thousands of players × ~20 recent matches, deduped ≈
a few hundred thousand unique matches per patch. Raw TFT JSON ≈ 30–60 KB each →
~tens of GB per patch. Keep **current + previous patch** hot; gzip older raw to object
storage or drop it (summaries are permanent). Dropping a patch = drop its partition.
