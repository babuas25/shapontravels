# Run the existing local frontend and Rust backend

This workspace's frontend uses Rust on **127.0.0.1:18081** and the existing loopback PostgreSQL cluster on **127.0.0.1:55439**. The generic `scripts/dev.sh` uses a different database and port; use the portal launcher for this frontend.

## Terminal 1 — Rust

```sh
cd /Users/ashifbabu/Projects/shapontravels
node scripts/portal-dev.mjs
```

Keep this terminal open. After the 2026-09-17 identity cutover, the launcher detects private `.local/identity-local/config.json` and resumes **`portal_local_20260917_identity_test`** with canonical identity. It starts the existing local PostgreSQL cluster if needed, builds `local_identity` and serves port 18081. It does not migrate, import, bootstrap or reactivate automatically. Ctrl+C stops Rust; PostgreSQL stays available. Despite its test suffix, this selected database contains retained user/business data and must not be treated as disposable.

The canonical launcher uses the private selected configuration, Rust `.env` supplier settings and `.local/pgsql` runtime. It does not print credentials or overwrite either project's environment file. It watches `.env`: a save reloads the Rust backend after in-flight requests finish, discards cached supplier logins, and invalidates previous offers. Run a fresh search after reload. If invalid settings prevent startup, correct the file and save again; the watcher retries. The old `api_portal_local_*` database and bridge-credentials file remain retained for evidence, not as an alternate identity authority. Do not remove canonical configuration to fall back to legacy. See [cutover evidence](evidence/RUST_IDENTITY_LOCAL_CUTOVER_2026-09-17.md).

## Terminal 2 — frontend

```sh
cd /Users/ashifbabu/Projects/shopontravels
npm run dev -- --hostname localhost --port 3000
```

If the frontend is already running on 3000, keep that process; do not start a duplicate. Open **http://localhost:3000** and sign in normally with Clerk. `.env.development.local` selects the matching Rust bridge/release pin and canonical identity/wallet authority. Use the `localhost` hostname; the explicit `127.0.0.1` Next bind caused Clerk/Next rewrite timeouts during verification. After changing frontend environment settings, restart its dev server.

The cutover left both servers running. Their private logs and launcher PIDs are under `.local/identity-local/`. Before manually restarting, identify the current listener with `lsof -nP -iTCP:3000 -iTCP:18081 -sTCP:LISTEN` and stop only that service; do not trust an old PID file without checking the process.

## Check the connection

```sh
curl -i http://127.0.0.1:18081/health/ready
```

Expect HTTP 200. Connection refused means Rust is stopped or listening on a different port. A readiness failure means PostgreSQL or migration checks are failing. Health does not test the supplier.

The canonical local launcher uses **configured supplier reads**: Search, FareRules and RePrice for FirstTrip, TakeOff and Triplover, plus each supplier's configured Hold and PNR support. Each supplier uses its own `*_BASE_URL`, `*_SEARCH_BASE_URL`, `*_EMAIL`, `*_PASSWORD` and `*_CURRENCY` from Rust `.env`. UAT and production endpoints are both supported; credentials and endpoints must belong to the same supplier environment/account. No endpoints or currencies are guessed. The old `LOCAL_API_UAT_HOLDS` flag no longer selects this launcher's supplier mode.

At startup/reload this local launcher enables search for suppliers with credentials in `.env` and disables search for suppliers with both email and password absent. Incomplete credentials or URLs produce an actionable startup error. This replaces stale local UAT-only participation switches; dashboard search switches still apply until the next reload. Supplier authentication/transport errors are isolated per supplier, so successful suppliers can return partial results. Invalid credentials, denied access and unavailable inventory cannot be made into real flight results.

Each supplier uses the same `SupplierAdapter` and capability flags as the main server:

| Supplier | Hold flag | Ticketing flag |
| --- | --- | --- |
| FirstTrip | `FIRSTTRIP_BOOKING_ENABLED` | `FIRSTTRIP_TICKETING_ENABLED` |
| TakeOff | `TAKEOFF_BOOKING_ENABLED` | `TAKEOFF_TICKETING_ENABLED` |
| Triplover | `TRIPLOVER_BOOKING_ENABLED` | `TRIPLOVER_TICKETING_ENABLED` |

Hold requires that supplier's booking flag to be `true`, plus `search_enabled=true` and `booking_enabled=true` in the selected database's `supplier_connections`. Missing/false flags disable the corresponding capability; invalid boolean values prevent startup. Flags are independent per supplier. Hold does not require ticketing. PNR reads still require database servicing permission.

Ticketing flags pass through to the shared adapter for all three suppliers in UAT and production. Database `ticketing_enabled` and `servicing_enabled` controls, client authority, verified held-booking evidence and wallet funds are also required. The configured HTTPS endpoints and matching supplier credentials select the actual supplier environment. Direct issue and cancellation remain disabled by default. Existing bookings are retained.

Start with `node scripts/portal-dev.mjs` to rebuild code and retain automatic credential reload; directly running the Rust binary requires a restart after `.env` edits. After restart/reload, run a fresh search before checkout because prior offers are invalidated.

## Recovery performed — 16 September 2026 (Bangladesh time)

The frontend and PostgreSQL were running, but there was no listener on port 18081. Frontend bridge credentials matched the saved local account. The local database was backed up, migration 0028 applied, and the latest Rust portal backend started. Readiness and bridge authentication returned HTTP 200. The two existing held bookings were preserved.

An authenticated Rust Search for DAC → SIN, 30 September 2026, one adult in Economy returned HTTP 200 with 38 offers in approximately 43 seconds. This was a live Triplover UAT read; no Book, Issue or Cancel was submitted.

The actual signed-in frontend was then tested through its Search button with the same route/date/passengers. It successfully rendered **102 flight options**, airline/stop filters, supplier labels and fares from BDT 35,536.00. The frontend expands supplier offers into selectable schedule options, so its displayed option count is distinct from the raw offer count. No frontend restart or credential change was necessary.

## Supplier Hold on the main server

The main binary already reads each supplier's booking/ticketing flags and uses the same adapters as the local connections. Set the relevant booking flag to `true` in the main server environment and enable that supplier's `search_enabled` and `booking_enabled` in that server's database. Deploy the built release through the normal deployment workflow and restart the service after environment changes; the main server does not use the local `.env` watcher. Local database controls are not copied to the main server. Verify a fresh checkout reports `submissionEnabled=true` before submitting any booking. The shared ticketing restrictions described above apply to both servers.
