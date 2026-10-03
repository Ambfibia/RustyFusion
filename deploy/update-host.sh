#!/usr/bin/env bash
# Run as user1. Local config.toml, database.db and ofapi secrets are never replaced.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$repo_root" != /opt/rustyfusion ]]; then
    echo 'This deployment targets /opt/rustyfusion and /opt/ofapi.' >&2
    exit 1
fi
exec 9>/opt/rustyfusion/.git/update-host.lock
flock -n 9 || { echo 'An update is already running.' >&2; exit 1; }
cd "$repo_root"
git diff --quiet && git diff --cached --quiet || { echo 'Commit or restore tracked changes first.' >&2; exit 1; }
git pull --ff-only
git submodule update --init --recursive
cargo build --locked --release --bin hybrid -j 2
# API configuration and secret are ignored by its repository; rebuild only on source change.
api_before="$(git -C /opt/ofapi rev-parse HEAD)"
git -C /opt/ofapi diff --quiet && git -C /opt/ofapi diff --cached --quiet || { echo 'ofapi has tracked changes.' >&2; exit 1; }
git -C /opt/ofapi pull --ff-only
api_after="$(git -C /opt/ofapi rev-parse HEAD)"
if [[ "$api_before" != "$api_after" ]]; then
    (cd /opt/ofapi && cargo +stable build --locked --release -j 2)
fi
# Stop all DB writers, then snapshot SQLite with the database backup API.
sudo -n systemctl stop ofapi rustyfusion
trap 'sudo -n systemctl start rustyfusion ofapi' EXIT
mkdir -p /opt/rustyfusion/backups
python3 - <<'PY'
import datetime, pathlib, sqlite3
root = pathlib.Path('/opt/rustyfusion')
stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%d-%H%M%S')
source = sqlite3.connect(root / 'database.db')
destination = sqlite3.connect(root / 'backups' / ('database-' + stamp + '.db'))
source.backup(destination)
destination.close()
source.close()
PY
sudo -n install -m 644 deploy/rustyfusion.service /etc/systemd/system/rustyfusion.service
sudo -n install -m 644 deploy/ofapi.service /etc/systemd/system/ofapi.service
sudo -n install -m 644 deploy/api.slavicfall.ru.nginx.conf /etc/nginx/sites-available/api.slavicfall.ru
sudo -n install -d -o www-data -g www-data /var/www/ofapi-static
sudo -n install -m 644 deploy/ofapi-static/* /var/www/ofapi-static/
sudo -n nginx -t
sudo -n systemctl reload nginx
sudo -n systemctl daemon-reload
sudo -n systemctl start rustyfusion ofapi
trap - EXIT
sleep 6
systemctl is-active --quiet rustyfusion
systemctl is-active --quiet ofapi
curl --fail --silent --show-error https://api.slavicfall.ru/status
printf '\nRustyFusion %s; ofapi %s\n' "$(git rev-parse --short HEAD)" "$(git -C /opt/ofapi rev-parse --short HEAD)"
