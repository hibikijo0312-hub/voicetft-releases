#!/usr/bin/env bash
#
# Lift the backend/ subtree out of voicetft-releases into the dedicated
# voicetft-backend repo, preserving the commit history of just that folder.
#
# Run locally (on a machine with both repos reachable), from a checkout of
# voicetft-releases on the claude/database-architecture-setup-1hv9q0 branch:
#
#   ./backend/deploy/migrate-to-backend-repo.sh \
#       git@github.com:hibikijo0312-hub/voicetft-backend.git
#
set -euo pipefail
DEST="${1:?usage: migrate-to-backend-repo.sh <voicetft-backend git url>}"
BRANCH="claude/database-architecture-setup-1hv9q0"

git rev-parse --is-inside-work-tree >/dev/null
git checkout "$BRANCH"

# history-preserving split of just backend/
git subtree split --prefix=backend -b _backend_export

TMP="$(mktemp -d)"
git clone "$DEST" "$TMP"
cd "$TMP"
# import the split history onto main (works whether or not the repo was auto-init'd)
git fetch "$OLDPWD" _backend_export
if git rev-parse --verify origin/main >/dev/null 2>&1; then
  git checkout main
  git merge --allow-unrelated-histories -m "Import backend from voicetft-releases" FETCH_HEAD
else
  git checkout -b main FETCH_HEAD
fi
git push -u origin main
cd "$OLDPWD"
git branch -D _backend_export
echo "Done. voicetft-backend now has the backend/ tree at its root, with history."
