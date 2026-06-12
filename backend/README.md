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
  docker-compose.yml  # local Postgres 16, auto-applies migrations on first start
```

## Quick start (local)
```sh
cd backend
docker compose up -d          # Postgres 16 on :5432, schema applied from migrations/
psql postgresql://voicetft:voicetft@localhost:5432/voicetft -c '\dt'
```

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
1. ✅ schema + migrations + docker-compose (this folder)
2. CDragon static ingest (no Riot key)
3. Riot ingest behind a trait + fixture impl (no key for the fixture path)
4. aggregation rollups + tests (no key)
5. JSON snapshot export + manifest + publish (no key)
6. VPS deploy: systemd timers, backups, retention (no key)

Only phase 3's *live* implementation waits on the Riot API key.

## Moving to voicetft-backend
Once the private `voicetft-backend` repo exists:
```sh
# from the voicetft-releases checkout, on this branch:
git subtree split --prefix=backend -b backend-export   # or just copy the folder
# then push the contents to the root of voicetft-backend
```
The migrations are plain SQL and drop straight into `sqlx migrate` (`sqlx migrate run`).
