#!/usr/bin/env bash
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 077
# Activation can be invoked from a root-only directory. Use a working directory
# that the postgres account can traverse before find restores its initial cwd.
cd /var/backups/shapontravels
# Separate lock from deployment; prevents scheduled and pre-release dumps racing.
exec 9>/var/backups/shapontravels/.backup.lock
flock -w 300 9
backup_path="/var/backups/shapontravels/db-$(date -u +%Y%m%dT%H%M%SZ).dump"
trap 'rm -f "${backup_path}.tmp"' EXIT
/usr/lib/postgresql/18/bin/pg_dump -p 5432 -Fc shapontravels > "${backup_path}.tmp"
/usr/lib/postgresql/18/bin/pg_restore --list "${backup_path}.tmp" >/dev/null
mv "${backup_path}.tmp" "$backup_path"
# Retention applies only after a successful new dump.
find /var/backups/shapontravels -maxdepth 1 -name 'db-*.dump' -type f -mtime +6 -delete
