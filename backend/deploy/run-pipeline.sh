#!/usr/bin/env bash
# One pipeline pass. Patch/set are taken from the patches table after static-refresh.
# RIOT_API_KEY unset -> crawl is skipped (pre-key operation still refreshes statics).
set -euo pipefail
cd "$(dirname "$0")/.."

VOICETFT=${VOICETFT_BIN:-./target/release/voicetft}
PLATFORMS=${PLATFORMS:-kr,jp1,euw1,eun1,na1,br1,la1,la2,oc1,tr1,ru,sg2,tw2,vn2,me1}

$VOICETFT static-refresh

read -r PATCH SET_ID < <(psql "$DATABASE_URL" -Atc \
  "SELECT patch, set_id FROM patches ORDER BY created_at DESC LIMIT 1" | tr '|' ' ')
echo "pipeline: patch=$PATCH set=$SET_ID"

if [ -n "${RIOT_API_KEY:-}" ]; then
  $VOICETFT crawl --source riot --platforms "$PLATFORMS"
  $VOICETFT aggregate --patch "$PATCH" --set "$SET_ID"
  $VOICETFT export
  if [ -n "${DATA_REPO_DIR:-}" ]; then
    rsync -a --delete export/ "$DATA_REPO_DIR/"
    git -C "$DATA_REPO_DIR" add -A
    git -C "$DATA_REPO_DIR" diff --cached --quiet || {
      git -C "$DATA_REPO_DIR" commit -m "stats: set${SET_ID} patch ${PATCH} $(date -u +%FT%TZ)"
      git -C "$DATA_REPO_DIR" push
    }
  fi
else
  echo "RIOT_API_KEY not set; skipping crawl/aggregate/export"
fi
