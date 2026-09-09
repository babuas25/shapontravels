#!/usr/bin/env bash
# Install as a root-owned helper. Trusted deployers can execute code and migrations.
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
[[ $# == 1 && $1 =~ ^[0-9a-f]{40}$ ]] || { echo 'Expected one full commit SHA' >&2; exit 2; }
[[ $EUID == 0 ]] || { echo 'Must run through sudo' >&2; exit 2; }
sha=$1
incoming="/home/deploy/incoming/$sha/shapontravels-api"
release="/opt/shapontravels/releases/$sha"
exec 9>/run/lock/shapontravels-deploy.lock
flock -n 9 || { echo 'Another deployment is active' >&2; exit 1; }
[[ -f $incoming && ! -L $incoming ]] || { echo 'Release binary missing' >&2; exit 1; }
[[ -s /etc/shapontravels/.env && -s /etc/shapontravels/migration-database-url ]] || {
  echo 'Complete server runtime and migration configuration first' >&2; exit 1;
}
[[ -x /usr/local/sbin/shapontravels-backup ]] || { echo 'Install backup helper first' >&2; exit 1; }
systemctl cat shapontravels.service >/dev/null
install -d -o root -g shapontravels -m 750 /opt/shapontravels/releases
install -d -o root -g shapontravels -m 750 "$release"
# Read caller-controlled paths with the caller's permissions, not root's.
umask 077
runuser -u deploy -- cat "$incoming" > "$release/shapontravels-api.new"
chown root:shapontravels "$release/shapontravels-api.new"
chmod 750 "$release/shapontravels-api.new"
mv -f "$release/shapontravels-api.new" "$release/shapontravels-api"
runuser -u postgres -- /usr/local/sbin/shapontravels-backup
stopped=false
on_exit() {
  result=$?
  if [[ $result != 0 && $stopped == true ]]; then
    echo 'Deployment failed after stopping API. Service remains stopped; inspect migrations and logs before recovery.' >&2
    systemctl stop shapontravels.service || true
  fi
  exit "$result"
}
trap on_exit EXIT
systemctl stop shapontravels.service
stopped=true
# URL is root-only on disk; never source configuration or print credentials.
DATABASE_URL=$(cat /etc/shapontravels/migration-database-url)
[[ $DATABASE_URL == postgres://shapon_migrator:*@127.0.0.1:5432/shapontravels ]] || {
  echo 'Invalid migration URL: expected local shapon_migrator database URL' >&2; exit 1;
}
export DATABASE_URL
cd /etc/shapontravels
runuser -u shapontravels -- "$release/shapontravels-api" migrate
unset DATABASE_URL
runuser -u postgres -- psql -p 5432 -d shapontravels -v ON_ERROR_STOP=1 <<'SQL'
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO shapon_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO shapon_app;
REVOKE INSERT, UPDATE, DELETE ON TABLE _sqlx_migrations FROM shapon_app;
REVOKE UPDATE, DELETE ON TABLE audit_events, markup_rule_versions FROM shapon_app;
SQL
ln -sfn "$release/shapontravels-api" /opt/shapontravels/shapontravels-api.next
mv -Tf /opt/shapontravels/shapontravels-api.next /opt/shapontravels/shapontravels-api
systemctl start shapontravels.service
healthy=false
for attempt in {1..30}; do
  if curl --fail --silent --max-time 5 http://127.0.0.1:8080/health/live >/dev/null &&
     curl --fail --silent --max-time 5 http://127.0.0.1:8080/health/ready >/dev/null; then
    healthy=true
    break
  fi
  sleep 2
done
[[ $healthy == true ]] || { echo 'Release health check failed' >&2; exit 1; }
stopped=false
printf '%s\n' "$sha" > /opt/shapontravels/deployed-sha
runuser -u deploy -- rm -rf -- "/home/deploy/incoming/$sha"
echo "Deployed $sha; live and ready checks passed."
