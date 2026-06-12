# VPS deploy (Phase 6)

Single VPS: PostgreSQL 16 + the `voicetft` binary + systemd timer. Stats are
published as static JSON to the `voicetft-data` repo (served by jsDelivr), so the
VPS takes zero read traffic from app users.

## Install
```sh
# postgres
apt install postgresql-16
sudo -u postgres createuser voicetft && sudo -u postgres createdb -O voicetft voicetft

# app
useradd -r -m voicetft
git clone <voicetft-backend repo> /opt/voicetft-backend
cd /opt/voicetft-backend && cargo build --release
install -m 755 deploy/run-pipeline.sh bin/run-pipeline.sh

# config — never commit this file
mkdir -p /etc/voicetft
cat > /etc/voicetft/env <<'EOF'
DATABASE_URL=postgresql://voicetft@localhost/voicetft
# RIOT_API_KEY=...            # add when granted; crawl auto-enables
# DATA_REPO_DIR=/opt/voicetft-data   # checkout of the public stats repo (jsDelivr origin)
EOF
chmod 600 /etc/voicetft/env

# schema + first static load (works pre-key)
sudo -u voicetft env $(cat /etc/voicetft/env | xargs) ./target/release/voicetft migrate
sudo -u voicetft env $(cat /etc/voicetft/env | xargs) ./target/release/voicetft static-refresh

# schedule
cp deploy/voicetft-pipeline.{service,timer} /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now voicetft-pipeline.timer
```

## Backups
Summaries and raw matches are re-derivable from each other only partially — back up
nightly:
```sh
# /etc/cron.d/voicetft-backup (or another systemd timer)
0 4 * * * postgres pg_dump -Fc voicetft > /var/backups/voicetft-$(date +\%a).dump
```
Seven rotating daily dumps; raw `matches` dominates size. If dumps grow too large,
exclude raw partitions (`pg_dump --exclude-table-data 'matches_p*'`) and accept that
only summaries are restorable.

## Retention
Keep current + previous patch hot; drop older raw partitions (summaries are
permanent in `stat_snapshots` + stat tables):
```sql
-- after verifying the patch is fully aggregated and exported:
DROP TABLE matches_p16_10;   -- instant, no vacuum debt
```
Optionally `pg_dump --table=matches_p16_10 | gzip` to object storage first.

## Monitoring (minimal)
- `systemctl status voicetft-pipeline.timer` / `journalctl -u voicetft-pipeline`
- Alert if `stat_snapshots.published_at` for the active patch is older than 24h:
  the desktop app keeps working off the last CDN snapshot either way.
