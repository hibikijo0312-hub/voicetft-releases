# Handoff: finishing the repo migration (do this in a scoped session)

This `backend/` tree is staged inside `voicetft-releases` because the session that
built it was scoped to that repo only — it could not push to `voicetft-backend` or
`voicetft-data` (both the git proxy and GitHub MCP hard-deny out-of-scope repos).

The two target repos now exist (created manually):
- `hibikijo0312-hub/voicetft-backend` (private) — code goes here
- `hibikijo0312-hub/voicetft-data` (public) — published stats go here

## To finish automatically
Start a **new Claude Code (web) session whose scope includes all three repos**:
`voicetft-releases`, `voicetft-backend`, `voicetft-data`. Then ask the agent to:

1. **Move the backend into voicetft-backend.**
   - Locally: run `backend/deploy/migrate-to-backend-repo.sh <voicetft-backend url>`
     (history-preserving subtree split).
   - Or, purely via tools: copy every file under `backend/` to the **root** of
     `voicetft-backend` and push with `mcp__github__push_files`.
2. **Initialize voicetft-data.**
   - Copy `backend/deploy/voicetft-data-template/` (README + placeholder
     `manifest.json` + `.gitignore`) to the root of `voicetft-data` and push.
   - This makes the jsDelivr URL live immediately with an empty (snapshot_id 0)
     manifest, which the app treats as "nothing published yet".

## Then (no Claude needed)
3. Provision the VPS: `backend/deploy/provision.sh` (see `docs/deploy.md`). Works
   without a Riot key — loads static data, arms the timer, leaves crawl dormant.
4. When the Riot key arrives, add `RIOT_API_KEY=...` to `/etc/voicetft/env` and run
   `systemctl start voicetft-pipeline.service`. Crawl → aggregate → export → publish
   runs end to end.
5. App side: lift `docs/app-sync-client.md`'s reference module into the Tauri app.

## State at handoff (all verified)
- Schema (4 migrations) applies clean on Postgres 16; 18 tables.
- `static-refresh` ingests live CDragon (Set 17 / patch 16.12).
- `crawl --source fixture` → `aggregate` → `export` runs end to end on fixtures.
- `HttpRiotSource` is written but its league-entry field names should be re-checked
  against the live API once the key is in hand (noted inline in `src/riot.rs`).
