# voicetft-backend

TFT stats backend for voicetft — ingestion (CDragon static + Riot match crawl),
aggregation, and versioned JSON snapshot export. PostgreSQL + Rust.

> **Staging note.** This `backend/` tree currently lives on a feature branch of
> `voicetft-releases` because that is the only repo this work session could push to.
> It is self-contained and meant to be lifted, as-is, into a dedicated private
> `voicetft-backend` repo (see "Moving to voicetft-backend").

## Layout
```
backend/
  docs/
    architecture.md   # locked design decisions + system diagram
    pipeline.md       # static refresh / crawl / aggregate+export specs
  migrations/
    0001_static.sql   # sets, patches, units, traits, items, augments (set-versioned)
    0002_players.sql  # summoners, ranked_entries (Master+ set), crawl_state
    0003_matches.sql  # matches (raw jsonb), partitioned BY LIST (patch)
    0004_stats.sql    # stat_snapshots + comp/unit/unit_item/item/augment/trait stats
  src/                # Rust binary `voicetft` (sqlx + tokio + reqwest)
    cdragon.rs        # Phase 2: CDragon static ingest (no Riot key)
    riot.rs           # Phase 3: RiotSource trait, FixtureSource + HttpRiotSource (rate-limited)
    crawl.rs          # league snapshot -> incremental match ids -> dedup -> raw insert
    aggregate.rs      # Phase 4: per-patch rollup, 案Z filter, carry+trait comp heuristic
    export.rs         # Phase 5: versioned JSON + sha256 manifest
  fixtures/           # synthetic Set17 matches (real CDragon api names) for keyless dev
  docker-compose.yml  # local Postgres 16, auto-applies migrations on first start
```

## Quick start (local)
```sh
cd backend
docker compose up -d          # Postgres 16 on :5432, schema applied from migrations/
export DATABASE_URL=postgresql://voicetft:voicetft@localhost:5432/voicetft

cargo run -- migrate          # no-op if compose already applied; canonical path on VPS
cargo run -- static-refresh   # live CDragon -> sets/patches/units/traits/items/augments
cargo run -- crawl --source fixture --platforms kr       # keyless: fixture matches
cargo run -- aggregate --patch 16.12 --set 17 \
  --min-games-comp 5 --min-games-pair 3 --min-games-entity 3   # low thresholds for fixtures
cargo run -- export --out export                          # JSON snapshot + manifest.json
```
When the Riot key arrives: `RIOT_API_KEY=... cargo run -- crawl --source riot --platforms kr,euw1,na1,...`
(production thresholds: defaults `--min-games-comp 200 --min-games-pair 50 --min-games-entity 20`).

## Design summary
- **PostgreSQL 16**, VPS-hosted, systemd-timer scheduled.
- **案B** — raw match JSON is the single source of truth; aggregate → summary tables.
- **案② heuristic comps** — carry+trait labels, upgradeable to clustering later.
- **③ all-region Master+ (案Z)** — count a placement only if the puuid is in the
  Master+ league set.
- **Distribution** — versioned JSON + manifest on a free static CDN (jsDelivr, later
  R2); the desktop app auto-applies diffs with no user action and no VPS read load.

See `docs/architecture.md` for the full rationale.

## Build phases
1. ✅ schema + migrations + docker-compose
2. ✅ CDragon static ingest — verified against live CDragon (Set 17, patch 16.12)
3. ✅ Riot ingest behind `RiotSource` trait; `FixtureSource` done, `HttpRiotSource`
   scaffolded with per-route rate limiting (verify league field names when key arrives)
4. ✅ aggregation rollups (案Z filter + comp heuristic) — E2E-tested on fixtures
5. ✅ JSON snapshot export + sha256 manifest
6. ✅ deploy assets: systemd service/timer + pipeline script + `docs/deploy.md` (VPS provisioning itself is manual)

Only phase 3's *live* path waits on the Riot API key — flip `--source riot`.

## Moving to voicetft-backend
Once the private `voicetft-backend` repo exists:
```sh
# from the voicetft-releases checkout, on this branch:
git subtree split --prefix=backend -b backend-export   # or just copy the folder
# then push the contents to the root of voicetft-backend
```
The migrations are plain SQL and drop straight into `sqlx migrate` (`sqlx migrate run`).
