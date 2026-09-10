#!/usr/bin/env bash
# Root-owned, fixed read-only observation surface for the deploy account.
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
[[ $# == 0 ]] || { echo 'No arguments accepted' >&2; exit 2; }
[[ $EUID == 0 ]] || { echo 'Run through the installed sudo rule' >&2; exit 1; }
export SYSTEMD_PAGER=cat PAGER=cat
systemctl show shapontravels --property=ActiveState --property=MainPID --property=MemoryCurrent --property=MemoryPeak
journalctl --system -u shapontravels.service --since '30 minutes ago' --grep='Search cleanup' --no-pager -o cat || true
runuser -u postgres -- psql -X -p 5432 -d shapontravels -v ON_ERROR_STOP=1 <<'SQL'
BEGIN READ ONLY;
SET LOCAL statement_timeout = '3s';
SELECT now() AS observed_at,
 (SELECT count(*) FROM flight_offers o JOIN flight_searches s ON s.id=o.search_id WHERE o.created_at<=now()-INTERVAL '15 minutes' AND o.expires_at<=now() AND s.expires_at<=now() AND NOT EXISTS(SELECT 1 FROM flight_reprices r WHERE r.offer_id=o.id) AND NOT EXISTS(SELECT 1 FROM flight_bookings b WHERE b.offer_id=o.id)) AS eligible_unused_offers,
 (SELECT count(*) FROM expired_flight_offers) AS expired_markers,
 (SELECT count(*) FROM expired_flight_offers WHERE purged_at>now()-INTERVAL '30 minutes') AS removed_last_30_minutes,
 (SELECT max(purged_at) FROM expired_flight_offers) AS last_payload_cleanup;
SELECT (SELECT count(*) FROM flight_reprices) AS reprices,
 (SELECT count(*) FROM flight_bookings) AS bookings,
 (SELECT count(*) FROM flight_offers o WHERE EXISTS(SELECT 1 FROM flight_reprices r WHERE r.offer_id=o.id) OR EXISTS(SELECT 1 FROM flight_bookings b WHERE b.offer_id=o.id)) AS protected_offers;
SELECT relname,n_live_tup,n_dead_tup,last_autovacuum FROM pg_stat_user_tables WHERE relname IN ('flight_searches','flight_offers','expired_flight_offers');
COMMIT;
SQL
