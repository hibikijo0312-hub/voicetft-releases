#!/usr/bin/env bash
#
# Idempotent VPS provisioning for the voicetft backend.
# Target: Debian 12 / Ubuntu 22.04+ with root or passwordless sudo.
#
# This is the automation for "set up the VPS". It is safe to re-run: every step
# checks for prior state. It does NOT need a Riot key — it provisions Postgres,
# builds the binary, loads static data, and installs the timer. The crawl stays
# dormant until RIOT_API_KEY is added to /etc/voicetft/env.
#
# Because stats are published to a static CDN (jsDelivr over voicetft-data), the
# VPS needs NO inbound ports, NO domain, and NO TLS.
#
# Usage:
#   sudo BACKEND_GIT=git@github.com:hibikijo0312-hub/voicetft-backend.git ./provision.sh
#
# Optional env:
#   DATA_REPO_GIT=git@github.com:hibikijo0312-hub/voicetft-data.git   # enable publish
#   PLATFORMS=kr,jp1,euw1,...                                         # crawl scope
#   RIOT_API_KEY=RGAPI-...                                            # enable crawl now
set -euo pipefail

APP_USER=voicetft
APP_DIR=/opt/voicetft-backend
DATA_DIR=/opt/voicetft-data
ENV_FILE=/etc/voicetft/env
: "${BACKEND_GIT:?set BACKEND_GIT to the voicetft-backend clone URL}"

say() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }

say "packages"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq postgresql postgresql-contrib git curl build-essential pkg-config rsync ca-certificates

say "rust toolchain (system-wide)"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal
  ln -sf "$HOME/.cargo/bin/cargo" /usr/local/bin/cargo
  ln -sf "$HOME/.cargo/bin/rustc" /usr/local/bin/rustc
fi

say "app user"
id "$APP_USER" >/dev/null 2>&1 || useradd -r -m -s /bin/bash "$APP_USER"

say "postgres role + db"
sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='$APP_USER'" | grep -q 1 \
  || sudo -u postgres createuser "$APP_USER"
sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='voicetft'" | grep -q 1 \
  || sudo -u postgres createdb -O "$APP_USER" voicetft

say "source checkout"
if [ -d "$APP_DIR/.git" ]; then
  git -C "$APP_DIR" pull --ff-only
else
  git clone "$BACKEND_GIT" "$APP_DIR"
fi
chown -R "$APP_USER:$APP_USER" "$APP_DIR"

say "build (release)"
sudo -u "$APP_USER" bash -lc "cd $APP_DIR && cargo build --release"
install -d "$APP_DIR/bin"
install -m 755 "$APP_DIR/deploy/run-pipeline.sh" "$APP_DIR/bin/run-pipeline.sh"

say "env file"
install -d -m 750 "$(dirname "$ENV_FILE")"
if [ ! -f "$ENV_FILE" ]; then
  {
    echo "DATABASE_URL=postgresql://$APP_USER@localhost/voicetft"
    echo "VOICETFT_BIN=$APP_DIR/target/release/voicetft"
    echo "EXPORT_DIR=$APP_DIR/export"
    [ -n "${PLATFORMS:-}" ]    && echo "PLATFORMS=$PLATFORMS"
    [ -n "${RIOT_API_KEY:-}" ] && echo "RIOT_API_KEY=$RIOT_API_KEY"
    [ -n "${DATA_REPO_GIT:-}" ] && echo "DATA_REPO_DIR=$DATA_DIR"
  } > "$ENV_FILE"
  chmod 640 "$ENV_FILE"; chown root:"$APP_USER" "$ENV_FILE"
fi

if [ -n "${DATA_REPO_GIT:-}" ]; then
  say "data repo checkout (publish target)"
  if [ ! -d "$DATA_DIR/.git" ]; then
    sudo -u "$APP_USER" git clone "$DATA_REPO_GIT" "$DATA_DIR"
  fi
fi

say "schema + static load (works without a Riot key)"
sudo -u "$APP_USER" bash -lc "set -a; . $ENV_FILE; set +a; cd $APP_DIR; \
  ./target/release/voicetft migrate && ./target/release/voicetft static-refresh"

say "systemd timer"
install -m 644 "$APP_DIR/deploy/voicetft-pipeline.service" /etc/systemd/system/
install -m 644 "$APP_DIR/deploy/voicetft-pipeline.timer"   /etc/systemd/system/
# point unit at the real build dir if it differs from the default
sed -i "s#/opt/voicetft-backend#$APP_DIR#g" /etc/systemd/system/voicetft-pipeline.service
systemctl daemon-reload
systemctl enable --now voicetft-pipeline.timer

say "nightly backup"
cat > /etc/cron.d/voicetft-backup <<EOF
0 4 * * * postgres pg_dump -Fc voicetft > /var/backups/voicetft-\$(date +\%a).dump
EOF

say "done"
echo "Static data is loaded and the timer is armed."
if [ -z "${RIOT_API_KEY:-}" ]; then
  echo "Crawl is dormant. Add RIOT_API_KEY to $ENV_FILE and the next timer run goes live:"
  echo "  systemctl start voicetft-pipeline.service   # to run immediately"
fi
